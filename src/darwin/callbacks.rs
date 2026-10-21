//! C/C++ process-exit registration for a single active guest program.
//! Never hold the registry lock across a guest call: destructors may register
//! or finalize other callbacks. Entries are removed before invocation.
use crate::Result;
use std::{
    ffi::c_void,
    sync::{Mutex, OnceLock},
};
enum Function {
    Plain(unsafe extern "sysv64" fn()),
    Argument(unsafe extern "sysv64" fn(*mut c_void), usize),
}
struct Callback {
    function: Function,
    dso: usize,
}
#[derive(Default)]
struct Registry {
    active: bool,
    ranges: Vec<(u64, u64)>,
    callbacks: Vec<Callback>,
}
fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}
pub struct Guard;
pub fn executable(address: u64) -> bool {
    registry()
        .lock()
        .is_ok_and(|r| r.active && r.ranges.iter().any(|&(a, b)| address >= a && address < b))
}
pub fn begin(ranges: Vec<(u64, u64)>) -> Result<Guard> {
    let mut r = registry()
        .lock()
        .map_err(|_| "exit callback registry poisoned")?;
    if r.active {
        return Err("concurrent guest execution in one process unsupported".into());
    }
    *r = Registry {
        active: true,
        ranges,
        callbacks: Vec::new(),
    };
    Ok(Guard)
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut r) = registry().lock() {
            *r = Registry::default();
        }
    }
}
pub fn add_ranges(ranges: Vec<(u64, u64)>) -> Result<()> {
    let mut r = registry()
        .lock()
        .map_err(|_| "exit callback registry poisoned")?;
    if !r.active {
        return Err("exit callback registry is inactive".into());
    }
    r.ranges.extend(ranges);
    Ok(())
}
fn register(function: Function, address: u64, dso: usize) -> i32 {
    let Ok(mut r) = registry().lock() else {
        return -1;
    };
    if !r.active
        || r.callbacks.len() >= 1_000_000
        || !r.ranges.iter().any(|&(a, b)| address >= a && address < b)
    {
        return -1;
    }
    r.callbacks.push(Callback { function, dso });
    0
}
unsafe extern "sysv64" fn cxa_atexit(
    function: Option<unsafe extern "sysv64" fn(*mut c_void)>,
    arg: *mut c_void,
    dso: *mut c_void,
) -> i32 {
    let Some(function) = function else {
        return -1;
    };
    register(
        Function::Argument(function, arg as usize),
        function as *const () as u64,
        dso as usize,
    )
}
unsafe extern "sysv64" fn atexit(function: Option<unsafe extern "sysv64" fn()>) -> i32 {
    let Some(function) = function else {
        return -1;
    };
    register(Function::Plain(function), function as *const () as u64, 0)
}
pub fn finalize(dso: usize) -> Result<()> {
    for _ in 0..1_000_000 {
        let callback = {
            let mut r = registry()
                .lock()
                .map_err(|_| "exit callback registry poisoned")?;
            let Some(index) = r.callbacks.iter().rposition(|c| dso == 0 || c.dso == dso) else {
                return Ok(());
            };
            r.callbacks.remove(index)
        };
        match callback.function {
            Function::Plain(f) => unsafe { f() },
            Function::Argument(f, arg) => unsafe { f(arg as *mut c_void) },
        }
    }
    Err("exit callback invocation limit exceeded".into())
}
unsafe extern "sysv64" fn cxa_finalize(dso: *mut c_void) {
    if let Err(e) = finalize(dso as usize) {
        eprintln!("adapter-run [finalize]: {e}");
        std::process::exit(125);
    }
}
pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "___cxa_atexit" => cxa_atexit as *const () as u64,
        "___cxa_finalize" => cxa_finalize as *const () as u64,
        "_atexit" => atexit as *const () as u64,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    static LOG: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    unsafe extern "sysv64" fn note(p: *mut c_void) {
        LOG.lock().unwrap().push(p as usize);
    }
    #[test]
    fn reverse_order_dso_filter_once_and_reset() {
        let address = note as *const () as u64;
        let guard = begin(vec![(address, address + 1)]).unwrap();
        unsafe {
            assert_eq!(
                cxa_atexit(
                    Some(note),
                    std::ptr::without_provenance_mut(1),
                    std::ptr::without_provenance_mut(10)
                ),
                0
            );
            assert_eq!(
                cxa_atexit(Some(note), 2usize as *mut _, 20usize as *mut _),
                0
            );
            assert_eq!(
                cxa_atexit(Some(note), 3usize as *mut _, 10usize as *mut _),
                0
            );
        }
        finalize(10).unwrap();
        finalize(10).unwrap();
        finalize(0).unwrap();
        assert_eq!(*LOG.lock().unwrap(), [3, 1, 2]);
        drop(guard);
        unsafe {
            assert_eq!(
                cxa_atexit(Some(note), std::ptr::null_mut(), std::ptr::null_mut()),
                -1
            );
        }
    }
}
