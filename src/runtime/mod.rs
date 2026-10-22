//! Map a prepared image graph, relocate all images, then enter guest code.
use crate::{
    Result,
    loader::{
        Config,
        link::{self, Program},
    },
};
use std::path::Path;

// Values pinned from Xcode 27's dlfcn.h (AppleTVOS hardlink carrier).
pub const RTLD_LAZY: i32 = 0x1;
pub const RTLD_NOW: i32 = 0x2;
pub const RTLD_LOCAL: i32 = 0x4;
pub const RTLD_GLOBAL: i32 = 0x8;
pub const RTLD_NOLOAD: i32 = 0x10;
pub const RTLD_NODELETE: i32 = 0x80;
pub const RTLD_FIRST: i32 = 0x100;
pub const RTLD_NEXT: isize = -1;
pub const RTLD_DEFAULT: isize = -2;
pub const RTLD_SELF: isize = -3;
pub const RTLD_MAIN_ONLY: isize = -5;

pub fn preflight(path: &Path, cfg: &Config) -> Result<Program> {
    link::prepare(path, cfg)
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod native {
    use super::*;
    use crate::{
        loader::{
            fixups::Kind,
            link::{DependencyTarget, Value},
        },
        platform::windows::Mapping,
    };
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::{CString, c_char},
    };

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
    type Term = unsafe extern "sysv64" fn();

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
            super::debug::record(&loaded.plan, &loaded.mappings)?;
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
        /// Snapshot public exports and mapped ranges. `dlsym` uses these
        /// absolute addresses and never borrows the loaded program again.
        pub fn records(&self) -> Result<Vec<super::dynamic::ImageRecord>> {
            let mut out = Vec::new();
            for (index, node) in self.plan.images.iter().enumerate() {
                let text = node
                    .image
                    .segments
                    .iter()
                    .find(|s| s.name == "__TEXT")
                    .ok_or("image has no __TEXT segment")?;
                let header = self.mappings[index].relocated(text.vmaddr)?;
                let mut exports = BTreeMap::new();
                for export in &node.image.exports {
                    if let Ok(Some(value)) = link::resolve_export(&self.plan, index, &export.name) {
                        let address = match value {
                            Value::Image { index, address } => {
                                self.mappings[index].relocated(address)?
                            }
                            Value::Absolute(value) => value,
                        };
                        exports.insert(export.name.clone(), address);
                    }
                }
                let mut executable = Vec::new();
                for segment in &node.image.segments {
                    if segment.name == "__PAGEZERO" || segment.vmsize == 0 {
                        continue;
                    }
                    if segment.initprot & 4 != 0 {
                        let low = self.mappings[index].relocated(segment.vmaddr)?;
                        let high = low
                            .checked_add(segment.vmsize)
                            .ok_or("relocated segment range overflow")?;
                        executable.push((low, high));
                    }
                }
                out.push(super::dynamic::ImageRecord {
                    path: node.path.clone(),
                    header,
                    slide: header as i64 - text.vmaddr as i64,
                    exports,
                    executable,
                });
            }
            Ok(out)
        }
        /// Root-first dependency order used by `dlsym(handle, ...)`.
        pub fn search_order(&self) -> Vec<usize> {
            fn visit(
                plan: &Program,
                index: usize,
                seen: &mut BTreeSet<usize>,
                out: &mut Vec<usize>,
            ) {
                if !seen.insert(index) {
                    return;
                }
                out.push(index);
                for dependency in &plan.images[index].dependencies {
                    if let DependencyTarget::Image(child) = dependency {
                        visit(plan, *child, seen, out);
                    }
                }
            }
            let mut out = Vec::new();
            visit(&self.plan, 0, &mut BTreeSet::new(), &mut out);
            out
        }
        pub fn adapter_names(&self) -> Vec<String> {
            let mut names = BTreeSet::new();
            for node in &self.plan.images {
                for dependency in &node.dependencies {
                    if let DependencyTarget::Adapter(name) = dependency {
                        names.insert(name.clone());
                    }
                }
            }
            names.into_iter().collect()
        }
        pub fn initialize(
            &self,
            argc: i32,
            argv: *const *const c_char,
            env: *const *const c_char,
            apple: *const *const c_char,
            trace: bool,
        ) {
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
                        init(argc, argv, env, apple);
                    }
                }
            }
        }
        pub fn finalize(&self, trace: bool) {
            for &index in self.plan.init_order.iter().rev() {
                for &pointer in self.finalizers[index].iter().rev() {
                    if trace {
                        eprintln!(
                            "adapter-run [finalize]: {}",
                            self.plan.images[index].path.display()
                        );
                    }
                    let term: Term = unsafe { std::mem::transmute(pointer as usize) };
                    unsafe {
                        term();
                    }
                }
            }
        }
        pub fn run(&self, cfg: &Config, args: &[String], trace: bool) -> Result<i32> {
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
            let records = self.records()?;
            crate::darwin::dyld::begin(
                records
                    .iter()
                    .map(|record| {
                        (
                            record.path.display().to_string(),
                            record.header,
                            record.slide,
                        )
                    })
                    .collect(),
            );
            let _tls = crate::darwin::tlv::begin(self.tls.clone())?;
            let _thread_state = crate::darwin::thread_state::begin()?;
            let _threads = crate::darwin::threads::begin()?;
            crate::darwin::dispatch::begin();
            crate::darwin::mach::begin(trace)?;
            crate::darwin::libc::begin();
            crate::darwin::libc::set_executable_path(&argv[0]);
            unsafe { crate::darwin::libc::set_arguments(argv.len() as i32, av.as_ptr()) };
            let _dynamic = super::dynamic::begin(
                cfg.clone(),
                records,
                self.search_order(),
                self.adapter_names(),
                argv.len() as i32,
                av.as_ptr() as usize,
                ev.as_ptr() as usize,
                ap.as_ptr() as usize,
                trace,
            )?;
            let mut objc_images = Vec::new();
            for (node, mapping) in self.plan.images.iter().zip(&self.mappings) {
                let mut regions = Vec::new();
                for segment in &node.image.segments {
                    if segment.name == "__PAGEZERO" || segment.vmsize == 0 {
                        continue;
                    }
                    regions.push(crate::objc::Region {
                        address: mapping.relocated(segment.vmaddr)?,
                        size: segment.vmsize,
                        readable: segment.initprot & 1 != 0,
                        writable: segment.initprot & 2 != 0 && segment.flags & 0x10 == 0,
                        executable: segment.initprot & 4 != 0,
                    });
                }
                let mut sections = Vec::new();
                for section in &node.image.sections {
                    if section.name.starts_with("__objc_") && section.size != 0 {
                        sections.push(crate::objc::MetadataSection {
                            name: section.name.clone(),
                            address: mapping.relocated(section.address)?,
                            size: section.size,
                        });
                    }
                }
                objc_images.push(crate::objc::Image { regions, sections });
            }
            let objc = crate::objc::begin(objc_images)?;
            objc.initialize()?;
            self.initialize(
                argv.len() as i32,
                av.as_ptr(),
                ev.as_ptr(),
                ap.as_ptr(),
                trace,
            );
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
            crate::darwin::dispatch::finish()?;
            crate::darwin::threads::finish()?;
            crate::darwin::callbacks::finalize(0)?;
            super::dynamic::finalize_all()?;
            self.finalize(trace);
            crate::darwin::mach::finish()?;
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
        loaded.run(cfg, args, trace)
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
pub mod debug;
#[cfg(all(windows, target_arch = "x86_64"))]
mod dynamic;
#[cfg(all(windows, target_arch = "x86_64"))]
pub use dynamic::{
    ImageRecord, close as dynamic_close, load as dynamic_load, symbol as dynamic_symbol,
};
#[cfg(all(windows, target_arch = "x86_64"))]
pub use native::execute;

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn execute(_path: &Path, _cfg: &Config, _args: &[String], _trace: bool) -> Result<i32> {
    Err("execution requires Windows x86_64".into())
}
#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn dynamic_load(_path: &str, _mode: i32) -> Result<usize> {
    Err("dynamic loading requires Windows x86_64".into())
}
#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn dynamic_symbol(_handle: isize, _name: &str) -> Result<Option<u64>> {
    Err("dynamic loading requires Windows x86_64".into())
}
#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn dynamic_close(_handle: isize) -> Result<()> {
    Err("dynamic loading requires Windows x86_64".into())
}
