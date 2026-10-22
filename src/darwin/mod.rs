//! Narrow libSystem adapters. All guest interfaces use Darwin's x86-64 SysV
//! calling convention, including when compiled for the Windows MSVC target.
pub mod blocks;
pub mod callbacks;
pub mod dispatch;
pub mod dl;
pub mod dyld;
pub mod libc;
pub mod mach;
pub mod qos;
pub mod signal;
pub mod stack;
pub mod sync;
pub mod sysctl;
pub mod thread_state;
pub mod threads;
pub mod tlv;
pub mod vm;
pub mod xpc;
use std::{
    ffi::{CStr, c_char, c_void},
    io::Write,
};
unsafe extern "C" {
    #[link_name = "malloc"]
    fn host_malloc(n: usize) -> *mut c_void;
    #[link_name = "free"]
    fn host_free(p: *mut c_void);
}
unsafe extern "sysv64" fn puts(s: *const c_char) -> i32 {
    let bytes = unsafe { CStr::from_ptr(s) }.to_bytes();
    let mut out = std::io::stdout().lock();
    if out
        .write_all(bytes)
        .and_then(|_| out.write_all(b"\n"))
        .is_err()
    {
        -1
    } else {
        0
    }
}
unsafe extern "sysv64" fn strlen(s: *const c_char) -> usize {
    unsafe { CStr::from_ptr(s) }.to_bytes().len()
}
unsafe extern "sysv64" fn malloc(n: usize) -> *mut c_void {
    let value = unsafe { host_malloc(n) };
    if value.is_null() && n != 0 {
        thread_state::set_errno(12);
    }
    value
}
unsafe extern "sysv64" fn free(p: *mut c_void) {
    unsafe { host_free(p) }
}

pub fn guest_alloc(n: usize) -> *mut u8 {
    unsafe { host_malloc(n).cast() }
}

unsafe extern "sysv64" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if n != 0 {
        unsafe { std::ptr::copy_nonoverlapping(src, dst, n) }
    }
    dst
}
unsafe extern "sysv64" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let (a, b) = unsafe { (*a.add(i), *b.add(i)) };
        if a != b {
            return a as i32 - b as i32;
        }
    }
    0
}
pub fn symbol(name: &str) -> Option<u64> {
    if let Some(value) = sysctl::symbol(name) {
        return Some(value);
    }
    if let Some(value) = qos::symbol(name) {
        return Some(value);
    }
    if let Some(value) = dispatch::symbol(name) {
        return Some(value);
    }
    if let Some(value) = signal::symbol(name) {
        return Some(value);
    }
    if let Some(value) = libc::symbol(name) {
        return Some(value);
    }
    if let Some(value) = blocks::symbol(name) {
        return Some(value);
    }
    if let Some(value) = xpc::symbol(name) {
        return Some(value);
    }
    if let Some(value) = thread_state::symbol(name) {
        return Some(value);
    }
    if let Some(value) = threads::symbol(name) {
        return Some(value);
    }
    if let Some(value) = sync::symbol(name) {
        return Some(value);
    }
    if let Some(value) = tlv::symbol(name) {
        return Some(value);
    }
    if let Some(value) = callbacks::symbol(name) {
        return Some(value);
    }
    if let Some(value) = dl::symbol(name) {
        return Some(value);
    }
    if let Some(value) = dyld::symbol(name) {
        return Some(value);
    }
    if let Some(value) = mach::symbol(name) {
        return Some(value);
    }
    if let Some(value) = vm::symbol(name) {
        return Some(value);
    }
    if let Some(value) = stack::symbol(name) {
        return Some(value);
    }
    Some(match name {
        "dyld_stub_binder" => unexpected_lazy_bind as *const () as u64,
        "_puts" => puts as *const () as u64,
        "_strlen" => strlen as *const () as u64,
        "_malloc" => malloc as *const () as u64,
        "_free" => free as *const () as u64,
        "_memcpy" => memcpy as *const () as u64,
        "_memcmp" => memcmp as *const () as u64,
        _ => return None,
    })
}
extern "sysv64" fn unexpected_lazy_bind() -> ! {
    eprintln!("adapter-run [bind]: unexpected lazy binder call after eager binding");
    std::process::exit(125)
}
