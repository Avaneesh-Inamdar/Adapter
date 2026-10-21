//! Darwin `dlfcn` adapters over the runtime's guest image registry.
use crate::runtime::{self, RTLD_DEFAULT, RTLD_MAIN_ONLY, RTLD_NEXT, RTLD_SELF};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_void},
};

struct Errors {
    current: Option<CString>,
    returned: Option<CString>,
}

thread_local! {
    static ERRORS: RefCell<Errors> = const {
        RefCell::new(Errors {
            current: None,
            returned: None,
        })
    };
}

fn set_error(message: String) {
    let message = CString::new(message.replace('\0', "\\0"))
        .unwrap_or_else(|_| CString::new("dlfcn error").unwrap());
    ERRORS.with(|errors| {
        let mut errors = errors.borrow_mut();
        errors.returned = None;
        errors.current = Some(message);
    });
}

unsafe extern "sysv64" fn dlopen(path: *const c_char, mode: i32) -> *mut c_void {
    if path.is_null() {
        return RTLD_DEFAULT as usize as *mut c_void;
    }
    let name = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(name) => name,
        Err(_) => {
            set_error("dlopen: path is not valid UTF-8".into());
            return std::ptr::null_mut();
        }
    };
    match runtime::dynamic_load(name, mode) {
        Ok(handle) => handle as *mut c_void,
        Err(error) => {
            set_error(error);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "sysv64" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    if symbol.is_null() {
        set_error("dlsym: null symbol name".into());
        return std::ptr::null_mut();
    }
    let name = match unsafe { CStr::from_ptr(symbol) }.to_str() {
        Ok(name) => name,
        Err(_) => {
            set_error("dlsym: symbol name is not valid UTF-8".into());
            return std::ptr::null_mut();
        }
    };
    match runtime::dynamic_symbol(handle as isize, name) {
        Ok(Some(value)) => value as *mut c_void,
        Ok(None) => {
            set_error(format!("dlsym: symbol {name} not found"));
            std::ptr::null_mut()
        }
        Err(error) => {
            set_error(error);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "sysv64" fn dlclose(handle: *mut c_void) -> i32 {
    match runtime::dynamic_close(handle as isize) {
        Ok(()) => 0,
        Err(error) => {
            set_error(error);
            -1
        }
    }
}

unsafe extern "sysv64" fn dlerror() -> *mut c_char {
    ERRORS.with(|errors| {
        let mut errors = errors.borrow_mut();
        errors.returned = None;
        match errors.current.take() {
            Some(message) => {
                let pointer = message.as_ptr() as *mut c_char;
                errors.returned = Some(message);
                pointer
            }
            None => std::ptr::null_mut(),
        }
    })
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_dlopen" => dlopen as *const () as u64,
        "_dlsym" => dlsym as *const () as u64,
        "_dlclose" => dlclose as *const () as u64,
        "_dlerror" => dlerror as *const () as u64,
        _ => return None,
    })
}

const _: () = {
    assert!(RTLD_DEFAULT == -2 && RTLD_MAIN_ONLY == -5);
    assert!(RTLD_NEXT == -1 && RTLD_SELF == -3);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_string_is_returned_once_and_can_be_replaced() {
        set_error("first".into());
        let first = unsafe { dlerror() };
        assert!(!first.is_null());
        assert_eq!(unsafe { CStr::from_ptr(first) }.to_str().unwrap(), "first");
        assert!(unsafe { dlerror() }.is_null());
        set_error("second".into());
        let second = unsafe { dlerror() };
        assert_eq!(
            unsafe { CStr::from_ptr(second) }.to_str().unwrap(),
            "second"
        );
        assert!(unsafe { dlerror() }.is_null());
    }
}
