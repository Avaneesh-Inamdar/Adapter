//! Grand Central Dispatch queues, groups, once, after and apply over host threads.
//!
//! Every queue is a record in one registry guarded by a single lock; admission
//! rules (serial exclusion, barriers, suspension, target chains) are evaluated
//! by `pump`, which starts queued items on short-lived worker threads. Each
//! worker is set up like a guest `pthread` (thread state, TLV teardown) so
//! blocks may use errno, keys and thread-locals. A `dispatch_sync` block runs on
//! the calling thread once the queue admits it. The main queue is only drained
//! by `dispatch_main`. Sources, `dispatch_data`, I/O channels, workloops and
//! QoS-driven scheduling are not modelled.
mod block_objects;
mod sources;

use crate::Result;
use crate::darwin::{blocks, callbacks, libc, thread_state, tlv};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    ffi::{CStr, c_void},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicIsize, AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};

/// Same non-zero value the semaphore adapter reports for an expired deadline.
const TIMED_OUT: i64 = 60;
const FOREVER: u64 = u64::MAX;
const MAX_WORKERS: usize = 64;
const WORKER_STACK: usize = 4 << 20;

#[repr(align(16))]
struct Cell {
    _bytes: [u8; 128],
}

/// Identity cells for the main queue, the five root queues and the queue
/// attributes. One array keeps every address distinct.
static CELLS: [Cell; 21] = [const { Cell { _bytes: [0; 128] } }; 21];
const MAIN: usize = 0;
const ATTR_CONCURRENT: usize = 6;
const ATTR_SERIAL_INACTIVE: usize = 7;
const ATTR_CONCURRENT_INACTIVE: usize = 8;
/// First of the twelve `dispatch_source_type_t` identity cells.
const SOURCE_TYPE_BASE: usize = 9;
const ROOT_QOS: [u32; 5] = [0x21, 0x19, 0x15, 0x11, 0x09];
const ROOT_LABELS: [&str; 5] = [
    "com.apple.root.user-interactive-qos",
    "com.apple.root.user-initiated-qos",
    "com.apple.root.default-qos",
    "com.apple.root.utility-qos",
    "com.apple.root.background-qos",
];

fn cell(index: usize) -> usize {
    std::ptr::addr_of!(CELLS[index]) as usize
}

#[derive(Clone, Copy)]
enum Work {
    None,
    /// An owned `Block_copy` that is released after it runs.
    Block(usize),
    /// A block owned by a waiting caller.
    Borrowed(usize),
    Func(usize, usize),
    /// A source handler: `(source, 0 event | 1 cancel | 2 registration)`.
    Source(usize, u8),
}

#[derive(Default)]
struct SlotState {
    granted: bool,
    done: bool,
    chain: Vec<usize>,
}
type Slot = Arc<Mutex<SlotState>>;

struct Item {
    work: Work,
    barrier: bool,
    group: Option<usize>,
    slot: Option<Slot>,
}

struct Queue {
    label: Option<Box<[u8]>>,
    serial: bool,
    main: bool,
    global: bool,
    items: VecDeque<Item>,
    running: usize,
    barrier_running: bool,
    suspend: u32,
    target: Option<usize>,
    inactive: bool,
    qos: u32,
    /// `(key, context, destructor)` from `dispatch_queue_set_specific`.
    specifics: Vec<(usize, usize, usize)>,
}

#[derive(Default)]
struct Group {
    outstanding: usize,
    notifies: Vec<(usize, Work)>,
}

enum Kind {
    Queue(Queue),
    Group(Group),
    Source(sources::Source),
}

struct Obj {
    refs: usize,
    immortal: bool,
    _storage: Option<Box<[u8; 64]>>,
    context: usize,
    finalizer: usize,
    kind: Kind,
}

struct State {
    objects: BTreeMap<usize, Obj>,
    workers: usize,
    shutdown: bool,
    ready: bool,
    blocks: BTreeMap<usize, block_objects::BlockState>,
}

static STATE: Mutex<State> = Mutex::new(State {
    objects: BTreeMap::new(),
    workers: 0,
    shutdown: false,
    ready: false,
    blocks: BTreeMap::new(),
});
static CV: Condvar = Condvar::new();
static ONCE_LOCK: Mutex<()> = Mutex::new(());
static ONCE_CV: Condvar = Condvar::new();
static MAIN_THREAD: Mutex<Option<ThreadId>> = Mutex::new(None);

thread_local! {
    static CURRENT: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn fatal(message: &str) -> ! {
    eprintln!("adapter-run [dispatch]: {message}");
    // libdispatch traps on client bugs, which is SIGILL on x86-64.
    std::process::exit(128 + 4)
}

fn new_queue(label: Option<Box<[u8]>>, serial: bool, main: bool, global: bool) -> Queue {
    Queue {
        label,
        serial,
        main,
        global,
        items: VecDeque::new(),
        running: 0,
        barrier_running: false,
        suspend: 0,
        target: None,
        inactive: false,
        qos: 0,
        specifics: Vec::new(),
    }
}

fn label_bytes(text: &str) -> Box<[u8]> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes.into_boxed_slice()
}

fn init(state: &mut State) {
    state.ready = true;
    let mut add = |address: usize, queue: Queue| {
        state.objects.insert(
            address,
            Obj {
                refs: 1,
                immortal: true,
                _storage: None,
                context: 0,
                finalizer: 0,
                kind: Kind::Queue(queue),
            },
        );
    };
    add(
        cell(MAIN),
        new_queue(
            Some(label_bytes("com.apple.main-thread")),
            true,
            true,
            false,
        ),
    );
    for (index, label) in ROOT_LABELS.iter().enumerate() {
        let mut root = new_queue(Some(label_bytes(label)), false, false, true);
        root.qos = ROOT_QOS[index];
        add(cell(1 + index), root);
    }
}

fn state() -> MutexGuard<'static, State> {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if !guard.ready {
        init(&mut guard);
    }
    guard
}

fn wait(guard: MutexGuard<'static, State>) -> MutexGuard<'static, State> {
    CV.wait(guard).unwrap_or_else(|e| e.into_inner())
}

fn queue(state: &State, address: usize) -> Option<&Queue> {
    match &state.objects.get(&address)?.kind {
        Kind::Queue(queue) => Some(queue),
        _ => None,
    }
}

fn queue_mut(state: &mut State, address: usize) -> Option<&mut Queue> {
    match &mut state.objects.get_mut(&address)?.kind {
        Kind::Queue(queue) => Some(queue),
        _ => None,
    }
}

fn group_mut(state: &mut State, address: usize) -> Option<&mut Group> {
    match &mut state.objects.get_mut(&address)?.kind {
        Kind::Group(group) => Some(group),
        _ => None,
    }
}

fn insert_object(state: &mut State, kind: Kind) -> usize {
    let storage = Box::new([0u8; 64]);
    let address = storage.as_ptr() as usize;
    state.objects.insert(
        address,
        Obj {
            refs: 1,
            immortal: false,
            _storage: Some(storage),
            context: 0,
            finalizer: 0,
            kind,
        },
    );
    address
}

fn chain(state: &State, start: usize) -> Vec<usize> {
    let mut links = vec![start];
    let mut current = start;
    while let Some(next) = queue(state, current).and_then(|q| q.target) {
        if links.contains(&next) {
            break;
        }
        links.push(next);
        current = next;
    }
    links
}

/// Returns the queue chain and barrier flag when the head item may start now.
fn admissible(state: &State, address: usize) -> Option<(Vec<usize>, bool)> {
    let head = queue(state, address)?;
    if head.main {
        return None;
    }
    let item = head.items.front()?;
    let links = chain(state, address);
    for (index, link) in links.iter().enumerate() {
        let q = queue(state, *link)?;
        if q.suspend > 0 || q.main {
            return None;
        }
        let exclusive = !q.global && (q.serial || (index == 0 && item.barrier));
        if exclusive {
            if q.running > 0 {
                return None;
            }
        } else if index == 0 && !q.global && q.barrier_running {
            return None;
        }
    }
    if item.slot.is_none() && state.workers >= MAX_WORKERS {
        return None;
    }
    Some((links, item.barrier))
}

fn pump(state: &mut State) {
    let queues: Vec<usize> = state
        .objects
        .iter()
        .filter(|(_, object)| matches!(object.kind, Kind::Queue(_)))
        .map(|(address, _)| *address)
        .collect();
    for address in queues {
        while let Some((links, barrier)) = admissible(state, address) {
            start(state, address, links, barrier);
        }
    }
}

fn start(state: &mut State, address: usize, links: Vec<usize>, barrier: bool) {
    let item = match queue_mut(state, address).and_then(|q| q.items.pop_front()) {
        Some(item) => item,
        None => return,
    };
    for link in &links {
        if let Some(q) = queue_mut(state, *link) {
            q.running += 1;
        }
    }
    if barrier && let Some(q) = queue_mut(state, address) {
        q.barrier_running = true;
    }
    match &item.slot {
        Some(slot) => {
            let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
            slot.granted = true;
            slot.chain = links;
        }
        None => {
            state.workers += 1;
            spawn_worker(address, links, barrier, item);
        }
    }
}

fn guest_thread(body: impl FnOnce()) {
    thread_state::reset_thread();
    body();
    if let Err(error) = thread_state::finish_thread() {
        eprintln!("adapter-run [thread cleanup]: {error}");
        std::process::exit(125);
    }
    tlv::clear_thread();
}

fn spawn_worker(address: usize, links: Vec<usize>, barrier: bool, item: Item) {
    let spawned = std::thread::Builder::new()
        .name("Adapter-dispatch".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            guest_thread(|| run_work(address, item.work));
            let mut guard = state();
            guard.workers -= 1;
            complete(&mut guard, &links, barrier, item.group);
            pump(&mut guard);
            drop(guard);
            CV.notify_all();
        });
    if spawned.is_err() {
        fatal("cannot create a dispatch worker thread");
    }
}

fn run_work(address: usize, work: Work) {
    CURRENT.with(|stack| stack.borrow_mut().push(address));
    unsafe {
        match work {
            Work::None => {}
            Work::Block(block) => {
                blocks::invoke_void(block as *const c_void);
                blocks::release(block as *mut c_void);
            }
            Work::Borrowed(block) => blocks::invoke_void(block as *const c_void),
            Work::Source(source, what) => sources::run(source, what),
            Work::Func(function, context) => {
                let function: extern "sysv64" fn(usize) = std::mem::transmute(function);
                function(context);
            }
        }
    }
    CURRENT.with(|stack| {
        stack.borrow_mut().pop();
    });
}

/// Book-keeping after an item finished running on `links[0]`.
fn complete(state: &mut State, links: &[usize], barrier: bool, group: Option<usize>) {
    for link in links {
        if let Some(q) = queue_mut(state, *link) {
            q.running = q.running.saturating_sub(1);
        }
    }
    if barrier && let Some(q) = links.first().and_then(|a| queue_mut(state, *a)) {
        q.barrier_running = false;
    }
    if let Some(group) = group {
        leave_group(state, group);
    }
    for link in links {
        maybe_free(state, *link);
    }
}

fn maybe_free(state: &mut State, address: usize) {
    let removable = match state.objects.get(&address) {
        Some(object) if !object.immortal && object.refs == 0 => match &object.kind {
            Kind::Queue(q) => q.items.is_empty() && q.running == 0,
            Kind::Group(g) => g.outstanding == 0 && g.notifies.is_empty(),
            Kind::Source(s) => sources::reclaimable(s),
        },
        _ => false,
    };
    if !removable {
        return;
    }
    let Some(object) = state.objects.remove(&address) else {
        return;
    };
    // Guest destructors never run under the registry lock.
    let mut calls = Vec::new();
    if object.finalizer != 0 {
        calls.push((object.finalizer, object.context));
    }
    let mut target = None;
    let mut owned_blocks = Vec::new();
    if let Kind::Queue(q) = &object.kind {
        target = q.target;
        for (_, context, destructor) in &q.specifics {
            if *destructor != 0 && *context != 0 {
                calls.push((*destructor, *context));
            }
        }
    }
    if let Kind::Source(source) = object.kind {
        let (handlers, source_queue) = sources::dismantle(source);
        owned_blocks = handlers;
        target = Some(source_queue);
    }
    for block in owned_blocks {
        calls.push((release_block as *const () as usize, block));
    }
    run_detached(state, calls);
    if let Some(target) = target {
        release_object(state, target);
    }
}

extern "sysv64" fn release_block(block: usize) {
    unsafe { blocks::release(block as *mut c_void) };
}

/// Runs `(function, context)` pairs on a fresh guest thread.
fn run_detached(registry: &mut State, calls: Vec<(usize, usize)>) {
    if calls.is_empty() {
        return;
    }
    registry.workers += 1;
    let spawned = std::thread::Builder::new()
        .name("Adapter-dispatch".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            guest_thread(|| {
                for (function, context) in calls {
                    let function: extern "sysv64" fn(usize) =
                        unsafe { std::mem::transmute(function) };
                    function(context);
                }
            });
            let mut guard = state();
            guard.workers -= 1;
            drop(guard);
            CV.notify_all();
        });
    if spawned.is_err() {
        fatal("cannot create a dispatch finalizer thread");
    }
}

fn retain_object(state: &mut State, address: usize) {
    if let Some(object) = state.objects.get_mut(&address) {
        object.refs += 1;
    }
}

fn release_object(state: &mut State, address: usize) {
    match state.objects.get_mut(&address) {
        Some(object) if object.immortal => {}
        Some(object) if object.refs == 0 => fatal("over-release of a dispatch object"),
        Some(object) => {
            object.refs -= 1;
            maybe_free(state, address);
        }
        None => {}
    }
}

fn leave_group(state: &mut State, address: usize) {
    let ready = match group_mut(state, address) {
        Some(group) if group.outstanding == 0 => {
            fatal("unbalanced dispatch_group_leave: the group is not entered")
        }
        Some(group) => {
            group.outstanding -= 1;
            if group.outstanding == 0 {
                std::mem::take(&mut group.notifies)
            } else {
                Vec::new()
            }
        }
        None => return,
    };
    for (target, work) in ready {
        push_locked(
            state,
            target,
            Item {
                work,
                barrier: false,
                group: None,
                slot: None,
            },
        );
        release_object(state, target);
    }
    maybe_free(state, address);
}

fn push_locked(state: &mut State, address: usize, mut item: Item) {
    if state.shutdown {
        return;
    }
    let group = item.group;
    let Some(q) = queue_mut(state, address) else {
        fatal("operation on an invalid or released dispatch queue");
    };
    if q.global {
        item.barrier = false;
    }
    q.items.push_back(item);
    if let Some(group) = group
        && let Some(group) = group_mut(state, group)
    {
        group.outstanding += 1;
    }
}

fn push(address: usize, item: Item) {
    let mut guard = state();
    push_locked(&mut guard, address, item);
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

fn check_block(block: *const c_void, operation: &str) {
    let invoke = unsafe { blocks::invoke_address(block) };
    if !callbacks::executable(invoke as u64) && invoke != block_objects::invoke_address() {
        fatal(&format!("{operation} received a NULL or invalid block"));
    }
}

fn check_function(function: usize, operation: &str) {
    if !callbacks::executable(function as u64) {
        fatal(&format!("{operation} received a NULL or invalid function"));
    }
}

fn copy_block(block: *const c_void, operation: &str) -> Work {
    check_block(block, operation);
    let copy = unsafe { blocks::copy(block) } as usize;
    if copy == 0 {
        fatal(&format!("{operation} could not copy its block"));
    }
    Work::Block(copy)
}

fn func_work(function: usize, context: usize, operation: &str) -> Work {
    check_function(function, operation);
    Work::Func(function, context)
}

fn enqueue(address: usize, work: Work, barrier: bool, group: Option<usize>) {
    push(
        address,
        Item {
            work,
            barrier,
            group,
            slot: None,
        },
    );
}

/// Runs `work` once `address` admits it, on the calling thread (or, for the main
/// queue, on the thread draining it).
fn run_sync(address: usize, work: Work, barrier: bool, operation: &str) {
    let slot: Slot = Arc::new(Mutex::new(SlotState::default()));
    let mut guard = state();
    if guard.shutdown {
        return;
    }
    let (main, serial) = match queue(&guard, address) {
        Some(q) => (q.main, q.serial),
        None => fatal(&format!("{operation} on an invalid dispatch queue")),
    };
    if (serial || barrier) && CURRENT.with(|stack| stack.borrow().contains(&address)) {
        fatal(&format!(
            "{operation} called on a queue already running on this thread (deadlock)"
        ));
    }
    let item = Item {
        work: if main { work } else { Work::None },
        barrier,
        group: None,
        slot: Some(slot.clone()),
    };
    push_locked(&mut guard, address, item);
    pump(&mut guard);
    CV.notify_all();
    if main {
        while !slot.lock().unwrap_or_else(|e| e.into_inner()).done {
            guard = wait(guard);
        }
        return;
    }
    while !slot.lock().unwrap_or_else(|e| e.into_inner()).granted {
        guard = wait(guard);
    }
    let links = slot.lock().unwrap_or_else(|e| e.into_inner()).chain.clone();
    drop(guard);
    run_work(address, work);
    let mut guard = state();
    complete(&mut guard, &links, barrier, None);
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

fn now() -> u64 {
    thread_local! {
        static TIME: Option<extern "sysv64" fn(u64, i64) -> u64> =
            libc::symbol("_dispatch_time").map(|f| unsafe { std::mem::transmute(f) });
    }
    TIME.with(|time| time.map_or(0, |time| time(0, 0)))
}

fn wall_nanoseconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

/// `dispatch_walltime` values are the negated nanoseconds since the epoch, so
/// they have the top bit set; convert one to the monotonic clock `now` reads.
fn monotonic_deadline(deadline: u64) -> u64 {
    if deadline == FOREVER || deadline >> 63 == 0 {
        return deadline;
    }
    let wall = (deadline as i64).wrapping_neg();
    let remaining = wall.saturating_sub(wall_nanoseconds()).max(0) as u64;
    now().saturating_add(remaining)
}

fn after(when: u64, address: usize, work: Work) {
    let when = monotonic_deadline(when);
    if when == FOREVER {
        return;
    }
    {
        let mut guard = state();
        if queue(&guard, address).is_none() {
            fatal("dispatch_after on an invalid dispatch queue");
        }
        retain_object(&mut guard, address);
    }
    let spawned = std::thread::Builder::new()
        .name("Adapter-dispatch-timer".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            let mut guard = state();
            loop {
                if guard.shutdown {
                    return;
                }
                let current = now();
                if current >= when {
                    break;
                }
                guard = CV
                    .wait_timeout(guard, Duration::from_nanos(when - current))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
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
            release_object(&mut guard, address);
            pump(&mut guard);
            drop(guard);
            CV.notify_all();
        });
    if spawned.is_err() {
        fatal("cannot create a dispatch timer thread");
    }
}

fn root_queue(identifier: isize) -> Option<usize> {
    let index = match identifier {
        0x21 => 0,
        2 | 0x19 => 1,
        0 | 0x15 => 2,
        -2 | 0x11 => 3,
        -32768 | 0x09 => 4,
        _ => return None,
    };
    Some(cell(1 + index))
}

extern "sysv64" fn dispatch_get_global_queue(identifier: isize, flags: usize) -> usize {
    if flags != 0 {
        return 0;
    }
    root_queue(identifier).unwrap_or(0)
}

fn create_queue(label: *const i8, attr: usize, target: usize) -> usize {
    let label = (!label.is_null()).then(|| {
        let mut bytes = unsafe { CStr::from_ptr(label) }.to_bytes().to_vec();
        bytes.push(0);
        bytes.into_boxed_slice()
    });
    let mut guard = state();
    let concurrent = attr == cell(ATTR_CONCURRENT) || attr == cell(ATTR_CONCURRENT_INACTIVE);
    let inactive = attr == cell(ATTR_SERIAL_INACTIVE) || attr == cell(ATTR_CONCURRENT_INACTIVE);
    let mut created = new_queue(label, !concurrent, false, false);
    if inactive {
        created.inactive = true;
        created.suspend = 1;
    }
    let address = insert_object(&mut guard, Kind::Queue(created));
    drop(guard);
    if target != 0 {
        set_target(address, target);
    }
    address
}

extern "sysv64" fn dispatch_queue_create(label: *const i8, attr: usize) -> usize {
    create_queue(label, attr, 0)
}

extern "sysv64" fn dispatch_queue_create_with_target(
    label: *const i8,
    attr: usize,
    target: usize,
) -> usize {
    create_queue(label, attr, target)
}

extern "sysv64" fn dispatch_queue_attr_make_with_qos_class(
    attr: usize,
    _class: u32,
    _relative: i32,
) -> usize {
    attr
}

extern "sysv64" fn dispatch_queue_attr_make_with_autorelease_frequency(
    attr: usize,
    _frequency: usize,
) -> usize {
    attr
}

extern "sysv64" fn dispatch_queue_attr_make_initially_inactive(attr: usize) -> usize {
    if attr == cell(ATTR_CONCURRENT) || attr == cell(ATTR_CONCURRENT_INACTIVE) {
        cell(ATTR_CONCURRENT_INACTIVE)
    } else {
        cell(ATTR_SERIAL_INACTIVE)
    }
}

extern "sysv64" fn dispatch_activate(address: usize) {
    let mut guard = state();
    if sources::resume(&mut guard, address) {
        pump(&mut guard);
        drop(guard);
        CV.notify_all();
        return;
    }
    if let Some(q) = queue_mut(&mut guard, address)
        && q.inactive
    {
        q.inactive = false;
        q.suspend = q.suspend.saturating_sub(1);
    }
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_queue_get_qos_class(address: usize, relative: *mut i32) -> u32 {
    if !relative.is_null() {
        unsafe { *relative = 0 };
    }
    queue(&state(), address).map_or(0, |q| q.qos)
}

extern "sysv64" fn dispatch_set_context(address: usize, context: usize) {
    if let Some(object) = state().objects.get_mut(&address) {
        object.context = context;
    }
}

extern "sysv64" fn dispatch_get_context(address: usize) -> usize {
    state().objects.get(&address).map_or(0, |o| o.context)
}

extern "sysv64" fn dispatch_set_finalizer_f(address: usize, finalizer: usize) {
    if finalizer != 0 {
        check_function(finalizer, "dispatch_set_finalizer_f");
    }
    if let Some(object) = state().objects.get_mut(&address) {
        object.finalizer = finalizer;
    }
}

extern "sysv64" fn dispatch_queue_set_specific(
    address: usize,
    key: usize,
    context: usize,
    destructor: usize,
) {
    if destructor != 0 {
        check_function(destructor, "dispatch_queue_set_specific");
    }
    let mut guard = state();
    let Some(q) = queue_mut(&mut guard, address) else {
        fatal("dispatch_queue_set_specific on an invalid dispatch queue");
    };
    let mut calls = Vec::new();
    if let Some(index) = q.specifics.iter().position(|s| s.0 == key) {
        let (_, old, old_destructor) = q.specifics.remove(index);
        if old_destructor != 0 && old != 0 {
            calls.push((old_destructor, old));
        }
    }
    if context != 0 {
        q.specifics.push((key, context, destructor));
    }
    run_detached(&mut guard, calls);
}

extern "sysv64" fn dispatch_queue_get_specific(address: usize, key: usize) -> usize {
    queue(&state(), address)
        .and_then(|q| q.specifics.iter().find(|s| s.0 == key))
        .map_or(0, |s| s.1)
}

/// Looks `key` up on the queue the caller is running on, then its targets.
extern "sysv64" fn dispatch_get_specific(key: usize) -> usize {
    let running = CURRENT.with(|stack| stack.borrow().clone());
    let guard = state();
    for address in running.iter().rev() {
        for link in chain(&guard, *address) {
            if let Some(found) =
                queue(&guard, link).and_then(|q| q.specifics.iter().find(|s| s.0 == key))
            {
                return found.1;
            }
        }
    }
    0
}

fn set_target(object: usize, target: usize) {
    let mut guard = state();
    // The main queue as a target and the root queues carry no extra
    // serialisation here, so only user queues are recorded.
    let usable = target != 0
        && queue(&guard, target).is_some_and(|q| !q.main && !q.global)
        && !chain(&guard, target).contains(&object);
    if queue(&guard, object).is_none_or(|q| q.main || q.global) {
        return;
    }
    let old = queue_mut(&mut guard, object).and_then(|q| q.target.take());
    if usable {
        retain_object(&mut guard, target);
        if let Some(q) = queue_mut(&mut guard, object) {
            q.target = Some(target);
        }
    }
    if let Some(old) = old {
        release_object(&mut guard, old);
    }
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_set_target_queue(object: usize, target: usize) {
    set_target(object, target);
}

extern "sysv64" fn dispatch_queue_get_label(address: usize) -> *const u8 {
    static EMPTY: [u8; 1] = [0];
    let address = if address == 0 {
        CURRENT.with(|stack| stack.borrow().last().copied().unwrap_or(cell(MAIN)))
    } else {
        address
    };
    let guard = state();
    match queue(&guard, address) {
        Some(q) => q.label.as_ref().map_or(EMPTY.as_ptr(), |l| l.as_ptr()),
        None => EMPTY.as_ptr(),
    }
}

extern "sysv64" fn dispatch_get_current_queue() -> usize {
    if let Some(current) = CURRENT.with(|stack| stack.borrow().last().copied()) {
        return current;
    }
    let on_main = MAIN_THREAD
        .lock()
        .map(|id| *id == Some(std::thread::current().id()))
        .unwrap_or(false);
    if on_main { cell(MAIN) } else { cell(3) }
}

extern "sysv64" fn dispatch_async(address: usize, block: *const c_void) {
    enqueue(address, copy_block(block, "dispatch_async"), false, None);
}

extern "sysv64" fn dispatch_async_f(address: usize, context: usize, function: usize) {
    enqueue(
        address,
        func_work(function, context, "dispatch_async_f"),
        false,
        None,
    );
}

extern "sysv64" fn dispatch_barrier_async(address: usize, block: *const c_void) {
    enqueue(
        address,
        copy_block(block, "dispatch_barrier_async"),
        true,
        None,
    );
}

extern "sysv64" fn dispatch_barrier_async_f(address: usize, context: usize, function: usize) {
    enqueue(
        address,
        func_work(function, context, "dispatch_barrier_async_f"),
        true,
        None,
    );
}

extern "sysv64" fn dispatch_sync(address: usize, block: *const c_void) {
    check_block(block, "dispatch_sync");
    run_sync(
        address,
        Work::Borrowed(block as usize),
        false,
        "dispatch_sync",
    );
}

extern "sysv64" fn dispatch_sync_f(address: usize, context: usize, function: usize) {
    let work = func_work(function, context, "dispatch_sync_f");
    run_sync(address, work, false, "dispatch_sync_f");
}

extern "sysv64" fn dispatch_barrier_sync(address: usize, block: *const c_void) {
    check_block(block, "dispatch_barrier_sync");
    run_sync(
        address,
        Work::Borrowed(block as usize),
        true,
        "dispatch_barrier_sync",
    );
}

extern "sysv64" fn dispatch_barrier_sync_f(address: usize, context: usize, function: usize) {
    let work = func_work(function, context, "dispatch_barrier_sync_f");
    run_sync(address, work, true, "dispatch_barrier_sync_f");
}

extern "sysv64" fn dispatch_async_and_wait(address: usize, block: *const c_void) {
    check_block(block, "dispatch_async_and_wait");
    let work = Work::Borrowed(block as usize);
    run_sync(address, work, false, "dispatch_async_and_wait");
}

extern "sysv64" fn dispatch_async_and_wait_f(address: usize, context: usize, function: usize) {
    let work = func_work(function, context, "dispatch_async_and_wait_f");
    run_sync(address, work, false, "dispatch_async_and_wait_f");
}

extern "sysv64" fn dispatch_barrier_async_and_wait(address: usize, block: *const c_void) {
    check_block(block, "dispatch_barrier_async_and_wait");
    let work = Work::Borrowed(block as usize);
    run_sync(address, work, true, "dispatch_barrier_async_and_wait");
}

extern "sysv64" fn dispatch_barrier_async_and_wait_f(
    address: usize,
    context: usize,
    function: usize,
) {
    let work = func_work(function, context, "dispatch_barrier_async_and_wait_f");
    run_sync(address, work, true, "dispatch_barrier_async_and_wait_f");
}

/// `struct timespec` on Darwin x86-64.
#[repr(C)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

extern "sysv64" fn dispatch_walltime(when: *const Timespec, delta: i64) -> u64 {
    let base = if when.is_null() {
        wall_nanoseconds()
    } else {
        let value = unsafe { std::ptr::read_unaligned(when) };
        value
            .seconds
            .saturating_mul(1_000_000_000)
            .saturating_add(value.nanoseconds)
    };
    let target = base.saturating_add(delta);
    if target <= 0 || target == i64::MAX {
        return FOREVER;
    }
    // Wall-clock deadlines are negated so they cannot collide with monotonic ones.
    target.wrapping_neg() as u64
}

/// `dispatch_wait` on a group; semaphores belong to the host library adapter.
extern "sysv64" fn dispatch_wait(address: usize, timeout: u64) -> i64 {
    if state().blocks.contains_key(&address) {
        return block_objects::wait_for_block(address, timeout);
    }
    let is_group = matches!(
        state().objects.get(&address).map(|o| &o.kind),
        Some(Kind::Group(_))
    );
    if is_group {
        return dispatch_group_wait(address, timeout);
    }
    if let Some(function) = libc::symbol("_dispatch_semaphore_wait") {
        let function: extern "sysv64" fn(usize, u64) -> i64 =
            unsafe { std::mem::transmute(function) };
        return function(address, timeout);
    }
    fatal("dispatch_wait on an unsupported dispatch object")
}

extern "sysv64" fn dispatch_notify(address: usize, queue: usize, block: *const c_void) {
    if state().blocks.contains_key(&address) {
        block_objects::notify(address, queue, block);
        return;
    }
    group_notify(address, queue, copy_block(block, "dispatch_notify"));
}

/// `dispatch_cancel` for blocks made by `dispatch_block_create` and for sources.
extern "sysv64" fn dispatch_cancel(address: usize) {
    if block_objects::cancel(address) {
        return;
    }
    if sources::is_source(&state(), address) {
        sources::cancel_source(address);
        return;
    }
    fatal("dispatch_cancel on an object that cannot be cancelled");
}

extern "sysv64" fn dispatch_testcancel(address: usize) -> i64 {
    if let Some(value) = block_objects::cancelled(address) {
        return i64::from(value);
    }
    if sources::is_source(&state(), address) {
        return sources::source_cancelled(address);
    }
    fatal("dispatch_testcancel on an object that cannot be cancelled")
}

/// Signal sources do not exist, so there is nothing to allow; the call succeeds.
extern "sysv64" fn dispatch_allow_send_signals(_signal: i32) -> i32 {
    0
}

extern "sysv64" fn dispatch_noop(_object: usize, _argument: usize) {}

extern "sysv64" fn dispatch_after(when: u64, address: usize, block: *const c_void) {
    after(when, address, copy_block(block, "dispatch_after"));
}

extern "sysv64" fn dispatch_after_f(when: u64, address: usize, context: usize, function: usize) {
    after(
        when,
        address,
        func_work(function, context, "dispatch_after_f"),
    );
}

fn apply(count: usize, address: usize, call: impl Fn(usize) + Sync) {
    if count == 0 {
        return;
    }
    let concurrent = {
        let guard = state();
        match queue(&guard, address) {
            Some(q) => !q.serial && !q.main,
            None => fatal("dispatch_apply on an invalid dispatch queue"),
        }
    };
    let next = AtomicUsize::new(0);
    let body = || {
        CURRENT.with(|stack| stack.borrow_mut().push(address));
        loop {
            let index = next.fetch_add(1, Ordering::SeqCst);
            if index >= count {
                break;
            }
            call(index);
        }
        CURRENT.with(|stack| {
            stack.borrow_mut().pop();
        });
    };
    let threads = if concurrent {
        count.min(
            std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(2, 8),
        )
    } else {
        1
    };
    std::thread::scope(|scope| {
        for _ in 1..threads {
            let spawned = std::thread::Builder::new()
                .name("Adapter-dispatch".into())
                .stack_size(WORKER_STACK)
                .spawn_scoped(scope, || guest_thread(body));
            if spawned.is_err() {
                fatal("cannot create a dispatch_apply worker thread");
            }
        }
        body();
    });
}

extern "sysv64" fn dispatch_apply(count: usize, address: usize, block: *const c_void) {
    check_block(block, "dispatch_apply");
    let block = block as usize;
    apply(count, address, |index| unsafe {
        blocks::invoke_index(block as *const c_void, index)
    });
}

extern "sysv64" fn dispatch_apply_f(count: usize, address: usize, context: usize, function: usize) {
    check_function(function, "dispatch_apply_f");
    apply(count, address, |index| unsafe {
        let function: extern "sysv64" fn(usize, usize) = std::mem::transmute(function);
        function(context, index);
    });
}

fn once(predicate: *mut isize, run: impl FnOnce()) {
    if predicate.is_null() {
        fatal("dispatch_once received a NULL predicate");
    }
    let flag = unsafe { AtomicIsize::from_ptr(predicate) };
    if flag.load(Ordering::Acquire) == -1 {
        return;
    }
    let mut guard = ONCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match flag.load(Ordering::Acquire) {
        -1 => {}
        0 => {
            flag.store(1, Ordering::Release);
            drop(guard);
            run();
            let _guard = ONCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            flag.store(-1, Ordering::Release);
            ONCE_CV.notify_all();
        }
        _ => {
            while flag.load(Ordering::Acquire) != -1 {
                guard = ONCE_CV.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

extern "sysv64" fn dispatch_once(predicate: *mut isize, block: *const c_void) {
    check_block(block, "dispatch_once");
    once(predicate, || unsafe { blocks::invoke_void(block) });
}

extern "sysv64" fn dispatch_once_f(predicate: *mut isize, context: usize, function: usize) {
    check_function(function, "dispatch_once_f");
    once(predicate, || unsafe {
        let function: extern "sysv64" fn(usize) = std::mem::transmute(function);
        function(context);
    });
}

extern "sysv64" fn dispatch_group_create() -> usize {
    insert_object(&mut state(), Kind::Group(Group::default()))
}

extern "sysv64" fn dispatch_group_enter(address: usize) {
    match group_mut(&mut state(), address) {
        Some(group) => group.outstanding += 1,
        None => fatal("dispatch_group_enter on an invalid dispatch group"),
    }
}

extern "sysv64" fn dispatch_group_leave(address: usize) {
    let mut guard = state();
    if group_mut(&mut guard, address).is_none() {
        fatal("dispatch_group_leave on an invalid dispatch group");
    }
    leave_group(&mut guard, address);
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_group_wait(address: usize, timeout: u64) -> i64 {
    let timeout = monotonic_deadline(timeout);
    let mut guard = state();
    loop {
        match group_mut(&mut guard, address) {
            Some(group) if group.outstanding == 0 => return 0,
            Some(_) => {}
            None => fatal("dispatch_group_wait on an invalid dispatch group"),
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

fn group_notify(group: usize, address: usize, work: Work) {
    let mut guard = state();
    let Some(outstanding) = group_mut(&mut guard, group).map(|g| g.outstanding) else {
        fatal("dispatch_group_notify on an invalid dispatch group");
    };
    if outstanding == 0 {
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
        if queue(&guard, address).is_none() {
            fatal("dispatch_group_notify on an invalid dispatch queue");
        }
        retain_object(&mut guard, address);
        if let Some(g) = group_mut(&mut guard, group) {
            g.notifies.push((address, work));
        }
    }
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_group_notify(group: usize, address: usize, block: *const c_void) {
    group_notify(group, address, copy_block(block, "dispatch_group_notify"));
}

extern "sysv64" fn dispatch_group_notify_f(
    group: usize,
    address: usize,
    context: usize,
    function: usize,
) {
    group_notify(
        group,
        address,
        func_work(function, context, "dispatch_group_notify_f"),
    );
}

extern "sysv64" fn dispatch_group_async(group: usize, address: usize, block: *const c_void) {
    enqueue(
        address,
        copy_block(block, "dispatch_group_async"),
        false,
        Some(group),
    );
}

extern "sysv64" fn dispatch_group_async_f(
    group: usize,
    address: usize,
    context: usize,
    function: usize,
) {
    enqueue(
        address,
        func_work(function, context, "dispatch_group_async_f"),
        false,
        Some(group),
    );
}

extern "sysv64" fn dispatch_suspend(address: usize) {
    let mut guard = state();
    if sources::suspend(&mut guard, address) {
        return;
    }
    if let Some(q) = queue_mut(&mut guard, address) {
        q.suspend += 1;
    }
}

extern "sysv64" fn dispatch_resume(address: usize) {
    let mut guard = state();
    if sources::resume(&mut guard, address) {
        pump(&mut guard);
        drop(guard);
        CV.notify_all();
        return;
    }
    if let Some(q) = queue_mut(&mut guard, address) {
        if q.inactive {
            fatal("dispatch_resume called on an inactive queue; use dispatch_activate");
        }
        if q.suspend == 0 {
            fatal("over-resume of a dispatch object");
        }
        q.suspend -= 1;
    }
    pump(&mut guard);
    drop(guard);
    CV.notify_all();
}

extern "sysv64" fn dispatch_retain(address: usize) {
    retain_object(&mut state(), address);
}

extern "sysv64" fn dispatch_release(address: usize) {
    let mut guard = state();
    if guard.objects.contains_key(&address) {
        release_object(&mut guard, address);
        pump(&mut guard);
        return;
    }
    drop(guard);
    // Anything else is a semaphore owned by the host library adapter.
    if let Some(release) = libc::symbol("_dispatch_release") {
        let release: extern "sysv64" fn(usize) = unsafe { std::mem::transmute(release) };
        release(address);
    }
}

extern "sysv64" fn dispatch_assert_queue(address: usize) {
    if !CURRENT.with(|stack| stack.borrow().contains(&address)) {
        fatal("dispatch_assert_queue: the current code is not running on the expected queue");
    }
}

extern "sysv64" fn dispatch_assert_queue_not(address: usize) {
    if CURRENT.with(|stack| stack.borrow().contains(&address)) {
        fatal("dispatch_assert_queue_not: the current code is running on the queue");
    }
}

fn take_main(state: &mut State) -> Option<Item> {
    let main = queue_mut(state, cell(MAIN))?;
    if main.suspend > 0 || main.running > 0 {
        return None;
    }
    let item = main.items.pop_front()?;
    main.running += 1;
    Some(item)
}

/// Serves the main queue on the calling thread and never returns.
extern "sysv64" fn dispatch_main() -> ! {
    let main = cell(MAIN);
    loop {
        let item = {
            let mut guard = state();
            loop {
                if let Some(item) = take_main(&mut guard) {
                    break item;
                }
                guard = wait(guard);
            }
        };
        run_work(main, item.work);
        let mut guard = state();
        complete(&mut guard, &[main], false, item.group);
        if let Some(slot) = &item.slot {
            slot.lock().unwrap_or_else(|e| e.into_inner()).done = true;
        }
        pump(&mut guard);
        drop(guard);
        CV.notify_all();
    }
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "__dispatch_main_q" => cell(MAIN) as u64,
        "__dispatch_queue_attr_concurrent" => cell(ATTR_CONCURRENT) as u64,
        "_dispatch_get_global_queue" => dispatch_get_global_queue as *const () as u64,
        "_dispatch_queue_create" => dispatch_queue_create as *const () as u64,
        "_dispatch_queue_create_with_target" => {
            dispatch_queue_create_with_target as *const () as u64
        }
        "_dispatch_queue_attr_make_with_qos_class" => {
            dispatch_queue_attr_make_with_qos_class as *const () as u64
        }
        "_dispatch_set_target_queue" => dispatch_set_target_queue as *const () as u64,
        "_dispatch_queue_get_label" => dispatch_queue_get_label as *const () as u64,
        "_dispatch_get_current_queue" => dispatch_get_current_queue as *const () as u64,
        "_dispatch_async" => dispatch_async as *const () as u64,
        "_dispatch_async_f" => dispatch_async_f as *const () as u64,
        "_dispatch_barrier_async" => dispatch_barrier_async as *const () as u64,
        "_dispatch_barrier_async_f" => dispatch_barrier_async_f as *const () as u64,
        "_dispatch_sync" => dispatch_sync as *const () as u64,
        "_dispatch_sync_f" => dispatch_sync_f as *const () as u64,
        "_dispatch_barrier_sync" => dispatch_barrier_sync as *const () as u64,
        "_dispatch_barrier_sync_f" => dispatch_barrier_sync_f as *const () as u64,
        "_dispatch_after" => dispatch_after as *const () as u64,
        "_dispatch_after_f" => dispatch_after_f as *const () as u64,
        "_dispatch_apply" => dispatch_apply as *const () as u64,
        "_dispatch_apply_f" => dispatch_apply_f as *const () as u64,
        "_dispatch_once" => dispatch_once as *const () as u64,
        "_dispatch_once_f" => dispatch_once_f as *const () as u64,
        "_dispatch_group_create" => dispatch_group_create as *const () as u64,
        "_dispatch_group_enter" => dispatch_group_enter as *const () as u64,
        "_dispatch_group_leave" => dispatch_group_leave as *const () as u64,
        "_dispatch_group_wait" => dispatch_group_wait as *const () as u64,
        "_dispatch_group_notify" => dispatch_group_notify as *const () as u64,
        "_dispatch_group_notify_f" => dispatch_group_notify_f as *const () as u64,
        "_dispatch_group_async" => dispatch_group_async as *const () as u64,
        "_dispatch_group_async_f" => dispatch_group_async_f as *const () as u64,
        "_dispatch_suspend" => dispatch_suspend as *const () as u64,
        "_dispatch_resume" => dispatch_resume as *const () as u64,
        "_dispatch_retain" => dispatch_retain as *const () as u64,
        "_dispatch_release" => dispatch_release as *const () as u64,
        "_dispatch_assert_queue" => dispatch_assert_queue as *const () as u64,
        "_dispatch_assert_queue_not" => dispatch_assert_queue_not as *const () as u64,
        "_dispatch_assert_queue_barrier" => dispatch_assert_queue as *const () as u64,
        "_dispatch_async_and_wait" => dispatch_async_and_wait as *const () as u64,
        "_dispatch_async_and_wait_f" => dispatch_async_and_wait_f as *const () as u64,
        "_dispatch_barrier_async_and_wait" => dispatch_barrier_async_and_wait as *const () as u64,
        "_dispatch_barrier_async_and_wait_f" => {
            dispatch_barrier_async_and_wait_f as *const () as u64
        }
        "_dispatch_walltime" => dispatch_walltime as *const () as u64,
        "_dispatch_wait" => dispatch_wait as *const () as u64,
        "_dispatch_notify" => dispatch_notify as *const () as u64,
        "_dispatch_debug" | "_dispatch_debugv" | "_dispatch_set_qos_class_floor" => {
            dispatch_noop as *const () as u64
        }
        "_dispatch_activate" => dispatch_activate as *const () as u64,
        "_dispatch_allow_send_signals" => dispatch_allow_send_signals as *const () as u64,
        "_dispatch_cancel" => dispatch_cancel as *const () as u64,
        "_dispatch_testcancel" => dispatch_testcancel as *const () as u64,
        "_dispatch_queue_attr_make_initially_inactive" => {
            dispatch_queue_attr_make_initially_inactive as *const () as u64
        }
        "_dispatch_queue_attr_make_with_autorelease_frequency" => {
            dispatch_queue_attr_make_with_autorelease_frequency as *const () as u64
        }
        "_dispatch_queue_get_qos_class" => dispatch_queue_get_qos_class as *const () as u64,
        "_dispatch_set_context" => dispatch_set_context as *const () as u64,
        "_dispatch_get_context" => dispatch_get_context as *const () as u64,
        "_dispatch_set_finalizer_f" => dispatch_set_finalizer_f as *const () as u64,
        "_dispatch_queue_set_specific" => dispatch_queue_set_specific as *const () as u64,
        "_dispatch_queue_get_specific" => dispatch_queue_get_specific as *const () as u64,
        "_dispatch_get_specific" => dispatch_get_specific as *const () as u64,
        "_dispatch_main" => dispatch_main as *const () as u64,
        _ => return block_objects::symbol(name).or_else(|| sources::symbol(name)),
    })
}

/// Marks the calling thread as the main thread for `dispatch_get_current_queue`.
pub fn begin() {
    if let Ok(mut id) = MAIN_THREAD.lock() {
        *id = Some(std::thread::current().id());
    }
    let mut guard = state();
    guard.shutdown = false;
}

/// Discards queued work and waits for items that are already running, so no
/// worker touches guest code while the images are torn down.
pub fn finish() -> Result<()> {
    let mut guard = state();
    guard.shutdown = true;
    for object in guard.objects.values_mut() {
        if let Kind::Queue(q) = &mut object.kind {
            q.items.clear();
        }
    }
    CV.notify_all();
    while guard.workers > 0 {
        guard = wait(guard);
    }
    Ok(())
}
