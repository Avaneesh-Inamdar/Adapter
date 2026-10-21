//! Default-type pthread mutex adapters with Apple's public LP64 layout:
//! `_opaque_pthread_mutex_t` is `long __sig` plus 56 opaque bytes, so
//! PTHREAD_MUTEX_INITIALIZER gives `{0x32AAABA7, {0}}`. The opaque head hosts a
//! zero-initialized SRWLOCK, matching UnixLAND normal mutex semantics.
#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
const NORMAL: i64 = 0x32AAABA7; /* Apple _PTHREAD_MUTEX_SIG_init */
const EINVAL: i32 = 22;
const EBUSY: i32 = 16;

#[cfg(windows)]
unsafe fn valid(mutex: *const c_void) -> bool {
    unsafe { std::ptr::read_unaligned(mutex as *const i64) == NORMAL }
}

#[cfg(windows)]
unsafe extern "sysv64" fn init(mutex: *mut c_void, attributes: *const c_void) -> i32 {
    if mutex.is_null() || !attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(mutex as *mut u8, 0, 64);
        /* Store the signature last so a visible tag implies valid opaque data. */
        std::ptr::write_unaligned(mutex as *mut i64, NORMAL);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn lock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() || !unsafe { valid(mutex) } {
        return EINVAL;
    }
    unsafe {
        windows_sys::Win32::System::Threading::AcquireSRWLockExclusive(
            (mutex as *mut u8).add(8) as *mut _
        );
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn try_lock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() || !unsafe { valid(mutex) } {
        return EINVAL;
    }
    let acquired = unsafe {
        windows_sys::Win32::System::Threading::TryAcquireSRWLockExclusive(
            (mutex as *mut u8).add(8) as *mut _
        )
    };
    if acquired { 0 } else { EBUSY }
}

#[cfg(windows)]
unsafe extern "sysv64" fn unlock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() || !unsafe { valid(mutex) } {
        return EINVAL;
    }
    unsafe {
        windows_sys::Win32::System::Threading::ReleaseSRWLockExclusive(
            (mutex as *mut u8).add(8) as *mut _
        );
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
            _ => return None,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}
