//! Default, recursive and error-check pthread mutex adapters with Apple's
//! public LP64 layout: `_opaque_pthread_mutex_t` is `long __sig` plus 56
//! opaque bytes. The opaque head hosts a zero-initialized SRWLOCK, matching
//! static initializers; recursive mutexes additionally track owner and depth.
//! Apple's internal signatures come from the pinned `pthread_impl.h` artifact:
//! normal 0x32AAABA7, recursive 0x32AAABA2, error-check 0x32AAABA1.
//!
//! Condition variables use `_opaque_pthread_cond_t` (`long __sig` plus 40
//! opaque bytes) and 16-byte condattrs, per the pinned Xcode SDK
//! `_pthread_types.h`. The opaque head hosts a Windows CONDITION_VARIABLE that
//! waits on the SRWLOCK inside the adapter's mutex; Darwin's default pshared
//! value is 2 (`PTHREAD_PROCESS_PRIVATE`). Process-shared operation is not
//! supported because the host condition variable is process-local.
#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
const NORMAL: i64 = 0x32AAABA7; /* Apple _PTHREAD_MUTEX_SIG_init */
#[cfg(windows)]
const RECURSIVE: i64 = 0x32AAABA2; /* Apple _PTHREAD_RECURSIVE_MUTEX_SIG_init */
#[cfg(windows)]
const MUTEXATTR_SIG: i64 = 0x4D55544154545231; /* adapter-private: no public initializer exists */
#[cfg(windows)]
const COND_SIG: i64 = 0x3CB0B1BB; /* Apple _PTHREAD_COND_SIG_init */
#[cfg(windows)]
const CONDATTR_SIG: i64 = 0x434F4E4441545452; /* adapter-private: no public initializer exists */
#[cfg(windows)]
const PTHREAD_PROCESS_SHARED: i32 = 1;
#[cfg(windows)]
const PTHREAD_PROCESS_PRIVATE: i32 = 2;
const EINVAL: i32 = 22;
const EBUSY: i32 = 16;
const EPERM: i32 = 1;
const ENOTSUP: i32 = 45;
const ETIMEDOUT: i32 = 60;

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
unsafe fn cond_word(
    cond: *mut c_void,
) -> *mut windows_sys::Win32::System::Threading::CONDITION_VARIABLE {
    unsafe { (cond as *mut u8).add(8) as *mut _ }
}

#[cfg(windows)]
fn cond_valid(cond: *const c_void) -> bool {
    !cond.is_null() && unsafe { std::ptr::read_unaligned(cond as *const i64) } == COND_SIG
}

#[cfg(windows)]
unsafe fn condattr_pshared(attributes: *const c_void) -> Option<i32> {
    if attributes.is_null() {
        return Some(PTHREAD_PROCESS_PRIVATE);
    }
    unsafe {
        if std::ptr::read_unaligned(attributes as *const i64) != CONDATTR_SIG {
            return None;
        }
        Some(std::ptr::read_unaligned(
            (attributes as *const u8).add(8) as *const i32
        ))
    }
}

#[cfg(windows)]
unsafe fn cond_timeout_ms(timespec: *const c_void) -> Result<u32, i32> {
    if timespec.is_null() {
        return Err(EINVAL);
    }
    let (sec, nsec) = unsafe {
        (
            std::ptr::read_unaligned(timespec as *const i64),
            std::ptr::read_unaligned((timespec as *const i64).add(1)),
        )
    };
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return Err(EINVAL);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| EINVAL)?;
    let deadline = std::time::Duration::new(sec as u64, nsec as u32);
    if deadline <= now {
        return Ok(0);
    }
    Ok((deadline - now)
        .as_millis()
        .max(1)
        .min(u32::MAX as u128 - 1) as u32)
}

#[cfg(windows)]
unsafe fn cond_attributes(attributes: *const c_void) -> Result<(), i32> {
    if attributes.is_null() {
        return Ok(());
    }
    match unsafe { condattr_pshared(attributes) } {
        Some(PTHREAD_PROCESS_SHARED) | Some(PTHREAD_PROCESS_PRIVATE) => Ok(()),
        _ => Err(EINVAL),
    }
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

#[cfg(windows)]
unsafe extern "sysv64" fn condattr_init(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(attributes as *mut u8, 0, 16);
        std::ptr::write_unaligned(
            (attributes as *mut u8).add(8) as *mut i32,
            PTHREAD_PROCESS_PRIVATE,
        );
        std::ptr::write_unaligned(attributes as *mut i64, CONDATTR_SIG);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn condattr_destroy(attributes: *mut c_void) -> i32 {
    if attributes.is_null()
        || unsafe { std::ptr::read_unaligned(attributes as *const i64) } != CONDATTR_SIG
    {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_bytes(attributes as *mut u8, 0, 16);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn condattr_getpshared(attributes: *const c_void, output: *mut i32) -> i32 {
    if output.is_null() {
        return EINVAL;
    }
    let Some(shared) = (unsafe { condattr_pshared(attributes) }) else {
        return EINVAL;
    };
    unsafe {
        *output = shared;
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn condattr_setpshared(attributes: *mut c_void, shared: i32) -> i32 {
    if attributes.is_null() || !(PTHREAD_PROCESS_SHARED..=PTHREAD_PROCESS_PRIVATE).contains(&shared)
    {
        return EINVAL;
    }
    if unsafe { std::ptr::read_unaligned(attributes as *const i64) } != CONDATTR_SIG {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_unaligned((attributes as *mut u8).add(8) as *mut i32, shared);
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn cond_init(cond: *mut c_void, attributes: *const c_void) -> i32 {
    if cond.is_null() {
        return EINVAL;
    }
    if let Err(error) = unsafe { cond_attributes(attributes) } {
        return error;
    }
    unsafe {
        std::ptr::write_bytes(cond as *mut u8, 0, 48);
        windows_sys::Win32::System::Threading::InitializeConditionVariable(cond_word(cond));
        /* Store the signature last so a visible tag implies valid opaque data. */
        std::ptr::write_unaligned(cond as *mut i64, COND_SIG);
    }
    0
}

#[cfg(windows)]
unsafe fn cond_wait_common(cond: *mut c_void, mutex: *mut c_void, timeout: u32) -> i32 {
    if !cond_valid(cond) {
        return EINVAL;
    }
    let Some(sig) = mutex_type(mutex) else {
        return EINVAL;
    };
    if sig != NORMAL {
        return EINVAL;
    }
    let result = unsafe {
        windows_sys::Win32::System::Threading::SleepConditionVariableSRW(
            cond_word(cond),
            lock_word(mutex),
            timeout,
            0,
        )
    };
    if result != 0 {
        return 0;
    }
    if timeout != u32::MAX
        && unsafe { windows_sys::Win32::Foundation::GetLastError() }
            == windows_sys::Win32::Foundation::ERROR_TIMEOUT
    {
        ETIMEDOUT
    } else {
        EINVAL
    }
}

#[cfg(windows)]
unsafe extern "sysv64" fn cond_wait(cond: *mut c_void, mutex: *mut c_void) -> i32 {
    unsafe { cond_wait_common(cond, mutex, u32::MAX) }
}

#[cfg(windows)]
unsafe extern "sysv64" fn cond_timedwait(
    cond: *mut c_void,
    mutex: *mut c_void,
    timespec: *const c_void,
) -> i32 {
    let timeout = match unsafe { cond_timeout_ms(timespec) } {
        Ok(timeout) => timeout,
        Err(error) => return error,
    };
    unsafe { cond_wait_common(cond, mutex, timeout) }
}

#[cfg(windows)]
unsafe extern "sysv64" fn cond_signal(cond: *mut c_void) -> i32 {
    if !cond_valid(cond) {
        return EINVAL;
    }
    unsafe {
        windows_sys::Win32::System::Threading::WakeConditionVariable(cond_word(cond));
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn cond_broadcast(cond: *mut c_void) -> i32 {
    if !cond_valid(cond) {
        return EINVAL;
    }
    unsafe {
        windows_sys::Win32::System::Threading::WakeAllConditionVariable(cond_word(cond));
    }
    0
}

#[cfg(windows)]
unsafe extern "sysv64" fn cond_destroy(cond: *mut c_void) -> i32 {
    if !cond_valid(cond) {
        return EINVAL;
    }
    /* Waiters make destruction undefined, matching Darwin's contract. */
    unsafe {
        std::ptr::write_bytes(cond as *mut u8, 0, 48);
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
            "_pthread_cond_init" => cond_init as *const () as u64,
            "_pthread_cond_destroy" => cond_destroy as *const () as u64,
            "_pthread_cond_wait" => cond_wait as *const () as u64,
            "_pthread_cond_timedwait" => cond_timedwait as *const () as u64,
            "_pthread_cond_signal" => cond_signal as *const () as u64,
            "_pthread_cond_broadcast" => cond_broadcast as *const () as u64,
            "_pthread_condattr_init" => condattr_init as *const () as u64,
            "_pthread_condattr_destroy" => condattr_destroy as *const () as u64,
            "_pthread_condattr_getpshared" => condattr_getpshared as *const () as u64,
            "_pthread_condattr_setpshared" => condattr_setpshared as *const () as u64,
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

    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }

    #[test]
    fn condition_variable_times_out_and_rejects_stale_tags() {
        let mut mutex = [0u8; 64];
        let mut cond = [0u8; 48];
        let mut attr = [0u8; 16];
        let m = mutex.as_mut_ptr() as *mut c_void;
        let c = cond.as_mut_ptr() as *mut c_void;
        let a = attr.as_mut_ptr() as *mut c_void;
        unsafe {
            assert_eq!(init(m, std::ptr::null()), 0);
            assert_eq!(condattr_init(a), 0);
            let mut shared = 0;
            assert_eq!(condattr_getpshared(a, &mut shared), 0);
            assert_eq!(shared, PTHREAD_PROCESS_PRIVATE);
            assert_eq!(condattr_setpshared(a, PTHREAD_PROCESS_SHARED), 0);
            assert_eq!(condattr_getpshared(a, &mut shared), 0);
            assert_eq!(shared, PTHREAD_PROCESS_SHARED);
            assert_eq!(condattr_setpshared(a, 3), EINVAL);
            assert_eq!(cond_init(c, a), 0);
            assert_eq!(cond_signal(c), 0);
            assert_eq!(cond_broadcast(c), 0);
            assert_eq!(lock(m), 0);
            let past = Timespec { sec: 1, nsec: 0 };
            assert_eq!(
                cond_timedwait(c, m, &past as *const Timespec as *const c_void),
                ETIMEDOUT
            );
            assert_eq!(try_lock(m), EBUSY);
            assert_eq!(unlock(m), 0);
            let invalid = Timespec {
                sec: 1,
                nsec: 1_000_000_000,
            };
            assert_eq!(
                cond_timedwait(c, m, &invalid as *const Timespec as *const c_void),
                EINVAL
            );
            assert_eq!(destroy(m), 0);
            assert_eq!(cond_destroy(c), 0);
            assert_eq!(cond_signal(c), EINVAL);
            assert_eq!(condattr_destroy(a), 0);
            assert_eq!(condattr_destroy(a), EINVAL);
        }
    }

    #[test]
    fn statically_initialized_condition_variable_signals_without_init() {
        let mut cond = [0u8; 48];
        cond[..8].copy_from_slice(&COND_SIG.to_ne_bytes());
        let c = cond.as_mut_ptr() as *mut c_void;
        unsafe {
            assert_eq!(cond_signal(c), 0);
            assert_eq!(cond_broadcast(c), 0);
            assert_eq!(cond_destroy(c), 0);
            assert_eq!(cond_signal(c), EINVAL);
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
