//! Map a prepared image graph, relocate all images, then enter guest code.
use crate::{
    Result,
    loader::{
        Config,
        link::{self, Program},
    },
};
use std::path::Path;

pub fn preflight(path: &Path, cfg: &Config) -> Result<Program> {
    link::prepare(path, cfg)
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod native {
    use super::*;
    use crate::{
        loader::{fixups::Kind, link::Value},
        platform::windows::Mapping,
    };
    use std::ffi::{CString, c_char};

    pub struct LoadedProgram {
        pub plan: Program,
        pub mappings: Vec<Mapping>,
        initializers: Vec<Vec<u64>>,
        finalizers: Vec<Vec<u64>>,
        tls: Vec<crate::darwin::tlv::Template>,
    }
    impl LoadedProgram {
        pub fn load(plan: Program, trace: bool) -> Result<Self> {
            let mut mappings = Vec::new();
            for image in &plan.images {
                if trace {
                    eprintln!(
                        "adapter-run [map]: {} ({} segments; {} fixups)",
                        image.path.display(),
                        image.image.segments.len(),
                        image.fixups.len()
                    );
                }
                mappings.push(
                    Mapping::new(&image.image)
                        .map_err(|e| format!("map {}: {e}", image.path.display()))?,
                );
            }
            let mut loaded = Self {
                plan,
                mappings,
                initializers: Vec::new(),
                finalizers: Vec::new(),
                tls: Vec::new(),
            };
            for (index, node) in loaded.plan.images.iter().enumerate() {
                for (slot, fixup) in node.fixups.iter().enumerate() {
                    let dest = loaded.mappings[index]
                        .pointer(node.image.segments[fixup.segment].vmaddr + fixup.offset, 8)?
                        as *mut u64;
                    let value = match &fixup.kind {
                        Kind::Rebase => loaded.mappings[index]
                            .relocated(unsafe { std::ptr::read_unaligned(dest) })?,
                        Kind::RebaseTo { target } => loaded.mappings[index].relocated(*target)?,
                        Kind::Bind { .. } => match node
                            .bindings
                            .get(&slot)
                            .ok_or("binding absent from prepared plan")?
                        {
                            Value::Image { index, address } => {
                                loaded.mappings[*index].relocated(*address)?
                            }
                            Value::Absolute(value) => *value,
                        },
                    };
                    unsafe {
                        std::ptr::write_unaligned(dest, value);
                    }
                }
            }
            for (index, node) in loaded.plan.images.iter().enumerate() {
                if let Some(tls) = crate::loader::tls::plan(&node.image)? {
                    let map = &loaded.mappings[index];
                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            map.pointer(tls.address, tls.size)?,
                            tls.size as usize,
                        )
                    }
                    .to_vec();
                    let mut descriptors = Vec::new();
                    for address in tls.descriptors {
                        let descriptor = map.pointer(address, 24)?;
                        let offset =
                            unsafe { std::ptr::read_unaligned(descriptor.add(16) as *const u64) };
                        if offset >= tls.size {
                            return Err("relocated TLV offset exceeds template".into());
                        }
                        unsafe {
                            std::ptr::write_unaligned(
                                descriptor as *mut u64,
                                crate::darwin::tlv::thunk(),
                            );
                            std::ptr::write_unaligned(
                                descriptor.add(8) as *mut u64,
                                index as u64 + 1,
                            );
                        }
                        descriptors.push((descriptor as usize, offset as usize));
                    }
                    loaded.tls.push(crate::darwin::tlv::Template {
                        bytes: std::sync::Arc::new(bytes),
                        alignment: tls.alignment,
                        descriptors,
                    });
                }
            }
            // Validate every callback across every image before invoking any of
            // them. A bad dependency cannot leave earlier constructors running.
            for index in 0..loaded.plan.images.len() {
                let image = &loaded.plan.images[index].image;
                let map = &loaded.mappings[index];
                let mut init = Vec::new();
                let mut term = Vec::new();
                for section in &image.sections {
                    let kind = section.flags & 0xff;
                    let stride = match kind {
                        9 | 10 => 8,
                        0x16 => 4,
                        _ => continue,
                    };
                    for offset in (0..section.size).step_by(stride) {
                        let data = map.pointer(section.address + offset, stride as u64)?;
                        let pointer = if kind == 0x16 {
                            let offset = unsafe { std::ptr::read_unaligned(data as *const u32) };
                            map.relocated(
                                map.low
                                    .checked_add(offset as u64)
                                    .ok_or("initializer offset overflow")?,
                            )?
                        } else {
                            unsafe { std::ptr::read_unaligned(data as *const u64) }
                        };
                        if !loaded.executable_pointer(pointer) {
                            return Err(format!(
                                "initialize {}: initializer/finalizer points outside executable segments",
                                loaded.plan.images[index].path.display()
                            ));
                        }
                        if kind == 10 {
                            term.push(pointer);
                        } else {
                            init.push(pointer);
                        }
                    }
                }
                loaded.initializers.push(init);
                loaded.finalizers.push(term);
            }
            for (index, mapping) in loaded.mappings.iter().enumerate() {
                mapping.protect(&loaded.plan.images[index].image)?;
            }
            Ok(loaded)
        }
        fn executable_pointer(&self, pointer: u64) -> bool {
            self.plan
                .images
                .iter()
                .zip(&self.mappings)
                .any(|(node, map)| {
                    node.image.segments.iter().any(|s| {
                        if s.initprot & 4 == 0 || s.vmsize == 0 {
                            return false;
                        }
                        map.relocated(s.vmaddr)
                            .is_ok_and(|low| pointer >= low && pointer < low + s.vmsize)
                    })
                })
        }
        pub fn run(&self, args: &[String], trace: bool) -> Result<i32> {
            let executable = &self.plan.images[0].path;
            let mut argv = vec![
                CString::new(executable.to_string_lossy().as_bytes())
                    .map_err(|_| "NUL in executable path")?,
            ];
            for a in args {
                argv.push(CString::new(a.as_bytes()).map_err(|_| "NUL in argument")?);
            }
            let env: Vec<_> = std::env::vars_os()
                .filter_map(|(k, v)| {
                    CString::new(format!("{}={}", k.to_string_lossy(), v.to_string_lossy())).ok()
                })
                .collect();
            let apple = [
                CString::new(format!("executable_path={}", executable.display()))
                    .map_err(|_| "NUL in executable path")?,
            ];
            let pointers = |v: &[CString]| {
                v.iter()
                    .map(|s| s.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect::<Vec<_>>()
            };
            let (av, ev, ap) = (pointers(&argv), pointers(&env), pointers(&apple));
            let ranges = self
                .plan
                .images
                .iter()
                .zip(&self.mappings)
                .flat_map(|(node, map)| {
                    node.image.segments.iter().filter_map(|s| {
                        if s.initprot & 4 == 0 || s.vmsize == 0 {
                            None
                        } else {
                            map.relocated(s.vmaddr).ok().map(|a| (a, a + s.vmsize))
                        }
                    })
                })
                .collect();
            let _callbacks = crate::darwin::callbacks::begin(ranges)?;
            let images = self
                .plan
                .images
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let text = node
                        .image
                        .segments
                        .iter()
                        .find(|s| s.name == "__TEXT")
                        .ok_or("image has no __TEXT segment")?;
                    let header = self.mappings[index].relocated(text.vmaddr)?;
                    Ok((
                        node.path.display().to_string(),
                        header,
                        header as i64 - text.vmaddr as i64,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            crate::darwin::dyld::begin(images);
            let _tls = crate::darwin::tlv::begin(self.tls.clone())?;
            let _thread_state = crate::darwin::thread_state::begin()?;
            let _threads = crate::darwin::threads::begin()?;
            crate::darwin::libc::begin();
            crate::darwin::libc::set_executable_path(&argv[0]);
            unsafe { crate::darwin::libc::set_arguments(argv.len() as i32, av.as_ptr()) };
            type Init = unsafe extern "sysv64" fn(
                i32,
                *const *const c_char,
                *const *const c_char,
                *const *const c_char,
            );
            type Entry = unsafe extern "sysv64" fn(
                i32,
                *const *const c_char,
                *const *const c_char,
                *const *const c_char,
            ) -> i32;
            for &index in &self.plan.init_order {
                for &pointer in &self.initializers[index] {
                    if trace {
                        eprintln!(
                            "adapter-run [initialize]: {} {pointer:#x}",
                            self.plan.images[index].path.display()
                        );
                    }
                    let init: Init = unsafe { std::mem::transmute(pointer as usize) };
                    unsafe {
                        init(argv.len() as i32, av.as_ptr(), ev.as_ptr(), ap.as_ptr());
                    }
                }
            }
            let entry = link::entry(&self.plan.images[0].image)?;
            if trace {
                eprintln!(
                    "adapter-run [execute]: entry={entry:#x} argc={}",
                    argv.len()
                );
            }
            let main: Entry =
                unsafe { std::mem::transmute(self.mappings[0].relocated(entry)? as usize) };
            let result = unsafe { main(argv.len() as i32, av.as_ptr(), ev.as_ptr(), ap.as_ptr()) };
            // Never unmap code/TLS while an owned guest thread can still use it.
            crate::darwin::threads::finish()?;
            crate::darwin::callbacks::finalize(0)?;
            for &index in self.plan.init_order.iter().rev() {
                for &pointer in self.finalizers[index].iter().rev() {
                    if trace {
                        eprintln!(
                            "adapter-run [finalize]: {}",
                            self.plan.images[index].path.display()
                        );
                    }
                    let term: unsafe extern "sysv64" fn() =
                        unsafe { std::mem::transmute(pointer as usize) };
                    unsafe {
                        term();
                    }
                }
            }
            crate::darwin::libc::flush();
            if trace {
                eprintln!(
                    "adapter-run [complete]: guest returned {result}; releasing mapped images: {}",
                    self.mappings.len()
                );
            }
            Ok(result)
        }
    }
    pub fn execute(path: &Path, cfg: &Config, args: &[String], trace: bool) -> Result<i32> {
        let loaded = LoadedProgram::load(preflight(path, cfg)?, trace)?;
        loaded.run(args, trace)
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
pub use native::execute;

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn execute(_path: &Path, _cfg: &Config, _args: &[String], _trace: bool) -> Result<i32> {
    Err("execution requires Windows x86_64".into())
}
