//! `dispatch_source_*` for timer and user-data (`DATA_ADD`, `DATA_OR`,
//! `DATA_REPLACE`) sources.
//!
//! A source starts suspended. Its handlers run as ordinary queue items on the
//! source's target queue: registration once on first activation, then event
//! deliveries that coalesce while one is already queued, and finally the cancel
//! handler. Timers are driven by one short-lived host thread per configuration.
//! Descriptor, process, signal, vnode, Mach and memory-pressure sources need
//! kernel event sources that do not exist here; creating one returns NULL.
use super::*;

const EVENT: u8 = 0;
const CANCEL: u8 = 1;
const REGISTRATION: u8 = 2;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SourceKind {
    DataAdd,
    DataOr,
    DataReplace,
    Timer,
}

#[derive(Clone, Copy)]
enum Handler {
    Block(usize),
    Function(usize),
}

struct Timer {
    start: u64,
    interval: u64,
    epoch: u64,
}

pub(super) struct Source {
    kind: SourceKind,
    queue: usize,
    handle: usize,
    mask: usize,
    suspend: u32,
    activated: bool,
    cancelled: bool,
    event: Option<Handler>,
    cancel: Option<Handler>,
    registration: Option<Handler>,
    /// Handlers replaced while a delivery may still be running; released at free.
    retired: Vec<usize>,
    data: u64,
    delivering: u64,
    event_pending: bool,
    cancel_pending: bool,
    busy: usize,
    timer: Option<Timer>,
    epoch: u64,
}

fn source_mut(state: &mut State, address: usize) -> Option<&mut Source> {
    match &mut state.objects.get_mut(&address)?.kind {
        Kind::Source(source) => Some(source),
        _ => None,
    }
}

pub(super) fn is_source(state: &State, address: usize) -> bool {
    matches!(
        state.objects.get(&address).map(|o| &o.kind),
        Some(Kind::Source(_))
    )
}

fn kind_of(type_address: usize) -> Option<SourceKind> {
    Some(match type_address {
        a if a == cell(SOURCE_TYPE_BASE) => SourceKind::DataAdd,
        a if a == cell(SOURCE_TYPE_BASE + 1) => SourceKind::DataOr,
        a if a == cell(SOURCE_TYPE_BASE + 2) => SourceKind::DataReplace,
        a if a == cell(SOURCE_TYPE_BASE + 3) => SourceKind::Timer,
        _ => return None,
    })
}

extern "sysv64" fn dispatch_source_create(
    type_address: usize,
    handle: usize,
    mask: usize,
    queue_address: usize,
) -> usize {
    let Some(kind) = kind_of(type_address) else {
        return 0;
    };
    let queue_address = if queue_address == 0 {
        cell(3)
    } else {
        queue_address
    };
    let mut guard = state();
    if queue(&guard, queue_address).is_none() {
        return 0;
    }
    retain_object(&mut guard, queue_address);
    insert_object(
        &mut guard,
        Kind::Source(Source {
            kind,
            queue: queue_address,
            handle,
            mask,
            suspend: 1,
            activated: false,
            cancelled: false,
            event: None,
            cancel: None,
            registration: None,
            retired: Vec::new(),
            data: 0,
            delivering: 0,
            event_pending: false,
            cancel_pending: false,
            busy: 0,
            timer: None,
            epoch: 0,
        }),
    )
}

#[derive(Clone, Copy)]
enum Which {
    Event,
    Cancel,
    Registration,
}

fn set_handler(address: usize, which: Which, handler: Option<Handler>) {
    let mut guard = state();
    let Some(source) = source_mut(&mut guard, address) else {
        fatal("dispatch_source handler set on an invalid dispatch source");
    };
    let slot = match which {
        Which::Event => &mut source.event,
        Which::Cancel => &mut source.cancel,
        Which::Registration => &mut source.registration,
    };
    if let Some(Handler::Block(old)) = std::mem::replace(slot, handler) {
        source.retired.push(old);
    }
}

fn block_handler(block: *const c_void, operation: &str) -> Option<Handler> {
    if block.is_null() {
        return None;
    }
    match copy_block(block, operation) {
        Work::Block(copy) => Some(Handler::Block(copy)),
        _ => None,
    }
}

fn function_handler(function: usize, operation: &str) -> Option<Handler> {
    if function == 0 {
        return None;
    }
    check_function(function, operation);
    Some(Handler::Function(function))
}

macro_rules! handler_setters {
    ($block:ident, $function:ident, $which:expr, $name:literal) => {
        extern "sysv64" fn $block(address: usize, block: *const c_void) {
            set_handler(address, $which, block_handler(block, $name));
        }
        extern "sysv64" fn $function(address: usize, function: usize) {
            set_handler(
                address,
                $which,
                function_handler(function, concat!($name, "_f")),
            );
        }
    };
}

handler_setters!(
    dispatch_source_set_event_handler,
    dispatch_source_set_event_handler_f,
    Which::Event,
    "dispatch_source_set_event_handler"
);
handler_setters!(
    dispatch_source_set_cancel_handler,
    dispatch_source_set_cancel_handler_f,
    Which::Cancel,
    "dispatch_source_set_cancel_handler"
);
handler_setters!(
    dispatch_source_set_registration_handler,
    dispatch_source_set_registration_handler_f,
    Which::Registration,
    "dispatch_source_set_registration_handler"
);

fn schedule(state: &mut State, address: usize, what: u8) {
    let Some(source) = source_mut(state, address) else {
        return;
    };
    let queue = source.queue;
    push_locked(
        state,
        queue,
        Item {
            work: Work::Source(address, what),
            barrier: false,
            group: None,
            slot: None,
        },
    );
}

/// Queues an event delivery when data is pending and nothing blocks it.
fn trigger(state: &mut State, address: usize) {
    let Some(source) = source_mut(state, address) else {
        return;
    };
    if source.cancelled
        || source.suspend > 0
        || source.event_pending
        || source.event.is_none()
        || source.data == 0
    {
        return;
    }
    source.event_pending = true;
    schedule(state, address, EVENT);
}

fn finish_cancel_if_ready(state: &mut State, address: usize) {
    let Some(source) = source_mut(state, address) else {
        return;
    };
    if source.cancelled && source.suspend == 0 && !source.cancel_pending && source.activated {
        source.cancel_pending = true;
        if source.cancel.is_some() {
            schedule(state, address, CANCEL);
        }
    }
}

fn arm_timer(state: &mut State, address: usize) {
    let Some(source) = source_mut(state, address) else {
        return;
    };
    if source.kind != SourceKind::Timer || !source.activated || source.cancelled {
        return;
    }
    let Some(timer) = &source.timer else {
        return;
    };
    let (epoch, start, interval) = (timer.epoch, timer.start, timer.interval);
    let spawned = std::thread::Builder::new()
        .name("Adapter-dispatch-timer".into())
        .stack_size(64 * 1024)
        .spawn(move || timer_loop(address, epoch, start, interval));
    if spawned.is_err() {
        fatal("cannot create a dispatch source timer thread");
    }
}

fn timer_loop(address: usize, epoch: u64, start: u64, interval: u64) {
    let mut next = start;
    let mut guard = state();
    loop {
        if guard.shutdown {
            return;
        }
        let live = matches!(
            source_mut(&mut guard, address),
            Some(source) if source.epoch == epoch && !source.cancelled
        );
        if !live {
            return;
        }
        let current = now();
        if current < next {
            guard = CV
                .wait_timeout(guard, Duration::from_nanos(next - current))
                .unwrap_or_else(|e| e.into_inner())
                .0;
            continue;
        }
        let one_shot = interval == FOREVER || interval == 0;
        let fires = if one_shot {
            1
        } else {
            1 + (current - next) / interval
        };
        if let Some(source) = source_mut(&mut guard, address) {
            source.data = source.data.saturating_add(fires);
        }
        trigger(&mut guard, address);
        pump(&mut guard);
        CV.notify_all();
        if one_shot {
            return;
        }
        next = next.saturating_add(fires.saturating_mul(interval));
    }
}

extern "sysv64" fn dispatch_source_set_timer(
    address: usize,
    start: u64,
    interval: u64,
    _leeway: u64,
) {
    let start = monotonic_deadline(start);
    let mut guard = state();
    let Some(source) = source_mut(&mut guard, address) else {
        fatal("dispatch_source_set_timer on an invalid dispatch source");
    };
    if source.kind != SourceKind::Timer {
        return;
    }
    source.epoch += 1;
    let epoch = source.epoch;
    source.timer = (start != FOREVER).then_some(Timer {
        start,
        interval,
        epoch,
    });
    arm_timer(&mut guard, address);
}

extern "sysv64" fn dispatch_source_merge_data(address: usize, value: u64) {
    let mut guard = state();
    let Some(source) = source_mut(&mut guard, address) else {
        fatal("dispatch_source_merge_data on an invalid dispatch source");
    };
    match source.kind {
        SourceKind::DataAdd => source.data = source.data.saturating_add(value),
        SourceKind::DataOr => source.data |= value,
        SourceKind::DataReplace if value != 0 => source.data = value,
        _ => return,
    }
    trigger(&mut guard, address);
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_source_cancel(address: usize) {
    cancel_source(address);
}

pub(super) fn cancel_source(address: usize) {
    let mut guard = state();
    let Some(source) = source_mut(&mut guard, address) else {
        fatal("dispatch_source_cancel on an invalid dispatch source");
    };
    source.cancelled = true;
    source.epoch += 1;
    finish_cancel_if_ready(&mut guard, address);
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_source_testcancel(address: usize) -> i64 {
    source_cancelled(address)
}

pub(super) fn source_cancelled(address: usize) -> i64 {
    match source_mut(&mut state(), address) {
        Some(source) => i64::from(source.cancelled),
        None => fatal("dispatch_source_testcancel on an invalid dispatch source"),
    }
}

extern "sysv64" fn dispatch_source_get_data(address: usize) -> u64 {
    match source_mut(&mut state(), address) {
        Some(source) if source.busy > 0 => source.delivering,
        Some(source) => source.data,
        None => fatal("dispatch_source_get_data on an invalid dispatch source"),
    }
}

extern "sysv64" fn dispatch_source_get_handle(address: usize) -> usize {
    source_mut(&mut state(), address).map_or(0, |source| source.handle)
}

extern "sysv64" fn dispatch_source_get_mask(address: usize) -> usize {
    source_mut(&mut state(), address).map_or(0, |source| source.mask)
}

/// Suspension of a source; returns false when `address` is not a source.
pub(super) fn suspend(state: &mut State, address: usize) -> bool {
    match source_mut(state, address) {
        Some(source) => {
            source.suspend += 1;
            true
        }
        None => false,
    }
}

/// Resumption of a source; returns false when `address` is not a source.
pub(super) fn resume(state: &mut State, address: usize) -> bool {
    let Some(source) = source_mut(state, address) else {
        return false;
    };
    if source.suspend == 0 {
        fatal("over-resume of a dispatch source");
    }
    source.suspend -= 1;
    if source.suspend > 0 {
        return true;
    }
    if !source.activated {
        source.activated = true;
        if source.registration.is_some() {
            schedule(state, address, REGISTRATION);
        }
        arm_timer(state, address);
    }
    trigger(state, address);
    finish_cancel_if_ready(state, address);
    true
}

/// Executes one scheduled handler on the thread draining the source's queue.
pub(super) fn run(address: usize, what: u8) {
    let (handler, context) = {
        let mut guard = state();
        let context = guard.objects.get(&address).map_or(0, |o| o.context);
        let Some(source) = source_mut(&mut guard, address) else {
            return;
        };
        let handler = match what {
            EVENT => {
                source.event_pending = false;
                if source.cancelled {
                    return;
                }
                source.delivering = std::mem::take(&mut source.data);
                source.event
            }
            CANCEL => source.cancel,
            _ => source.registration,
        };
        let Some(handler) = handler else {
            return;
        };
        source.busy += 1;
        (handler, context)
    };
    unsafe {
        match handler {
            Handler::Block(block) => blocks::invoke_void(block as *const c_void),
            Handler::Function(function) => {
                let function: extern "sysv64" fn(usize) = std::mem::transmute(function);
                function(context);
            }
        }
    }
    let mut guard = state();
    if let Some(source) = source_mut(&mut guard, address) {
        source.busy -= 1;
    }
    trigger(&mut guard, address);
    maybe_free(&mut guard, address);
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

/// Whether a source with no references can be reclaimed now.
pub(super) fn reclaimable(source: &Source) -> bool {
    source.busy == 0
}

/// Everything a freed source still owns: handler blocks to release and the queue.
pub(super) fn dismantle(source: Source) -> (Vec<usize>, usize) {
    let mut blocks_owned = source.retired;
    for handler in [source.event, source.cancel, source.registration]
        .into_iter()
        .flatten()
    {
        if let Handler::Block(block) = handler {
            blocks_owned.push(block);
        }
    }
    (blocks_owned, source.queue)
}

pub(super) fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "__dispatch_source_type_data_add" => cell(SOURCE_TYPE_BASE) as u64,
        "__dispatch_source_type_data_or" => cell(SOURCE_TYPE_BASE + 1) as u64,
        "__dispatch_source_type_data_replace" => cell(SOURCE_TYPE_BASE + 2) as u64,
        "__dispatch_source_type_timer" => cell(SOURCE_TYPE_BASE + 3) as u64,
        "__dispatch_source_type_mach_send" => cell(SOURCE_TYPE_BASE + 4) as u64,
        "__dispatch_source_type_mach_recv" => cell(SOURCE_TYPE_BASE + 5) as u64,
        "__dispatch_source_type_memorypressure" => cell(SOURCE_TYPE_BASE + 6) as u64,
        "__dispatch_source_type_proc" => cell(SOURCE_TYPE_BASE + 7) as u64,
        "__dispatch_source_type_read" => cell(SOURCE_TYPE_BASE + 8) as u64,
        "__dispatch_source_type_signal" => cell(SOURCE_TYPE_BASE + 9) as u64,
        "__dispatch_source_type_vnode" => cell(SOURCE_TYPE_BASE + 10) as u64,
        "__dispatch_source_type_write" => cell(SOURCE_TYPE_BASE + 11) as u64,
        "_dispatch_source_create" => dispatch_source_create as *const () as u64,
        "_dispatch_source_set_event_handler" => {
            dispatch_source_set_event_handler as *const () as u64
        }
        "_dispatch_source_set_event_handler_f" => {
            dispatch_source_set_event_handler_f as *const () as u64
        }
        "_dispatch_source_set_cancel_handler" => {
            dispatch_source_set_cancel_handler as *const () as u64
        }
        "_dispatch_source_set_cancel_handler_f" => {
            dispatch_source_set_cancel_handler_f as *const () as u64
        }
        "_dispatch_source_set_registration_handler" => {
            dispatch_source_set_registration_handler as *const () as u64
        }
        "_dispatch_source_set_registration_handler_f" => {
            dispatch_source_set_registration_handler_f as *const () as u64
        }
        "_dispatch_source_set_timer" => dispatch_source_set_timer as *const () as u64,
        "_dispatch_source_merge_data" => dispatch_source_merge_data as *const () as u64,
        "_dispatch_source_cancel" => dispatch_source_cancel as *const () as u64,
        "_dispatch_source_testcancel" => dispatch_source_testcancel as *const () as u64,
        "_dispatch_source_get_data" => dispatch_source_get_data as *const () as u64,
        "_dispatch_source_get_handle" => dispatch_source_get_handle as *const () as u64,
        "_dispatch_source_get_mask" => dispatch_source_get_mask as *const () as u64,
        _ => return None,
    })
}
