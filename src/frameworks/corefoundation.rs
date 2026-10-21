//! Opaque runtime-owned immutable CF strings only. These handles do not claim
//! Apple object-layout compatibility, toll-free bridging, or constant strings.
use std::{
    collections::BTreeMap,
    ffi::{CStr, c_char, c_void},
    sync::{Mutex, OnceLock},
};
const UTF8: u32 = 0x08000100;
struct StringValue {
    text: String,
    refs: usize,
}
fn strings() -> &'static Mutex<BTreeMap<usize, StringValue>> {
    static S: OnceLock<Mutex<BTreeMap<usize, StringValue>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(BTreeMap::new()))
}
unsafe extern "sysv64" fn create(
    allocator: *const c_void,
    s: *const c_char,
    encoding: u32,
) -> *const c_void {
    if !allocator.is_null() || s.is_null() || encoding != UTF8 {
        return std::ptr::null();
    }
    let Ok(text) = unsafe { CStr::from_ptr(s) }.to_str() else {
        return std::ptr::null();
    };
    let Ok(mut values) = strings().lock() else {
        return std::ptr::null();
    };
    let handle = Box::into_raw(Box::new(0u8));
    values.insert(
        handle as usize,
        StringValue {
            text: text.into(),
            refs: 1,
        },
    );
    handle as *const c_void
}
unsafe extern "sysv64" fn length(s: *const c_void) -> i64 {
    strings()
        .lock()
        .ok()
        .and_then(|v| {
            v.get(&(s as usize))
                .map(|s| s.text.encode_utf16().count() as i64)
        })
        .unwrap_or(0)
}
unsafe extern "sysv64" fn get_cstring(
    s: *const c_void,
    out: *mut u8,
    size: i64,
    encoding: u32,
) -> u8 {
    if out.is_null() || size <= 0 || encoding != UTF8 {
        return 0;
    }
    let Ok(v) = strings().lock() else {
        return 0;
    };
    let Some(s) = v.get(&(s as usize)) else {
        return 0;
    };
    if size as u64 <= s.text.len() as u64 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(s.text.as_ptr(), out, s.text.len());
        *out.add(s.text.len()) = 0;
    }
    1
}
unsafe extern "sysv64" fn retain(s: *const c_void) -> *const c_void {
    let Ok(mut v) = strings().lock() else {
        return std::ptr::null();
    };
    let Some(value) = v.get_mut(&(s as usize)) else {
        return std::ptr::null();
    };
    let Some(refs) = value.refs.checked_add(1) else {
        return std::ptr::null();
    };
    value.refs = refs;
    s
}
unsafe extern "sysv64" fn release(s: *const c_void) {
    let Ok(mut v) = strings().lock() else {
        return;
    };
    if let Some(value) = v.get_mut(&(s as usize)) {
        value.refs -= 1;
        if value.refs == 0 {
            v.remove(&(s as usize));
            unsafe {
                drop(Box::from_raw(s as *mut u8));
            }
        }
    }
}
pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_CFStringCreateWithCString" => create as *const () as u64,
        "_CFStringGetLength" => length as *const () as u64,
        "_CFStringGetCString" => get_cstring as *const () as u64,
        "_CFRetain" => retain as *const () as u64,
        "_CFRelease" => release as *const () as u64,
        _ => return None,
    })
}
