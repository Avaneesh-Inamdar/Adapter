//! Default, recursive and error-check pthread mutex adapters with Apple's
//! public LP64 layout: `_opaque_pthread_mutex_t` is `long __sig` plus 56
//! opaque bytes. The opaque head hosts a zero-initialized SRWLOCK, matching
//! static initializers; recursive mutexes additionally track owner and depth.
//! Apple's internal signatures come from the pinned `pthread_impl.h` artifact:
//! normal 0x32AAABA7, recursive 0x32AAABA2, error-check 0x32AAABA1.
#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
const NORMAL: i64 = 0x32AAABA7; /* Apple _PTHREAD_MUTEX_SIG_init */
#[cfg(windows)]
const RECURSIVE: i64 = 0x32AAABA2; /* Apple _PTHREAD_RECURSIVE_MUTEX_SIG_init */
#[cfg(windows)]
const MUTEXATTR_SIG: i64 = 0x4D55544154545231; /* adapter-private: no public initializer exists */
const EINVAL: i32 = 22;
const EBUSY: i32 = 16;
const EPERM: i32 = 1;
const ENOTSUP: i32 = 45;

#[cfg(windows)]
fn mutex_type(mutex: *const c_void) -> Option<i64> {
    let sig = unsafe { std::ptr::read_unaligned(mutex as *const i64) };
    (sig == NORMAL || sig == RECURSIVE).then_some(sig)
}

#[cfg(windows)]
unsafe fn valid(mutex: *const c_void) -> bool {
    mutex_type(mutex).is_some()
}

#[cfg(windows)]
unsafe fn lock_word(mutex: *mut c_void) -> *mut windows_sys::Win32::System::Threading::SRWLOCK {
    unsafe { (mutex as *mut u8).add(8) as *mut _ }
}

#[cfg(windows)]
unsafe fn owner_word(mutex: *mut c_void) -> *mut u32 {
    unsafe { (mutex as *mut u8).add(16) as *mut _ }
}

#[cfg(windows)]
unsafe fn depth_word(mutex: *mut c_void) -> *mut u32 {
    unsafe { (mutex as *mut u8).add(20) as *mut _ }
}

#[cfg(windows)]
unsafe fn attribute_type(attributes: *const c_void) -> Option<i64> {
    if attributes.is_null() {
        return Some(NORMAL);
    }
    unsafe {
        if std::ptr::read_unaligned(attributes as *const i64) != MUTEXATTR_SIG {
            return None;
        }
        match std::ptr::read_unaligned((attributes as *const u8).add(8) as *const i32) {
            0 => Some(NORMAL),
            2 => Some(RECURSIVE),
            _ => None,
        }
    }
}

#[cfg(windows)]
unsafe extern "sysv64" fn mutexattr_init(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(attributes as *mut u8, 0, 16);
        std::ptr::write_unaligned(attributes as *mut i64, MUTEXATTR_SIG);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn mutexattr_destroy(attributes: *mut c_void) -> i32 {
    if attributes.is_null()
        || unsafe { std::ptr::read_unaligned(attributes as *const i64) } != MUTEXATTR_SIG
    {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(attributes as *mut u8, 0, 16);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn mutexattr_settype(attributes: *mut c_void, kind: i32) -> i32 {
    if attributes.is_null()
        || unsafe { std::ptr::read_unaligned(attributes as *const i64) } != MUTEXATTR_SIG
    {
        return EINVAL;
    }
    match kind {
        0 | 2 => unsafe {
            std::ptr::write_unaligned((attributes as *mut u8).add(8) as *mut i32, kind);
            0
        },
        _ => ENOTSUP,
    }
}

#[cfg(windows)]
unsafe extern "sysv64" fn init(mutex: *mut c_void, attributes: *const c_void) -> i32 {
    let Some(sig) = (unsafe { attribute_type(attributes) }) else {
        return EINVAL;
    };
    if mutex.is_null() {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(mutex as *mut u8, 0, 64);
        /* Store the signature last so a visible tag implies valid opaque data. */
        std::ptr::write_unaligned(mutex as *mut i64, sig);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn lock(mutex: *mut c_void) -> i32 {
    let Some(sig) = mutex_type(mutex) else {
        return EINVAL;
    };
    unsafe {
        if sig == RECURSIVE {
            let thread = windows_sys::Win32::System::Threading::GetCurrentThreadId();
            if *owner_word(mutex) == thread && *depth_word(mutex) != 0 {
                *depth_word(mutex) += 1;
                return 0;
            }
            windows_sys::Win32::System::Threading::AcquireSRWLockExclusive(lock_word(mutex));
            *owner_word(mutex) = thread;
            *depth_word(mutex) = 1;
        } else {
            windows_sys::Win32::System::Threading::AcquireSRWLockExclusive(lock_word(mutex));
        }
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn try_lock(mutex: *mut c_void) -> i32 {
    let Some(sig) = mutex_type(mutex) else {
        return EINVAL;
    };
    unsafe {
        if sig == RECURSIVE {
            let thread = windows_sys::Win32::System::Threading::GetCurrentThreadId();
            if *owner_word(mutex) == thread && *depth_word(mutex) != 0 {
                *depth_word(mutex) += 1;
                return 0;
            }
            if windows_sys::Win32::System::Threading::TryAcquireSRWLockExclusive(lock_word(mutex)) {
                *owner_word(mutex) = thread;
                *depth_word(mutex) = 1;
                return 0;
            }
            return EBUSY;
        }
        if windows_sys::Win32::System::Threading::TryAcquireSRWLockExclusive(lock_word(mutex)) {
            0
        } else {
            EBUSY
        }
    }
}

#[cfg(windows)]
unsafe extern "sysv64" fn unlock(mutex: *mut c_void) -> i32 {
    let Some(sig) = mutex_type(mutex) else {
        return EINVAL;
    };
    unsafe {
        if sig == RECURSIVE {
            let thread = windows_sys::Win32::System::Threading::GetCurrentThreadId();
            if *owner_word(mutex) != thread {
                return EPERM;
            }
            *depth_word(mutex) -= 1;
            if *depth_word(mutex) == 0 {
                *owner_word(mutex) = 0;
                windows_sys::Win32::System::Threading::ReleaseSRWLockExclusive(lock_word(mutex));
            }
        } else {
            windows_sys::Win32::System::Threading::ReleaseSRWLockExclusive(lock_word(mutex));
        }
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn destroy(mutex: *mut c_void) -> i32 {
    if mutex.is_null() || !unsafe { valid(mutex) } {
        return EINVAL;
    }
    /* A held lock cannot be detected, matching Darwin's undefined contract. */
    unsafe {
        std::ptr::write_bytes(mutex as *mut u8, 0, 64);
    }
    0
}

pub fn symbol(name: &str) -> Option<u64> {
    #[cfg(windows)]
    {
        Some(match name {
            "_pthread_mutex_init" => init as *const () as u64,
            "_pthread_mutex_lock" => lock as *const () as u64,
            "_pthread_mutex_trylock" => try_lock as *const () as u64,
            "_pthread_mutex_unlock" => unlock as *const () as u64,
            "_pthread_mutex_destroy" => destroy as *const () as u64,
            "_pthread_mutexattr_init" => mutexattr_init as *const () as u64,
            "_pthread_mutexattr_destroy" => mutexattr_destroy as *const () as u64,
            "_pthread_mutexattr_settype" => mutexattr_settype as *const () as u64,
            _ => return None,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn recursive_mutex_tracks_depth_and_owner() {
        let mut mutex = [0u8; 64];
        let mut attr = [0u8; 16];
        let m = mutex.as_mut_ptr() as *mut c_void;
        let a = attr.as_mut_ptr() as *mut c_void;
        unsafe {
            assert_eq!(mutexattr_init(a), 0);
            assert_eq!(mutexattr_settype(a, 2), 0);
            assert_eq!(init(m, a), 0);
            assert_eq!(lock(m), 0);
            assert_eq!(lock(m), 0);
            assert_eq!(try_lock(m), 0);
            assert_eq!(unlock(m), 0);
            assert_eq!(unlock(m), 0);
            assert_eq!(unlock(m), 0);
            assert_eq!(unlock(m), EPERM);
            assert_eq!(destroy(m), 0);
        }
    }

    #[test]
    fn mutexattr_rejects_unsupported_types_and_stale_tags() {
        let mut attr = [0u8; 16];
        let a = attr.as_mut_ptr() as *mut c_void;
        unsafe {
            assert_eq!(mutexattr_settype(a, 0), EINVAL);
            assert_eq!(mutexattr_init(a), 0);
            assert_eq!(mutexattr_settype(a, 1), ENOTSUP);
            assert_eq!(mutexattr_settype(a, 3), ENOTSUP);
            assert_eq!(mutexattr_settype(a, 2), 0);
            assert_eq!(mutexattr_destroy(a), 0);
            assert_eq!(mutexattr_destroy(a), EINVAL);
        }
    }
}
