//! pthread and libsystem quality-of-service classes.
//!
//! The class and relative priority are recorded per thread and reported back,
//! but they never change host scheduling: Windows threads keep their normal
//! priority. A pthread attribute stores its requested class in the reserved
//! bytes after the stack size; `pthread_create` does not apply it.
use crate::darwin::threads;
use std::{cell::Cell, ffi::c_void};

const EINVAL: i32 = 22;
const ATTR_SIG: i64 = 0x1410FAB3;
const USER_INTERACTIVE: u32 = 0x21;
const DEFAULT: u32 = 0x15;

fn valid_class(class: u32) -> bool {
    matches!(class, 0x21 | 0x19 | 0x15 | 0x11 | 0x09)
}

fn valid_priority(priority: i32) -> bool {
    (-15..=0).contains(&priority)
}

thread_local! {
    static CURRENT: Cell<Option<(u32, i32)>> = const { Cell::new(None) };
}

fn current() -> (u32, i32) {
    CURRENT.with(|cell| {
        cell.get().unwrap_or(if threads::current_id() == 1 {
            (USER_INTERACTIVE, 0)
        } else {
            (DEFAULT, 0)
        })
    })
}

extern "sysv64" fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32 {
    if !valid_class(class) || !valid_priority(priority) {
        return EINVAL;
    }
    CURRENT.with(|cell| cell.set(Some((class, priority))));
    0
}

extern "sysv64" fn pthread_get_qos_class_np(
    thread: usize,
    class: *mut u32,
    priority: *mut i32,
) -> i32 {
    // Only the calling thread's record is readable; other threads report the default.
    let (value, relative) = if thread == threads::current_id() || thread == 0 {
        current()
    } else {
        (DEFAULT, 0)
    };
    unsafe {
        if !class.is_null() {
            *class = value;
        }
        if !priority.is_null() {
            *priority = relative;
        }
    }
    0
}

fn attribute_ok(attributes: *const u8) -> bool {
    !attributes.is_null()
        && unsafe { std::ptr::read_unaligned(attributes as *const i64) } == ATTR_SIG
}

extern "sysv64" fn pthread_attr_set_qos_class_np(
    attributes: *mut u8,
    class: u32,
    priority: i32,
) -> i32 {
    if !attribute_ok(attributes) || !valid_class(class) || !valid_priority(priority) {
        return EINVAL;
    }
    unsafe {
        std::ptr::write_unaligned(attributes.add(16) as *mut u32, class);
        std::ptr::write_unaligned(attributes.add(20) as *mut i32, priority);
    }
    0
}

extern "sysv64" fn pthread_attr_get_qos_class_np(
    attributes: *const u8,
    class: *mut u32,
    priority: *mut i32,
) -> i32 {
    if !attribute_ok(attributes) {
        return EINVAL;
    }
    let (value, relative) = unsafe {
        (
            std::ptr::read_unaligned(attributes.add(16) as *const u32),
            std::ptr::read_unaligned(attributes.add(20) as *const i32),
        )
    };
    unsafe {
        if !class.is_null() {
            *class = value;
        }
        if !priority.is_null() {
            *priority = relative;
        }
    }
    0
}

/// Overrides are accepted and never applied; the token only has to be non-null.
extern "sysv64" fn pthread_override_qos_class_start_np(
    _thread: usize,
    class: u32,
    priority: i32,
) -> *mut c_void {
    if !valid_class(class) || !valid_priority(priority) {
        return std::ptr::null_mut();
    }
    std::ptr::dangling_mut::<c_void>()
}

extern "sysv64" fn pthread_override_qos_class_end_np(token: *mut c_void) -> i32 {
    if token.is_null() { EINVAL } else { 0 }
}

extern "sysv64" fn qos_class_self() -> u32 {
    current().0
}

extern "sysv64" fn qos_class_main() -> u32 {
    USER_INTERACTIVE
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_pthread_set_qos_class_self_np" => pthread_set_qos_class_self_np as *const () as u64,
        "_pthread_get_qos_class_np" => pthread_get_qos_class_np as *const () as u64,
        "_pthread_attr_set_qos_class_np" => pthread_attr_set_qos_class_np as *const () as u64,
        "_pthread_attr_get_qos_class_np" => pthread_attr_get_qos_class_np as *const () as u64,
        "_pthread_override_qos_class_start_np" => {
            pthread_override_qos_class_start_np as *const () as u64
        }
        "_pthread_override_qos_class_end_np" => {
            pthread_override_qos_class_end_np as *const () as u64
        }
        "_qos_class_self" => qos_class_self as *const () as u64,
        "_qos_class_main" => qos_class_main as *const () as u64,
        _ => return None,
    })
}
