//! Narrow Darwin pthread ABI adapters over owned Windows-hosted Rust threads.
//! pthread_t is an opaque pointer-width token here, never a Windows HANDLE.
use crate::Result;
use std::{
    cell::Cell,
    collections::BTreeMap,
    ffi::{CStr, c_char, c_void},
    sync::{Condvar, Mutex, OnceLock},
    thread::JoinHandle,
};
const EINVAL: i32 = 22;
const ESRCH: i32 = 3;
const EDEADLK: i32 = 11;
const EAGAIN: i32 = 35;
const ERANGE: i32 = 34;
/* Apple's 64-byte _opaque_pthread_attr_t carries this signature; the adapter
 * stores the requested stack size at offset 8 and leaves the rest reserved. */
const ATTR_SIG: i64 = 0x1410FAB3;
const PTHREAD_STACK_MIN: usize = 16384;
const MAX_STACK: usize = 512 * 1024 * 1024;
struct State {
    active: bool,
    next: usize,
    spawned: usize,
    running: usize,
    handles: BTreeMap<usize, JoinHandle<usize>>,
}
fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            active: false,
            next: 2,
            spawned: 0,
            running: 0,
            handles: BTreeMap::new(),
        })
    })
}
fn completion() -> &'static Condvar {
    static C: Condvar = Condvar::new();
    &C
}
struct Completion;
impl Drop for Completion {
    fn drop(&mut self) {
        if let Ok(mut s) = state().lock() {
            s.running -= 1;
            completion().notify_all();
        }
    }
}
thread_local! {static ID:Cell<usize>=const {Cell::new(1)};}
/// The calling thread's `pthread_t` token; the main thread is 1.
pub fn current_id() -> usize {
    ID.with(|id| id.get())
}
pub struct Guard;
pub fn begin() -> Result<Guard> {
    let mut s = state().lock().map_err(|_| "thread registry poisoned")?;
    if s.active {
        return Err("concurrent guest thread session unsupported".into());
    }
    s.active = true;
    s.spawned = 0;
    ID.with(|id| id.set(1));
    Ok(Guard)
}
pub fn finish() -> Result<()> {
    loop {
        let handle = {
            let mut s = state().lock().map_err(|_| "thread registry poisoned")?;
            loop {
                if let Some((_, handle)) = s.handles.pop_first() {
                    break Some(handle);
                }
                if s.running == 0 {
                    break None;
                }
                s = completion()
                    .wait(s)
                    .map_err(|_| "thread registry poisoned")?;
            }
        };
        let Some(handle) = handle else {
            return Ok(());
        };
        handle.join().map_err(|_| "guest host-thread panicked")?;
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = finish();
        if let Ok(mut s) = state().lock() {
            s.active = false;
        }
    }
}
#[cfg(windows)]
fn readable(pointer: *const c_void, length: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    let mut info = std::mem::MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
    if unsafe { VirtualQuery(pointer, info.as_mut_ptr(), std::mem::size_of_val(&info)) } == 0 {
        return false;
    }
    let info = unsafe { info.assume_init() };
    if info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return false;
    }
    let base = info.BaseAddress as usize;
    let start = pointer as usize;
    let end = base.saturating_add(info.RegionSize);
    start >= base && start.checked_add(length).is_some_and(|e| e <= end)
}

#[cfg(not(windows))]
fn readable(_pointer: *const c_void, _length: usize) -> bool {
    true
}

unsafe fn attribute_stack_size(attributes: *const c_void) -> Option<usize> {
    if attributes.is_null() {
        return Some(0);
    }
    if !readable(attributes, 64) {
        return None;
    }
    unsafe {
        if std::ptr::read_unaligned(attributes as *const i64) != ATTR_SIG {
            return None;
        }
        let size = std::ptr::read_unaligned((attributes as *const u8).add(8) as *const usize);
        if size == 0 || (PTHREAD_STACK_MIN..=MAX_STACK).contains(&size) {
            Some(size)
        } else {
            None
        }
    }
}

unsafe extern "sysv64" fn attr_init(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(attributes as *mut u8, 0, 64);
        std::ptr::write_unaligned(attributes as *mut i64, ATTR_SIG);
    }
    0
}

unsafe extern "sysv64" fn attr_destroy(attributes: *mut c_void) -> i32 {
    if unsafe { attribute_stack_size(attributes) }.is_none() {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(attributes as *mut u8, 0, 64);
    }
    0
}

unsafe extern "sysv64" fn attr_setstacksize(attributes: *mut c_void, size: usize) -> i32 {
    if unsafe { attribute_stack_size(attributes) }.is_none() {
        return EINVAL;
    }
    if size != 0 && !(PTHREAD_STACK_MIN..=MAX_STACK).contains(&size) {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_unaligned((attributes as *mut u8).add(8) as *mut usize, size);
    }
    0
}

thread_local! {static STACK: Cell<(usize, usize)> = const { Cell::new((0, 0)) };}

fn current_stack() -> (usize, usize) {
    STACK.with(|cell| {
        let cached = cell.get();
        if cached.0 != 0 {
            return cached;
        }
        let (mut low, mut high) = (0usize, 0usize);
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits(&mut low, &mut high);
        }
        cell.set((low, high));
        (low, high)
    })
}

extern "sysv64" fn get_stackaddr(_thread: usize) -> *mut c_void {
    current_stack().1 as *mut c_void
}

extern "sysv64" fn get_stacksize(_thread: usize) -> usize {
    let (low, high) = current_stack();
    high.saturating_sub(low)
}

unsafe extern "sysv64" fn setname(name: *const c_char) -> i32 {
    if name.is_null() {
        return EINVAL;
    }
    let bytes = unsafe { CStr::from_ptr(name) }.to_bytes();
    if bytes.len() > 63 {
        return ERANGE;
    }
    #[cfg(windows)]
    {
        let wide: Vec<u16> = String::from_utf8_lossy(bytes)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            windows_sys::Win32::System::Threading::SetThreadDescription(
                windows_sys::Win32::System::Threading::GetCurrentThread(),
                wide.as_ptr(),
            );
        }
    }
    0
}

unsafe extern "sysv64" fn threadid(thread: usize, output: *mut u64) -> i32 {
    if output.is_null() {
        return EINVAL;
    }
    if thread != 0 && thread != current() {
        return EINVAL;
    }
    #[cfg(windows)]
    unsafe {
        *output = windows_sys::Win32::System::Threading::GetCurrentThreadId() as u64;
    }
    0
}

unsafe extern "sysv64" fn create(
    output: *mut usize,
    attributes: *const c_void,
    start: Option<unsafe extern "sysv64" fn(*mut c_void) -> *mut c_void>,
    argument: *mut c_void,
) -> i32 {
    let Some(start) = start else {
        return EINVAL;
    };
    let Some(stack_size) = (unsafe { attribute_stack_size(attributes) }) else {
        return EINVAL;
    };
    if output.is_null() || !super::callbacks::executable(start as *const () as u64) {
        return EINVAL;
    }
    let Ok(mut s) = state().lock() else {
        return EAGAIN;
    };
    if !s.active || s.running >= 128 || s.spawned >= 4096 {
        return EAGAIN;
    }
    let id = s.next;
    let Some(next) = id.checked_add(1) else {
        return EAGAIN;
    };
    s.next = next;
    let argument = argument as usize;
    s.running += 1;
    let mut builder = std::thread::Builder::new().name(format!("Adapter-{id}"));
    if stack_size != 0 {
        builder = builder.stack_size(stack_size);
    }
    let handle = builder.spawn(move || {
        let _completion = Completion;
        ID.with(|v| v.set(id));
        super::thread_state::reset_thread();
        let result = unsafe { start(argument as *mut c_void) } as usize;
        if let Err(e) = super::thread_state::finish_thread() {
            eprintln!("adapter-run [thread cleanup]: {e}");
            std::process::exit(125);
        }
        super::tlv::clear_thread();
        result
    });
    let Ok(handle) = handle else {
        s.running -= 1;
        return EAGAIN;
    };
    s.handles.insert(id, handle);
    s.spawned += 1;
    unsafe {
        *output = id;
    }
    0
}
unsafe extern "sysv64" fn join(id: usize, result: *mut *mut c_void) -> i32 {
    if ID.with(|v| v.get()) == id {
        return EDEADLK;
    }
    let handle = match state().lock() {
        Ok(mut s) => s.handles.remove(&id),
        Err(_) => return ESRCH,
    };
    let Some(handle) = handle else {
        return ESRCH;
    };
    match handle.join() {
        Ok(value) => {
            if !result.is_null() {
                unsafe {
                    *result = value as *mut c_void;
                }
            }
            0
        }
        Err(_) => ESRCH,
    }
}
unsafe extern "sysv64" fn detach(id: usize) -> i32 {
    /* Detaching only relinquishes join ownership; the runtime still reaps the
     * thread at teardown before any code or TLS is unmapped. */
    let handle = match state().lock() {
        Ok(mut s) => s.handles.remove(&id),
        Err(_) => return ESRCH,
    };
    match handle {
        /* Dropping the handle means the guest no longer joins it; the runtime
         * still reaps the thread at teardown before code or TLS is unmapped. */
        Some(_) => 0,
        None => ESRCH,
    }
}
extern "sysv64" fn current() -> usize {
    ID.with(|v| v.get())
}
extern "sysv64" fn equal(a: usize, b: usize) -> i32 {
    i32::from(a == b)
}
pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_pthread_create" => create as *const () as u64,
        "_pthread_join" => join as *const () as u64,
        "_pthread_detach" => detach as *const () as u64,
        "_pthread_self" => current as *const () as u64,
        "_pthread_equal" => equal as *const () as u64,
        "_pthread_attr_init" => attr_init as *const () as u64,
        "_pthread_attr_destroy" => attr_destroy as *const () as u64,
        "_pthread_attr_setstacksize" => attr_setstacksize as *const () as u64,
        "_pthread_get_stackaddr_np" => get_stackaddr as *const () as u64,
        "_pthread_get_stacksize_np" => get_stacksize as *const () as u64,
        "_pthread_setname_np" => setname as *const () as u64,
        "_pthread_threadid_np" => threadid as *const () as u64,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_default_and_validate_stack_size() {
        let mut attributes = [0u8; 64];
        let a = attributes.as_mut_ptr() as *mut c_void;
        unsafe {
            assert_eq!(attr_setstacksize(a, 1 << 20), EINVAL);
            assert_eq!(attr_init(a), 0);
            assert_eq!(attribute_stack_size(a), Some(0));
            assert_eq!(attr_setstacksize(a, 1024), EINVAL);
            assert_eq!(attr_setstacksize(a, 1 << 20), 0);
            assert_eq!(attribute_stack_size(a), Some(1 << 20));
            assert_eq!(attr_destroy(a), 0);
            assert_eq!(attr_destroy(a), EINVAL);
            assert_eq!(attribute_stack_size(std::ptr::null()), Some(0));
            assert_eq!(attribute_stack_size(std::ptr::dangling::<c_void>()), None);
        }
    }
}
