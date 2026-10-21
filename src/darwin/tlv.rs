//! Runtime-owned per-thread template copies. Thunks preserve the Darwin TLV
//! calling convention, which is stricter than an ordinary SysV function call.
use crate::Result;
use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    cell::RefCell,
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
#[derive(Clone)]
pub struct Template {
    pub bytes: Arc<Vec<u8>>,
    pub alignment: usize,
    pub descriptors: Vec<(usize, usize)>,
}
struct Session {
    generation: u64,
    templates: Vec<Template>,
    descriptors: BTreeMap<usize, (usize, usize)>,
}
fn session() -> &'static Mutex<Option<Session>> {
    static S: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}
struct Block {
    pointer: std::ptr::NonNull<u8>,
    layout: Layout,
}
impl Drop for Block {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.pointer.as_ptr(), self.layout);
        }
    }
}
thread_local! {static BLOCKS:RefCell<BTreeMap<(u64,usize),Block>>=const {RefCell::new(BTreeMap::new())};}
thread_local! {static ATEXIT:RefCell<Vec<(usize,usize)>>=const {RefCell::new(Vec::new())};}
pub struct Guard;
pub fn begin(templates: Vec<Template>) -> Result<Guard> {
    if !templates.is_empty() {
        initialize_register_save()?;
    }
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let mut s = session().lock().map_err(|_| "TLS registry poisoned")?;
    if s.is_some() {
        return Err("concurrent guest TLS session unsupported".into());
    }
    let mut descriptors = BTreeMap::new();
    for (index, t) in templates.iter().enumerate() {
        for &(address, offset) in &t.descriptors {
            descriptors.insert(address, (index, offset));
        }
    }
    *s = Some(Session {
        generation: GENERATION.fetch_add(1, Ordering::Relaxed),
        templates,
        descriptors,
    });
    Ok(Guard)
}
impl Drop for Guard {
    fn drop(&mut self) {
        clear_thread();
        if let Ok(mut s) = session().lock() {
            *s = None;
        }
    }
}
pub fn clear_thread() {
    let entries = ATEXIT.with(|v| std::mem::take(&mut *v.borrow_mut()));
    for (function, object) in entries.into_iter().rev() {
        let function: unsafe extern "sysv64" fn(*mut u8) = unsafe { std::mem::transmute(function) };
        unsafe { function(object as *mut u8) };
    }
    BLOCKS.with(|blocks| blocks.borrow_mut().clear());
}

unsafe extern "sysv64" fn tlv_atexit(
    function: Option<unsafe extern "sysv64" fn(*mut u8)>,
    object: *mut u8,
) {
    if let Some(function) = function {
        ATEXIT.with(|v| v.borrow_mut().push((function as usize, object as usize)));
    }
}

fn lookup(descriptor: usize) -> Result<*mut u8> {
    let (generation, index, offset, template) = {
        let s = session().lock().map_err(|_| "TLS registry poisoned")?;
        let s = s.as_ref().ok_or("TLV access outside active guest")?;
        let &(index, offset) = s
            .descriptors
            .get(&descriptor)
            .ok_or("unknown TLV descriptor")?;
        (s.generation, index, offset, s.templates[index].clone())
    };
    BLOCKS.with(|blocks| {
        let mut blocks = blocks.borrow_mut();
        if let std::collections::btree_map::Entry::Vacant(e) = blocks.entry((generation, index)) {
            let layout = Layout::from_size_align(template.bytes.len(), template.alignment)
                .map_err(|_| "invalid TLS allocation layout")?;
            let pointer = std::ptr::NonNull::new(unsafe { alloc_zeroed(layout) })
                .ok_or("TLS allocation failed")?;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    template.bytes.as_ptr(),
                    pointer.as_ptr(),
                    template.bytes.len(),
                );
            }
            e.insert(Block { pointer, layout });
        }
        let pointer = blocks.get(&(generation, index)).unwrap().pointer;
        Ok(unsafe { pointer.as_ptr().add(offset) })
    })
}

#[unsafe(no_mangle)]
unsafe extern "sysv64" fn db_tlv_lookup(descriptor: usize) -> *mut u8 {
    match lookup(descriptor) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("adapter-run [TLS]: {e}");
            std::process::exit(125);
        }
    }
}
#[unsafe(no_mangle)]
static DB_TLV_XSAVE_MASK: AtomicU64 = AtomicU64::new(0);
fn initialize_register_save() -> Result<()> {
    static SETUP: OnceLock<Result<()>> = OnceLock::new();
    SETUP
        .get_or_init(|| {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                use std::arch::x86_64::{__cpuid, __cpuid_count, _xgetbv};
                let cpu = __cpuid(1);
                if cpu.ecx & (1 << 26) != 0 && cpu.ecx & (1 << 27) != 0 {
                    let mask = _xgetbv(0);
                    let size = __cpuid_count(0xd, 0).ebx;
                    if size > 16384 {
                        return Err("host XSAVE state exceeds 16 KiB TLV bridge limit".into());
                    }
                    DB_TLV_XSAVE_MASK.store(mask, Ordering::Release);
                }
            }
            Ok(())
        })
        .clone()
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
    .text
    .globl db_tlv_thunk
db_tlv_thunk:
    push rbp
    mov rbp, rsp
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    push r10
    push r11
    # Probe every page before using the large XSAVE area on Windows stacks.
    mov ecx, 5
2:
    sub rsp, 4096
    mov byte ptr [rsp], 0
    dec ecx
    jnz 2b
    and rsp, -64
    lea rdi, [rsp + 64]
    mov ecx, 2048
    xor eax, eax
    rep stosq
    mov rax, qword ptr [rip + DB_TLV_XSAVE_MASK]
    test rax, rax
    jz 3f
    mov rdx, rax
    shr rdx, 32
    xsave64 [rsp + 64]
    jmp 4f
3:
    fxsave64 [rsp + 64]
4:
    mov rdi, qword ptr [rbp - 8]
    call db_tlv_lookup
    mov qword ptr [rsp + 32], rax
    mov rax, qword ptr [rip + DB_TLV_XSAVE_MASK]
    test rax, rax
    jz 5f
    mov rdx, rax
    shr rdx, 32
    xrstor64 [rsp + 64]
    jmp 6f
5:
    fxrstor64 [rsp + 64]
6:
    mov rax, qword ptr [rsp + 32]
    lea rsp, [rbp - 64]
    pop r11
    pop r10
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    ret
"#
);
#[cfg(target_arch = "x86_64")]
unsafe extern "sysv64" {
    fn db_tlv_thunk();
}
pub fn thunk() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        db_tlv_thunk as *const () as u64
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}
pub fn symbol(name: &str) -> Option<u64> {
    if name == "__tlv_bootstrap" {
        Some(thunk())
    } else if name == "__tlv_atexit" || name == "_tlv_atexit" {
        Some(tlv_atexit as *const () as u64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_copy_alignment_thread_isolation_and_session_reset() {
        let template = Template {
            bytes: Arc::new(vec![7; 128]),
            alignment: 128,
            descriptors: vec![(42, 0), (43, 64)],
        };
        let guard = begin(vec![template.clone()]).unwrap();
        let pointer = lookup(42).unwrap();
        assert_eq!(pointer as usize % 128, 0);
        unsafe {
            *pointer = 31;
            assert_eq!(*lookup(43).unwrap(), 7);
        }
        std::thread::spawn(|| {
            let p = lookup(42).unwrap();
            assert_eq!(unsafe { *p }, 7);
            unsafe {
                *p = 99;
            }
            clear_thread();
        })
        .join()
        .unwrap();
        assert_eq!(unsafe { *lookup(42).unwrap() }, 31);
        drop(guard);
        assert!(lookup(42).is_err());
        let _guard = begin(vec![template]).unwrap();
        assert_eq!(unsafe { *lookup(42).unwrap() }, 7);
    }
}
