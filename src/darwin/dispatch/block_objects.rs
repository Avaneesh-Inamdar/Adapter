//! `dispatch_block_create` and friends: blocks that can be waited on, cancelled
//! and chained with a notification.
//!
//! The wrapper is a heap block whose invoke function lives in the host. It runs
//! the captured block unless it was cancelled first, then wakes waiters and
//! queues the notifications. QoS flags are validated but never applied, and the
//! `DISPATCH_BLOCK_BARRIER` flag does not turn a later `dispatch_async` into a
//! barrier.
use super::*;
use std::sync::OnceLock;

const FLAGS_VALID: usize = 0x3f;
const WRAPPER_SIZE: usize = 48;
/// Offset of the captured block inside the wrapper, after the 32-byte header.
const INNER_OFFSET: usize = 32;

pub(super) struct BlockState {
    inner: usize,
    started: bool,
    done: bool,
    cancelled: bool,
    notifies: Vec<(usize, Work)>,
}

fn descriptor() -> usize {
    static DESCRIPTOR: OnceLock<usize> = OnceLock::new();
    *DESCRIPTOR.get_or_init(|| {
        let table: &'static mut [usize; 4] = Box::leak(Box::new([
            0,
            WRAPPER_SIZE,
            0,
            dispose as *const () as usize,
        ]));
        table.as_ptr() as usize
    })
}

/// The invoke address of every wrapper, so `check_block` accepts host code.
pub(super) fn invoke_address() -> usize {
    invoke as *const () as usize
}

extern "sysv64" fn invoke(block: usize) {
    let (inner, skip) = {
        let mut guard = state();
        match guard.blocks.get_mut(&block) {
            Some(entry) => {
                entry.started = true;
                (entry.inner, entry.cancelled)
            }
            None => return,
        }
    };
    if !skip {
        unsafe { blocks::invoke_void(inner as *const c_void) };
    }
    let mut guard = state();
    let ready = match guard.blocks.get_mut(&block) {
        Some(entry) => {
            entry.done = true;
            std::mem::take(&mut entry.notifies)
        }
        None => return,
    };
    for (queue, work) in ready {
        push_locked(
            &mut guard,
            queue,
            Item {
                work,
                barrier: false,
                group: None,
                slot: None,
            },
        );
        release_object(&mut guard, queue);
    }
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

/// Descriptor dispose helper: runs when the last reference to the wrapper goes.
extern "sysv64" fn dispose(block: usize) {
    let removed = state().blocks.remove(&block);
    if let Some(entry) = removed {
        unsafe { blocks::release(entry.inner as *mut c_void) };
    }
}

fn create(flags: usize, block: *const c_void, operation: &str) -> usize {
    if flags & !FLAGS_VALID != 0 {
        return 0;
    }
    check_block(block, operation);
    let inner = unsafe { blocks::copy(block) } as usize;
    if inner == 0 {
        fatal(&format!("{operation} could not copy its block"));
    }
    let wrapper =
        unsafe { blocks::new_heap_block(WRAPPER_SIZE, invoke_address(), descriptor()) } as usize;
    if wrapper == 0 {
        fatal(&format!("{operation} could not allocate its wrapper"));
    }
    unsafe { *((wrapper + INNER_OFFSET) as *mut usize) = inner };
    state().blocks.insert(
        wrapper,
        BlockState {
            inner,
            started: false,
            done: false,
            cancelled: false,
            notifies: Vec::new(),
        },
    );
    wrapper
}

extern "sysv64" fn dispatch_block_create(flags: usize, block: *const c_void) -> usize {
    create(flags, block, "dispatch_block_create")
}

extern "sysv64" fn dispatch_block_create_with_qos_class(
    flags: usize,
    _class: u32,
    relative: i32,
    block: *const c_void,
) -> usize {
    if !(-15..=0).contains(&relative) {
        return 0;
    }
    create(flags, block, "dispatch_block_create_with_qos_class")
}

extern "sysv64" fn dispatch_block_perform(flags: usize, block: *const c_void) {
    if flags & !FLAGS_VALID != 0 {
        fatal("dispatch_block_perform received invalid flags");
    }
    check_block(block, "dispatch_block_perform");
    unsafe { blocks::invoke_void(block) };
}

extern "sysv64" fn dispatch_block_wait(block: usize, timeout: u64) -> i64 {
    wait_for_block(block, timeout)
}

pub(super) fn wait_for_block(block: usize, timeout: u64) -> i64 {
    let timeout = monotonic_deadline(timeout);
    let mut guard = state();
    loop {
        match guard.blocks.get(&block) {
            Some(entry) if entry.done => return 0,
            Some(_) => {}
            None => fatal("dispatch_block_wait on a block not made by dispatch_block_create"),
        }
        if timeout == FOREVER {
            guard = wait(guard);
            continue;
        }
        let current = now();
        if current >= timeout {
            return TIMED_OUT;
        }
        guard = CV
            .wait_timeout(guard, Duration::from_nanos(timeout - current))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

extern "sysv64" fn dispatch_block_notify(
    block: usize,
    address: usize,
    notification: *const c_void,
) {
    notify(block, address, notification);
}

pub(super) fn notify(block: usize, address: usize, notification: *const c_void) {
    let work = copy_block(notification, "dispatch_block_notify");
    let mut guard = state();
    if queue(&guard, address).is_none() {
        fatal("dispatch_block_notify on an invalid dispatch queue");
    }
    let done = match guard.blocks.get(&block) {
        Some(entry) => entry.done,
        None => fatal("dispatch_block_notify on a block not made by dispatch_block_create"),
    };
    if done {
        push_locked(
            &mut guard,
            address,
            Item {
                work,
                barrier: false,
                group: None,
                slot: None,
            },
        );
        pump(&mut guard);
    } else {
        retain_object(&mut guard, address);
        if let Some(entry) = guard.blocks.get_mut(&block) {
            entry.notifies.push((address, work));
        }
    }
    drop(guard);
    CV.notify_all();
}

/// Cancels a wrapper; returns false when `block` is not one.
pub(super) fn cancel(block: usize) -> bool {
    match state().blocks.get_mut(&block) {
        Some(entry) => {
            entry.cancelled = true;
            true
        }
        None => false,
    }
}

pub(super) fn cancelled(block: usize) -> Option<bool> {
    state().blocks.get(&block).map(|entry| entry.cancelled)
}

extern "sysv64" fn dispatch_block_cancel(block: usize) {
    if !cancel(block) {
        fatal("dispatch_block_cancel on a block not made by dispatch_block_create");
    }
}

extern "sysv64" fn dispatch_block_testcancel(block: usize) -> i64 {
    match cancelled(block) {
        Some(value) => i64::from(value),
        None => fatal("dispatch_block_testcancel on a block not made by dispatch_block_create"),
    }
}

pub(super) fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_dispatch_block_create" => dispatch_block_create as *const () as u64,
        "_dispatch_block_create_with_qos_class" => {
            dispatch_block_create_with_qos_class as *const () as u64
        }
        "_dispatch_block_perform" => dispatch_block_perform as *const () as u64,
        "_dispatch_block_wait" => dispatch_block_wait as *const () as u64,
        "_dispatch_block_notify" => dispatch_block_notify as *const () as u64,
        "_dispatch_block_cancel" => dispatch_block_cancel as *const () as u64,
        "_dispatch_block_testcancel" => dispatch_block_testcancel as *const () as u64,
        _ => return None,
    })
}
