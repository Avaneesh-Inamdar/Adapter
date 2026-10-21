//! Per-thread errno, pthread key values, and address-identified pthread_once.
use crate::Result;
use std::{
    cell::{RefCell, UnsafeCell},
    collections::BTreeMap,
    ffi::c_void,
    sync::{Arc, Condvar, Mutex, OnceLock},
};
type Destructor = unsafe extern "sysv64" fn(*mut c_void);
enum OnceStatus {
    New,
    Running(std::thread::ThreadId),
    Done,
}
struct Once {
    status: Mutex<OnceStatus>,
    ready: Condvar,
}
struct Registry {
    active: bool,
    next: usize,
    keys: BTreeMap<usize, Option<Destructor>>,
    once: BTreeMap<usize, Arc<Once>>,
}
fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| {
        Mutex::new(Registry {
            active: false,
            next: 1,
            keys: BTreeMap::new(),
            once: BTreeMap::new(),
        })
    })
}
thread_local! {
    static VALUES:RefCell<BTreeMap<usize,usize>>=const {RefCell::new(BTreeMap::new())};
    static ERRNO:UnsafeCell<i32>=const {UnsafeCell::new(0)};
}
pub fn reset_thread() {
    VALUES.with(|v| v.borrow_mut().clear());
    ERRNO.with(|e| unsafe {
        *e.get() = 0;
    });
}
pub struct Guard;
pub fn begin() -> Result<Guard> {
    let mut r = registry()
        .lock()
        .map_err(|_| "thread state registry poisoned")?;
    if r.active {
        return Err("concurrent guest thread state session unsupported".into());
    }
    r.active = true;
    reset_thread();
    Ok(Guard)
}
impl Drop for Guard {
    fn drop(&mut self) {
        reset_thread();
        if let Ok(mut r) = registry().lock() {
            r.active = false;
            r.keys.clear();
            r.once.clear();
        }
    }
}
extern "sysv64" fn error() -> *mut i32 {
    ERRNO.with(|e| e.get())
}
pub fn set_errno(value: i32) {
    ERRNO.with(|e| unsafe {
        *e.get() = value;
    });
}
pub fn errno_value() -> i32 {
    ERRNO.with(|e| unsafe { *e.get() })
}
unsafe extern "sysv64" fn key_create(output: *mut usize, destructor: Option<Destructor>) -> i32 {
    if output.is_null()
        || destructor.is_some_and(|f| !super::callbacks::executable(f as *const () as u64))
    {
        return 22;
    }
    let Ok(mut r) = registry().lock() else {
        return 35;
    };
    if !r.active || r.keys.len() >= 512 {
        return 35;
    }
    let key = r.next;
    let Some(next) = key.checked_add(1) else {
        return 35;
    };
    r.next = next;
    r.keys.insert(key, destructor);
    unsafe {
        *output = key;
    }
    0
}
extern "sysv64" fn key_delete(key: usize) -> i32 {
    match registry().lock() {
        Ok(mut r) => {
            if r.keys.remove(&key).is_some() {
                0
            } else {
                22
            }
        }
        Err(_) => 22,
    }
}
extern "sysv64" fn set_specific(key: usize, value: *const c_void) -> i32 {
    if !registry()
        .lock()
        .is_ok_and(|r| r.active && r.keys.contains_key(&key))
    {
        return 22;
    }
    VALUES.with(|v| {
        let mut v = v.borrow_mut();
        if value.is_null() {
            v.remove(&key);
        } else {
            v.insert(key, value as usize);
        }
    });
    0
}
extern "sysv64" fn get_specific(key: usize) -> *mut c_void {
    if !registry()
        .lock()
        .is_ok_and(|r| r.active && r.keys.contains_key(&key))
    {
        return std::ptr::null_mut();
    }
    VALUES.with(|v| v.borrow().get(&key).copied().unwrap_or(0) as *mut c_void)
}
pub fn finish_thread() -> Result<()> {
    // Apple's public limits.h specifies four destructor passes. Clear a value
    // before the call so a destructor can request another pass with setspecific.
    for _ in 0..4 {
        let keys: Vec<_> = registry()
            .lock()
            .map_err(|_| "thread state registry poisoned")?
            .keys
            .iter()
            .filter_map(|(&key, &f)| f.map(|f| (key, f)))
            .collect();
        for (key, function) in keys {
            let value = VALUES.with(|v| v.borrow_mut().remove(&key));
            if let Some(value) = value {
                unsafe {
                    function(value as *mut c_void);
                }
            }
        }
    }
    reset_thread();
    Ok(())
}
unsafe extern "sysv64" fn once(
    control: *mut c_void,
    initialize: Option<unsafe extern "sysv64" fn()>,
) -> i32 {
    let Some(initialize) = initialize else {
        return 22;
    };
    if control.is_null() || !super::callbacks::executable(initialize as *const () as u64) {
        return 22;
    }
    if unsafe { std::ptr::read_unaligned(control as *const u64) } != 0x30b1bcba {
        return 22;
    }
    let once = {
        let Ok(mut r) = registry().lock() else {
            return 22;
        };
        if !r.active {
            return 22;
        }
        if r.once.len() >= 65536 && !r.once.contains_key(&(control as usize)) {
            return 35;
        }
        r.once
            .entry(control as usize)
            .or_insert_with(|| {
                Arc::new(Once {
                    status: Mutex::new(OnceStatus::New),
                    ready: Condvar::new(),
                })
            })
            .clone()
    };
    let Ok(mut status) = once.status.lock() else {
        return 22;
    };
    loop {
        match *status {
            OnceStatus::Done => return 0,
            OnceStatus::New => {
                *status = OnceStatus::Running(std::thread::current().id());
                break;
            }
            OnceStatus::Running(owner) => {
                if owner == std::thread::current().id() {
                    return 11;
                }
                let Ok(next) = once.ready.wait(status) else {
                    return 22;
                };
                status = next;
            }
        }
    }
    drop(status);
    unsafe {
        initialize();
    }
    let Ok(mut status) = once.status.lock() else {
        return 22;
    };
    *status = OnceStatus::Done;
    once.ready.notify_all();
    0
}
pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "___error" => error as *const () as u64,
        "_pthread_key_create" => key_create as *const () as u64,
        "_pthread_key_delete" => key_delete as *const () as u64,
        "_pthread_getspecific" => get_specific as *const () as u64,
        "_pthread_setspecific" => set_specific as *const () as u64,
        "_pthread_once" => once as *const () as u64,
        _ => return None,
    })
}
