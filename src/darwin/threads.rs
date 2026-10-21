//! Narrow Darwin pthread ABI adapters over owned Windows-hosted Rust threads.
//! pthread_t is an opaque pointer-width token here, never a Windows HANDLE.
use crate::Result;
use std::{
    cell::Cell,
    collections::BTreeMap,
    ffi::c_void,
    sync::{Condvar, Mutex, OnceLock},
    thread::JoinHandle,
};
const EINVAL: i32 = 22;
const ESRCH: i32 = 3;
const EDEADLK: i32 = 11;
const EAGAIN: i32 = 35;
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
unsafe extern "sysv64" fn create(
    output: *mut usize,
    attributes: *const c_void,
    start: Option<unsafe extern "sysv64" fn(*mut c_void) -> *mut c_void>,
    argument: *mut c_void,
) -> i32 {
    let Some(start) = start else {
        return EINVAL;
    };
    if output.is_null()
        || !attributes.is_null()
        || !super::callbacks::executable(start as *const () as u64)
    {
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
    let handle = std::thread::Builder::new()
        .name(format!("Adapter-{id}"))
        .spawn(move || {
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
        "_pthread_self" => current as *const () as u64,
        "_pthread_equal" => equal as *const () as u64,
        _ => return None,
    })
}
