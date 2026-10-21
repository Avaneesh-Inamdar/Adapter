//! Native C translates SysV calls and Darwin data structures to Windows CRT
//! operations. Guest FILE and va_list objects are never passed to the CRT.
#[cfg(windows)]
unsafe extern "C" {
    fn db_libc_symbol(name: *const std::ffi::c_char) -> u64;
    fn db_libc_begin();
    fn db_libc_flush();
    fn db_set_executable_path(path: *const std::ffi::c_char);
    fn db_set_arguments(argc: i32, argv: *const *const std::ffi::c_char);
}

pub fn set_executable_path(path: &std::ffi::CStr) {
    #[cfg(windows)]
    unsafe {
        db_set_executable_path(path.as_ptr());
    }
}

/// # Safety
/// `argv` must point to `argc` readable C string pointers that outlive guest use.
pub unsafe fn set_arguments(argc: i32, argv: *const *const std::ffi::c_char) {
    #[cfg(windows)]
    unsafe {
        db_set_arguments(argc, argv);
    }
    #[cfg(not(windows))]
    {
        let _ = (argc, argv);
    }
}

pub fn symbol(name: &str) -> Option<u64> {
    #[cfg(windows)]
    {
        let name = std::ffi::CString::new(name).ok()?;
        let value = unsafe { db_libc_symbol(name.as_ptr()) };
        (value != 0).then_some(value)
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

pub fn begin() {
    #[cfg(windows)]
    unsafe {
        db_libc_begin()
    }
}

pub fn flush() {
    #[cfg(windows)]
    unsafe {
        db_libc_flush()
    }
}

#[unsafe(no_mangle)]
extern "C" fn db_set_errno(value: i32) {
    super::thread_state::set_errno(value);
}

#[unsafe(no_mangle)]
extern "C" fn db_get_errno() -> i32 {
    super::thread_state::errno_value()
}

#[unsafe(no_mangle)]
extern "sysv64" fn db_guest_exit(code: i32) -> ! {
    if std::env::var_os("ADAPTER_TRACE_EXIT").is_some() {
        eprintln!("adapter-run [exit]: guest requested {code}");
    }
    if let Err(error) = super::callbacks::finalize(0) {
        eprintln!("adapter-run [exit]: {error}");
        std::process::exit(125);
    }
    flush();
    std::process::exit(code)
}
