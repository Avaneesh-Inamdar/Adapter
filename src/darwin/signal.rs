//! Darwin signal dispositions, masks and synchronous delivery.
//!
//! Only delivery to the calling thread exists: `raise` and `kill` on the guest's
//! own pid run the installed handler on the guest's current stack. There is no
//! asynchronous interruption of other guest threads and no job control, so stop
//! and continue signals are recorded and ignored rather than faked.
use crate::darwin::thread_state;
use std::{
    collections::BTreeMap,
    sync::{Condvar, Mutex},
};

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
/// Signalled whenever a blocked signal becomes pending, for `sigwait`.
static PENDING_CHANGED: Condvar = Condvar::new();

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
        "_signal" | "__signal" | "_bsd_signal" => signal as *const () as u64,
        "_killpg" => killpg as *const () as u64,
        "_sigpending" => sigpending as *const () as u64,
        "_sigwait" => sigwait as *const () as u64,
        "_sighold" => sighold as *const () as u64,
        "_sigrelse" => sigrelse as *const () as u64,
        "_sigignore" => sigignore as *const () as u64,
        "_siginterrupt" => siginterrupt as *const () as u64,
        "_sigblock" => sigblock as *const () as u64,
        "_sigsetmask" => sigsetmask as *const () as u64,
        "_psignal" => psignal as *const () as u64,
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
                PENDING_CHANGED.notify_all();
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

/// The guest is its own process group, so only that group (or 0) is addressable.
extern "sysv64" fn killpg(group: i32, signal: i32) -> i32 {
    if !valid(signal) && signal != 0 || group == 1 || group < 0 {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    if group != 0 && group != std::process::id() as i32 {
        thread_state::set_errno(ESRCH);
        return -1;
    }
    if signal != 0 {
        deliver(signal);
    }
    0
}

extern "sysv64" fn sigpending(set: *mut u32) -> i32 {
    if set.is_null() {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    let pending = STATE.lock().map(|guard| guard.pending).unwrap_or(0);
    write_set(set, pending);
    0
}

/// Waits for one of the signals in `set` to become pending and consumes it. The
/// signals are expected to be blocked, as POSIX requires; they become pending
/// when any guest thread raises them.
extern "sysv64" fn sigwait(set: *const u32, received: *mut i32) -> i32 {
    let Some(wanted) = read_set(set) else {
        return EINVAL;
    };
    let wanted = wanted & !(bit(SIGKILL) | bit(SIGSTOP));
    if wanted == 0 || received.is_null() {
        return EINVAL;
    }
    let Ok(mut guard) = STATE.lock() else {
        return EINVAL;
    };
    loop {
        let ready = guard.pending & wanted;
        if let Some(signal) = (1..NSIG).find(|signal| ready & bit(*signal) != 0) {
            guard.pending &= !bit(signal);
            unsafe { *received = signal };
            return 0;
        }
        guard = match PENDING_CHANGED.wait(guard) {
            Ok(guard) => guard,
            Err(_) => return EINVAL,
        };
    }
}

fn change_mask(signal: i32, block: bool) -> i32 {
    if !valid(signal) {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    if let Ok(mut guard) = STATE.lock() {
        if block {
            guard.mask |= bit(signal) & !(bit(SIGKILL) | bit(SIGSTOP));
        } else {
            guard.mask &= !bit(signal);
        }
    }
    if !block {
        drain_pending();
    }
    0
}

extern "sysv64" fn sighold(signal: i32) -> i32 {
    change_mask(signal, true)
}

extern "sysv64" fn sigrelse(signal: i32) -> i32 {
    change_mask(signal, false)
}

extern "sysv64" fn sigignore(signal: i32) -> i32 {
    if !valid(signal) || signal == SIGKILL || signal == SIGSTOP {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    if let Ok(mut guard) = STATE.lock() {
        store_handler(&mut guard, signal, SIG_IGN);
    }
    0
}

/// System calls here are never interrupted by a signal, so this only validates.
extern "sysv64" fn siginterrupt(signal: i32, _interrupt: i32) -> i32 {
    if !valid(signal) {
        thread_state::set_errno(EINVAL);
        return -1;
    }
    0
}

/// Legacy `int`-mask interface; returns the previous mask.
extern "sysv64" fn sigblock(mask: i32) -> i32 {
    let mut previous = 0;
    if let Ok(mut guard) = STATE.lock() {
        previous = guard.mask;
        guard.mask |= mask as u32 & !(bit(SIGKILL) | bit(SIGSTOP));
    }
    previous as i32
}

extern "sysv64" fn sigsetmask(mask: i32) -> i32 {
    let mut previous = 0;
    if let Ok(mut guard) = STATE.lock() {
        previous = guard.mask;
        guard.mask = mask as u32 & !(bit(SIGKILL) | bit(SIGSTOP));
    }
    drain_pending();
    previous as i32
}

const DESCRIPTIONS: [&str; 31] = [
    "Hangup",
    "Interrupt",
    "Quit",
    "Illegal instruction",
    "Trace/BPT trap",
    "Abort trap",
    "EMT trap",
    "Floating point exception",
    "Killed",
    "Bus error",
    "Segmentation fault",
    "Bad system call",
    "Broken pipe",
    "Alarm clock",
    "Terminated",
    "Urgent I/O condition",
    "Suspended (signal)",
    "Suspended",
    "Continued",
    "Child exited",
    "Stopped (tty input)",
    "Stopped (tty output)",
    "I/O possible",
    "Cputime limit exceeded",
    "Filesize limit exceeded",
    "Virtual timer expired",
    "Profiling timer expired",
    "Window size changes",
    "Information request",
    "User defined signal 1",
    "User defined signal 2",
];

extern "sysv64" fn psignal(signal: i32, message: *const std::ffi::c_char) {
    let description = if valid(signal) {
        DESCRIPTIONS[(signal - 1) as usize].to_string()
    } else {
        format!("Unknown signal: {signal}")
    };
    let prefix = if message.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    if prefix.is_empty() {
        eprintln!("{description}");
    } else {
        eprintln!("{prefix}: {description}");
    }
}
