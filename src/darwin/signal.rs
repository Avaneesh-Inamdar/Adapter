//! Darwin signal dispositions, masks and synchronous delivery.
//!
//! Only delivery to the calling thread exists: `raise` and `kill` on the guest's
//! own pid run the installed handler on the guest's current stack. There is no
//! asynchronous interruption of other guest threads and no job control, so stop
//! and continue signals are recorded and ignored rather than faked.
use crate::darwin::thread_state;
use std::{collections::BTreeMap, sync::Mutex};

const SIG_DFL: u64 = 0;
const SIG_IGN: u64 = 1;
const SIG_ERR: u64 = u64::MAX;

const EINVAL: i32 = 22;
const ESRCH: i32 = 3;
const EPERM: i32 = 1;

const SIGNAL_BLOCK: i32 = 1;
const SIGNAL_UNBLOCK: i32 = 2;
const SIGNAL_SETMASK: i32 = 3;

const NSIG: i32 = 32;
const SIGKILL: i32 = 9;
const SIGSTOP: i32 = 17;

/// Darwin `struct sigaction` on x86-64: a handler/sigaction union, a 32-bit
/// `sigset_t` and the flag word.
#[repr(C)]
#[derive(Clone, Copy)]
struct Action {
    handler: u64,
    mask: u32,
    flags: i32,
}

#[derive(Clone, Default)]
struct State {
    handlers: BTreeMap<i32, u64>,
    masks: BTreeMap<i32, u32>,
    mask: u32,
    pending: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    handlers: BTreeMap::new(),
    masks: BTreeMap::new(),
    mask: 0,
    pending: 0,
});

fn bit(signal: i32) -> u32 {
    1u32 << (signal - 1)
}

fn valid(signal: i32) -> bool {
    signal > 0 && signal < NSIG
}

fn ignore_by_default(signal: i32) -> bool {
    matches!(signal, 16 | 19 | 20 | 23 | 28 | 29)
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_signal" | "__signal" => signal as *const () as u64,
        "_sigaction" | "__sigaction" => sigaction as *const () as u64,
        "_raise" | "__raise" => raise as *const () as u64,
        "_kill" | "__kill" => kill as *const () as u64,
        "_sigprocmask" | "__sigprocmask" => sigprocmask as *const () as u64,
        "_pthread_sigmask" | "__pthread_sigmask" => sigprocmask as *const () as u64,
        "_sigemptyset" | "__sigemptyset" => sigemptyset as *const () as u64,
        "_sigfillset" | "__sigfillset" => sigfillset as *const () as u64,
        "_sigaddset" | "__sigaddset" => sigaddset as *const () as u64,
        "_sigdelset" | "__sigdelset" => sigdelset as *const () as u64,
        "_sigismember" | "__sigismember" => sigismember as *const () as u64,
        _ => return None,
    })
}

fn read_set(set: *const u32) -> Option<u32> {
    if set.is_null() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(set) })
}

fn write_set(set: *mut u32, value: u32) {
    if !set.is_null() {
        unsafe { std::ptr::write_unaligned(set, value) };
    }
}

fn store_handler(state: &mut State, signal: i32, handler: u64) {
    if handler == SIG_DFL {
        state.handlers.remove(&signal);
    } else {
        state.handlers.insert(signal, handler);
    }
}

fn current_handler(signal: i32) -> u64 {
    match STATE.lock() {
        Ok(guard) => guard.handlers.get(&signal).copied().unwrap_or(SIG_DFL),
        Err(_) => SIG_DFL,
    }
}

fn terminate(signal: i32) -> ! {
    if signal == 6 {
        eprintln!("adapter-run [signal]: guest raised SIGABRT");
    } else {
        eprintln!("adapter-run [signal]: terminated by signal {signal}");
    }
    std::process::exit(128 + signal)
}

/// Run the installed disposition for one signal, applying Darwin's implicit
/// "block this signal while its handler runs" rule.
fn deliver(signal: i32) -> i32 {
    let (handler, previous_mask) = match STATE.lock() {
        Ok(mut guard) => {
            if guard.mask & bit(signal) != 0 {
                guard.pending |= bit(signal);
                return 0;
            }
            let handler = guard.handlers.get(&signal).copied().unwrap_or(SIG_DFL);
            let previous = guard.mask;
            let extra = guard.masks.get(&signal).copied().unwrap_or(0);
            guard.mask |= bit(signal) | (extra & !(bit(SIGKILL) | bit(SIGSTOP)));
            (handler, previous)
        }
        Err(_) => return -1,
    };
    match handler {
        SIG_IGN => {}
        SIG_DFL => {
            // Stop/continue and job control do not exist; those signals are
            // recorded and ignored instead of pretending to suspend.
            if !ignore_by_default(signal) && !matches!(signal, SIGSTOP | 18 | 21 | 22) {
                terminate(signal);
            }
        }
        address => {
            // A `sa_sigaction` handler ignores the extra arguments it was not
            // declared with, so one call shape serves both ABI options.
            let entry: extern "sysv64" fn(i32, usize, usize) =
                unsafe { std::mem::transmute(address) };
            entry(signal, 0, 0);
        }
    }
    if let Ok(mut guard) = STATE.lock() {
        guard.mask = previous_mask;
    }
    0
}

fn drain_pending() {
    loop {
        let next = match STATE.lock() {
            Ok(guard) => {
                let ready = guard.pending & !guard.mask;
                (1..NSIG).find(|signal| ready & bit(*signal) != 0)
            }
            Err(_) => None,
        };
        let Some(signal) = next else { return };
        if let Ok(mut guard) = STATE.lock() {
            guard.pending &= !bit(signal);
        }
        deliver(signal);
    }
}

extern "sysv64" fn sigaction(signal: i32, act: *const Action, old: *mut Action) -> i32 {
    if !valid(signal) || signal == SIGKILL || signal == SIGSTOP {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    if !old.is_null() {
        let handler = current_handler(signal);
        let mask = STATE
            .lock()
            .map(|guard| guard.masks.get(&signal).copied().unwrap_or(0))
            .unwrap_or(0);
        unsafe {
            std::ptr::write_unaligned(
                old,
                Action {
                    handler,
                    mask,
                    flags: 0,
                },
            )
        };
    }
    if !act.is_null() {
        let value = unsafe { std::ptr::read_unaligned(act) };
        match STATE.lock() {
            Ok(mut guard) => {
                store_handler(&mut guard, signal, value.handler);
                guard.masks.insert(signal, value.mask);
            }
            Err(_) => {
                thread_state::set_errno(EINVAL);
                return -1;
            }
        }
    }
    0
}

extern "sysv64" fn signal(signal: i32, handler: u64) -> u64 {
    if !valid(signal) || signal == SIGKILL || signal == SIGSTOP {
        thread_state::set_errno(EINVAL);
        return SIG_ERR;
    }
    let previous = current_handler(signal);
    match STATE.lock() {
        Ok(mut guard) => store_handler(&mut guard, signal, handler),
        Err(_) => return SIG_ERR,
    }
    previous
}

fn set_mask(how: i32, set: *const u32, old: *mut u32) -> i32 {
    if how != SIGNAL_BLOCK && how != SIGNAL_UNBLOCK && how != SIGNAL_SETMASK {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    let requested = if set.is_null() {
        0
    } else {
        read_set(set).unwrap_or(0)
    };
    match STATE.lock() {
        Ok(mut guard) => {
            write_set(old, guard.mask);
            guard.mask = match how {
                SIGNAL_BLOCK => guard.mask | requested,
                SIGNAL_UNBLOCK => guard.mask & !requested,
                _ => requested & !(bit(SIGKILL) | bit(SIGSTOP)),
            };
        }
        Err(_) => {
            thread_state::set_errno(EINVAL);
            return -1;
        }
    }
    drain_pending();
    0
}

extern "sysv64" fn sigprocmask(how: i32, set: *const u32, old: *mut u32) -> i32 {
    set_mask(how, set, old)
}

extern "sysv64" fn raise(signal: i32) -> i32 {
    if !valid(signal) {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    deliver(signal);
    0
}

extern "sysv64" fn kill(pid: i32, signal: i32) -> i32 {
    if !valid(signal) {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    if pid != std::process::id() as i32 && pid != 0 && pid != -1 {
        thread_state::set_errno(ESRCH);
        return -1;
    }
    if pid == -1 {
        thread_state::set_errno(EPERM);
        return -1;
    }
    if signal != 0 {
        deliver(signal);
    }
    0
}

extern "sysv64" fn sigemptyset(set: *mut u32) -> i32 {
    if set.is_null() {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    write_set(set, 0);
    0
}

extern "sysv64" fn sigfillset(set: *mut u32) -> i32 {
    if set.is_null() {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    write_set(set, u32::MAX);
    0
}

extern "sysv64" fn sigaddset(set: *mut u32, signal: i32) -> i32 {
    if !valid(signal) || set.is_null() {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    write_set(set, read_set(set).unwrap_or(0) | bit(signal));
    0
}

extern "sysv64" fn sigdelset(set: *mut u32, signal: i32) -> i32 {
    if !valid(signal) || set.is_null() {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    write_set(set, read_set(set).unwrap_or(0) & !bit(signal));
    0
}

extern "sysv64" fn sigismember(set: *const u32, signal: i32) -> i32 {
    if !valid(signal) || set.is_null() {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    i32::from(read_set(set).unwrap_or(0) & bit(signal) != 0)
}
