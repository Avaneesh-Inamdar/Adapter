//! Mach port rights, messages and the bootstrap namespace.
//!
//! Every process owns a name space. Receive ports are identified by a global id
//! (broker connection id in the high 32 bits); the broker forwards payloads and
//! owns service-name registration. Without `ADAPTER_MACH_BROKER` the name space
//! is process-local and can only be used for self-send.
use crate::broker::{self, Client, Incoming};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{CStr, c_char},
    path::Path,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MACH_TASK_SELF_NAME: u32 = 0x103;
const BOOTSTRAP_PORT_NAME: u32 = 0x203;
const MACH_PORT_NULL: u32 = 0;
const MACH_PORT_DEAD: u32 = 0xffff_ffff;
const FIRST_LOCAL_NAME: u32 = 0x1000;

const MACH_PORT_RIGHT_SEND: u32 = 0;
const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
const MACH_PORT_RIGHT_SEND_ONCE: u32 = 2;

const MACH_MSG_TYPE_MOVE_SEND: u32 = 17;
const MACH_MSG_TYPE_MOVE_SEND_ONCE: u32 = 18;
const MACH_MSG_TYPE_COPY_SEND: u32 = 19;
const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
const MACH_MSG_TYPE_MAKE_SEND_ONCE: u32 = 21;

const MACH_MSGH_BITS_COMPLEX: u32 = 0x8000_0000;
const MACH_SEND_MSG: u32 = 0x1;
const MACH_RCV_MSG: u32 = 0x2;
const MACH_RCV_LARGE: u32 = 0x4;
const MACH_RCV_TIMEOUT: u32 = 0x100;

const MACH_MSG_SUCCESS: i32 = 0;
const MACH_SEND_INVALID_DATA: i32 = 0x1000_0002;
const MACH_SEND_INVALID_DEST: i32 = 0x1000_0003;
const MACH_SEND_INVALID_REPLY: i32 = 0x1000_0009;
const MACH_SEND_INVALID_RIGHT: i32 = 0x1000_000a;
const MACH_SEND_INVALID_NOTIFY: i32 = 0x1000_000b;
const MACH_SEND_TOO_LARGE: i32 = 0x1000_000e;
const MACH_SEND_INVALID_TYPE: i32 = 0x1000_000f;
const MACH_SEND_INVALID_OPTIONS: i32 = 0x1000_0013;
const MACH_RCV_INVALID_NAME: i32 = 0x1000_4002;
const MACH_RCV_TIMED_OUT: i32 = 0x1000_4003;
const MACH_RCV_TOO_LARGE: i32 = 0x1000_4004;
const MACH_RCV_INVALID_NOTIFY: i32 = 0x1000_4007;
const MACH_RCV_INVALID_DATA: i32 = 0x1000_4008;
const MACH_RCV_PORT_DIED: i32 = 0x1000_4009;

const KERN_SUCCESS: i32 = 0;
const KERN_INVALID_ARGUMENT: i32 = 4;
const KERN_FAILURE: i32 = 5;
const KERN_INVALID_NAME: i32 = 15;
const KERN_INVALID_RIGHT: i32 = 17;

const BOOTSTRAP_SUCCESS: i32 = 0;
const BOOTSTRAP_NOT_PRIVILEGED: i32 = 1100;
const BOOTSTRAP_UNKNOWN_SERVICE: i32 = 1102;

const HEADER_SIZE: usize = 24;
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Header {
    bits: u32,
    size: u32,
    remote: u32,
    local: u32,
    voucher: u32,
    id: i32,
}

static MACH_TASK_SELF: u32 = MACH_TASK_SELF_NAME;
static BOOTSTRAP_PORT: u32 = BOOTSTRAP_PORT_NAME;
static STATE: Mutex<Option<State>> = Mutex::new(None);
static INBOX: Mutex<VecDeque<Incoming>> = Mutex::new(VecDeque::new());
static COND: Condvar = Condvar::new();
static LOST: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct Port {
    global: u64,
    receive: bool,
    send_refs: u32,
    send_once: u32,
    dead: bool,
}

struct Delivery {
    id: i32,
    bits: u32,
    local: u64,
    body: Vec<u8>,
}

#[derive(Clone, Copy)]
enum Consume {
    None,
    Send,
    SendOnce,
}

struct State {
    conn: u64,
    next_name: u32,
    next_port: u32,
    ports: BTreeMap<u32, Port>,
    by_global: BTreeMap<u64, u32>,
    queues: BTreeMap<u64, VecDeque<Delivery>>,
    services: BTreeMap<String, u64>,
    broker: Option<Arc<Client>>,
    trace: bool,
}

impl State {
    fn fresh_name(&mut self) -> u32 {
        loop {
            let name = self.next_name;
            self.next_name = self.next_name.wrapping_add(1);
            if name == MACH_TASK_SELF_NAME || name == BOOTSTRAP_PORT_NAME {
                continue;
            }
            return name;
        }
    }

    fn fresh_global(&mut self) -> u64 {
        let sequence = u64::from(self.next_port.max(1));
        self.next_port = self.next_port.wrapping_add(1);
        if self.next_port == 0 {
            self.next_port = 1;
        }
        (self.conn << 32) | sequence
    }

    fn alloc_receive(&mut self) -> (u32, u64) {
        let name = self.fresh_name();
        let global = self.fresh_global();
        self.ports.insert(
            name,
            Port {
                global,
                receive: true,
                send_refs: 0,
                send_once: 0,
                dead: false,
            },
        );
        self.by_global.insert(global, name);
        self.queues.insert(global, VecDeque::new());
        (name, global)
    }

    fn alloc_send(&mut self, global: u64, once: bool) -> u32 {
        let name = self.fresh_name();
        self.ports.insert(
            name,
            Port {
                global,
                receive: false,
                send_refs: u32::from(!once),
                send_once: u32::from(once),
                dead: false,
            },
        );
        name
    }

    fn remove_receive(&mut self, name: u32) -> Option<Port> {
        let port = self.ports.remove(&name)?;
        if port.receive {
            self.by_global.remove(&port.global);
            self.queues.remove(&port.global);
            self.services.retain(|_, global| *global != port.global);
            for other in self.ports.values_mut() {
                if !other.receive && other.global == port.global {
                    other.dead = true;
                }
            }
        }
        Some(port)
    }
}

fn deliver(incoming: Incoming) {
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.push_back(incoming);
    }
    COND.notify_all();
}

fn broker_lost() {
    LOST.store(true, Ordering::SeqCst);
    COND.notify_all();
}

fn drain_inbox(state: &mut State) {
    if let Ok(mut inbox) = INBOX.lock() {
        while let Some(incoming) = inbox.pop_front() {
            if let Some(queue) = state.queues.get_mut(&incoming.dest) {
                queue.push_back(Delivery {
                    id: incoming.id,
                    bits: incoming.bits,
                    local: incoming.local,
                    body: incoming.body,
                });
            }
        }
    }
}

pub fn begin(trace: bool) -> crate::Result<()> {
    let (broker, conn) = match std::env::var_os("ADAPTER_MACH_BROKER") {
        Some(path) if !path.is_empty() => {
            let delivery: Arc<dyn Fn(Incoming) + Send + Sync> = Arc::new(deliver);
            let lost: Arc<dyn Fn() + Send + Sync> = Arc::new(broker_lost);
            let (client, conn) = Client::connect(Path::new(&path), delivery, lost)?;
            (Some(client), conn)
        }
        _ => (None, 0),
    };
    if trace {
        match &broker {
            Some(_) => eprintln!("adapter-run [mach]: broker connection {conn}"),
            None => {
                eprintln!("adapter-run [mach]: process-local namespace (ADAPTER_MACH_BROKER unset)")
            }
        }
    }
    LOST.store(false, Ordering::SeqCst);
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.clear();
    }
    *STATE.lock().map_err(|_| "mach state poisoned")? = Some(State {
        conn,
        next_name: FIRST_LOCAL_NAME,
        next_port: 1,
        ports: BTreeMap::new(),
        by_global: BTreeMap::new(),
        queues: BTreeMap::new(),
        services: BTreeMap::new(),
        broker,
        trace,
    });
    Ok(())
}

pub fn finish() -> crate::Result<()> {
    let state = match STATE.lock() {
        Ok(mut guard) => guard.take(),
        Err(_) => None,
    };
    if let Some(state) = state
        && let Some(client) = state.broker
    {
        client.close();
    }
    Ok(())
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_mach_task_self_" | "__mach_task_self_" => {
            std::ptr::addr_of!(MACH_TASK_SELF) as usize as u64
        }
        "_bootstrap_port" | "__bootstrap_port" => {
            std::ptr::addr_of!(BOOTSTRAP_PORT) as usize as u64
        }
        "_mach_port_allocate" | "__mach_port_allocate" => mach_port_allocate as *const () as u64,
        "_mach_port_deallocate" | "__mach_port_deallocate" => {
            mach_port_deallocate as *const () as u64
        }
        "_mach_port_mod_refs" | "__mach_port_mod_refs" => mach_port_mod_refs as *const () as u64,
        "_mach_port_destroy" | "__mach_port_destroy" => mach_port_destroy as *const () as u64,
        "_mach_port_insert_right" | "__mach_port_insert_right" => {
            mach_port_insert_right as *const () as u64
        }
        "_mach_msg" | "__mach_msg" => mach_msg as *const () as u64,
        "_bootstrap_check_in" | "__bootstrap_check_in" => bootstrap_check_in as *const () as u64,
        "_bootstrap_look_up" | "__bootstrap_look_up" => bootstrap_look_up as *const () as u64,
        _ => return None,
    })
}

fn check_task(task: u32) -> bool {
    task == MACH_TASK_SELF_NAME
}

fn write_port(output: *mut u32, value: u32) -> bool {
    if output.is_null() {
        return false;
    }
    unsafe {
        std::ptr::write_unaligned(output, value);
    }
    true
}

extern "sysv64" fn mach_port_allocate(task: u32, right: u32, name: *mut u32) -> i32 {
    if !check_task(task) || right != MACH_PORT_RIGHT_RECEIVE || name.is_null() {
        return KERN_INVALID_ARGUMENT;
    }
    match STATE.lock() {
        Ok(mut guard) => match guard.as_mut() {
            Some(state) => {
                let (local, _) = state.alloc_receive();
                if !write_port(name, local) {
                    return KERN_INVALID_ARGUMENT;
                }
                KERN_SUCCESS
            }
            None => KERN_FAILURE,
        },
        Err(_) => KERN_FAILURE,
    }
}

extern "sysv64" fn mach_port_deallocate(task: u32, name: u32) -> i32 {
    if !check_task(task) {
        return KERN_INVALID_ARGUMENT;
    }
    if name == MACH_PORT_NULL {
        return KERN_SUCCESS;
    }
    if name == MACH_TASK_SELF_NAME || name == BOOTSTRAP_PORT_NAME {
        return KERN_INVALID_RIGHT;
    }
    let Ok(mut guard) = STATE.lock() else {
        return KERN_FAILURE;
    };
    let Some(state) = guard.as_mut() else {
        return KERN_FAILURE;
    };
    let Some(port) = state.ports.get(&name) else {
        return KERN_INVALID_NAME;
    };
    if port.receive && port.send_refs == 0 && port.send_once == 0 {
        return KERN_INVALID_RIGHT;
    }
    let remove = {
        let port = state.ports.get_mut(&name).expect("port checked above");
        if port.send_refs > 0 {
            port.send_refs -= 1;
        } else if port.send_once > 0 {
            port.send_once -= 1;
        } else {
            return KERN_INVALID_RIGHT;
        }
        port.send_refs == 0 && port.send_once == 0 && !port.receive
    };
    if remove {
        state.ports.remove(&name);
    }
    KERN_SUCCESS
}

extern "sysv64" fn mach_port_mod_refs(task: u32, name: u32, right: u32, delta: i32) -> i32 {
    if !check_task(task) {
        return KERN_INVALID_ARGUMENT;
    }
    if right != MACH_PORT_RIGHT_SEND && right != MACH_PORT_RIGHT_SEND_ONCE {
        return KERN_INVALID_RIGHT;
    }
    let Ok(mut guard) = STATE.lock() else {
        return KERN_FAILURE;
    };
    let Some(state) = guard.as_mut() else {
        return KERN_FAILURE;
    };
    if !state.ports.contains_key(&name) {
        return KERN_INVALID_NAME;
    }
    let remove = {
        let port = state.ports.get_mut(&name).expect("port checked above");
        let current = if right == MACH_PORT_RIGHT_SEND {
            &mut port.send_refs
        } else {
            &mut port.send_once
        };
        if delta == 0 {
            return KERN_SUCCESS;
        }
        let updated = i64::from(*current) + i64::from(delta);
        if updated < 0 || updated > i64::from(u32::MAX) {
            return KERN_INVALID_RIGHT;
        }
        *current = updated as u32;
        port.send_refs == 0 && port.send_once == 0 && !port.receive
    };
    if remove {
        state.ports.remove(&name);
    }
    KERN_SUCCESS
}

extern "sysv64" fn mach_port_insert_right(
    task: u32,
    name: u32,
    poly: u32,
    disposition: u32,
) -> i32 {
    if !check_task(task) {
        return KERN_INVALID_ARGUMENT;
    }
    if name != poly {
        return KERN_INVALID_ARGUMENT;
    }
    let Ok(mut guard) = STATE.lock() else {
        return KERN_FAILURE;
    };
    let Some(state) = guard.as_mut() else {
        return KERN_FAILURE;
    };
    let Some(port) = state.ports.get_mut(&name) else {
        return KERN_INVALID_NAME;
    };
    match disposition {
        MACH_MSG_TYPE_MAKE_SEND if port.receive => port.send_refs += 1,
        MACH_MSG_TYPE_MAKE_SEND_ONCE if port.receive => port.send_once += 1,
        MACH_MSG_TYPE_COPY_SEND if port.send_refs > 0 => port.send_refs += 1,
        _ => return KERN_INVALID_RIGHT,
    }
    KERN_SUCCESS
}

extern "sysv64" fn mach_port_destroy(task: u32, name: u32) -> i32 {
    if !check_task(task) {
        return KERN_INVALID_ARGUMENT;
    }
    let retire = {
        let Ok(mut guard) = STATE.lock() else {
            return KERN_FAILURE;
        };
        let Some(state) = guard.as_mut() else {
            return KERN_FAILURE;
        };
        let Some(port) = state.ports.get(&name) else {
            return KERN_INVALID_NAME;
        };
        if !port.receive {
            return KERN_INVALID_RIGHT;
        }
        let global = port.global;
        state.remove_receive(name);
        if let Some(client) = &state.broker
            && !LOST.load(Ordering::SeqCst)
        {
            let _ = client.retire(global);
        }
        true
    };
    if retire {
        COND.notify_all();
    }
    KERN_SUCCESS
}

fn resolve_dest(state: &State, name: u32, disposition: u32) -> Result<(u64, u32, Consume), i32> {
    if name == MACH_PORT_NULL || name == MACH_PORT_DEAD {
        return Err(MACH_SEND_INVALID_DEST);
    }
    let port = state.ports.get(&name).ok_or(MACH_SEND_INVALID_DEST)?;
    if port.dead {
        return Err(MACH_SEND_INVALID_DEST);
    }
    match disposition {
        MACH_MSG_TYPE_MOVE_SEND if port.send_refs > 0 => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND, Consume::Send))
        }
        MACH_MSG_TYPE_COPY_SEND if port.send_refs > 0 => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND, Consume::None))
        }
        MACH_MSG_TYPE_MOVE_SEND_ONCE if port.send_once > 0 => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND_ONCE, Consume::SendOnce))
        }
        MACH_MSG_TYPE_MAKE_SEND if port.receive => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND, Consume::None))
        }
        MACH_MSG_TYPE_MAKE_SEND_ONCE if port.receive => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND_ONCE, Consume::None))
        }
        _ => Err(MACH_SEND_INVALID_RIGHT),
    }
}

fn resolve_reply(state: &State, name: u32, disposition: u32) -> Result<(u64, u32, Consume), i32> {
    if name == MACH_PORT_NULL {
        return Ok((0, 0, Consume::None));
    }
    if name == MACH_PORT_DEAD {
        return Err(MACH_SEND_INVALID_REPLY);
    }
    let port = state.ports.get(&name).ok_or(MACH_SEND_INVALID_REPLY)?;
    if port.dead {
        return Err(MACH_SEND_INVALID_REPLY);
    }
    match disposition {
        MACH_MSG_TYPE_MAKE_SEND if port.receive => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND, Consume::None))
        }
        MACH_MSG_TYPE_MAKE_SEND_ONCE if port.receive => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND_ONCE, Consume::None))
        }
        MACH_MSG_TYPE_COPY_SEND if port.send_refs > 0 => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND, Consume::None))
        }
        MACH_MSG_TYPE_MOVE_SEND if port.send_refs > 0 => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND, Consume::Send))
        }
        MACH_MSG_TYPE_MOVE_SEND_ONCE if port.send_once > 0 => {
            Ok((port.global, MACH_MSG_TYPE_MOVE_SEND_ONCE, Consume::SendOnce))
        }
        _ => Err(MACH_SEND_INVALID_RIGHT),
    }
}

fn consume(state: &mut State, name: u32, consume: Consume) {
    if matches!(consume, Consume::None) {
        return;
    }
    let remove = {
        let Some(port) = state.ports.get_mut(&name) else {
            return;
        };
        match consume {
            Consume::Send => port.send_refs = port.send_refs.saturating_sub(1),
            Consume::SendOnce => port.send_once = port.send_once.saturating_sub(1),
            Consume::None => {}
        }
        port.send_refs == 0 && port.send_once == 0 && !port.receive
    };
    if remove {
        state.ports.remove(&name);
    }
}

fn send_locked(state: &mut State, msg: *mut Header, send_size: u32) -> i32 {
    if send_size < HEADER_SIZE as u32 {
        return MACH_SEND_INVALID_DATA;
    }
    let header = unsafe { std::ptr::read_unaligned(msg as *const Header) };
    if header.size < HEADER_SIZE as u32 {
        return MACH_SEND_INVALID_DATA;
    }
    if header.size > send_size || header.size as usize > MAX_MESSAGE {
        return MACH_SEND_TOO_LARGE;
    }
    if header.bits & MACH_MSGH_BITS_COMPLEX != 0 {
        return MACH_SEND_INVALID_TYPE;
    }
    let dest_disposition = header.bits & 0x1f;
    let reply_disposition = (header.bits >> 8) & 0x1f;
    let (dest, dest_type, consume_dest) = match resolve_dest(state, header.remote, dest_disposition)
    {
        Ok(value) => value,
        Err(code) => return code,
    };
    let (reply, reply_type, consume_reply) =
        match resolve_reply(state, header.local, reply_disposition) {
            Ok(value) => value,
            Err(code) => return code,
        };
    let bits = reply_type | (dest_type << 8);
    let body_length = header.size as usize - HEADER_SIZE;
    let body = unsafe {
        std::slice::from_raw_parts((msg as *const u8).add(HEADER_SIZE), body_length).to_vec()
    };
    if state.by_global.contains_key(&dest) {
        let Some(queue) = state.queues.get_mut(&dest) else {
            return MACH_SEND_INVALID_DEST;
        };
        queue.push_back(Delivery {
            id: header.id,
            bits,
            local: reply,
            body,
        });
        COND.notify_all();
    } else {
        let Some(client) = &state.broker else {
            return MACH_SEND_INVALID_DEST;
        };
        if LOST.load(Ordering::SeqCst) {
            return MACH_SEND_INVALID_DEST;
        }
        match client.send(dest, bits, header.id, reply, &body) {
            Ok(0) => {}
            Ok(status) => return status as i32,
            Err(error) => {
                if state.trace {
                    eprintln!("adapter-run [mach]: send failed: {error}");
                }
                return MACH_SEND_INVALID_DEST;
            }
        }
    }
    consume(state, header.remote, consume_dest);
    consume(state, header.local, consume_reply);
    MACH_MSG_SUCCESS
}

enum ReceiveStep {
    Ready(u64),
    Wait,
}

fn receive_locked(
    mut guard: MutexGuard<'_, Option<State>>,
    msg: *mut Header,
    option: u32,
    rcv_size: u32,
    rcv_name: u32,
    timeout: u32,
) -> i32 {
    if rcv_size < HEADER_SIZE as u32 {
        return MACH_RCV_INVALID_DATA;
    }
    let deadline = if option & MACH_RCV_TIMEOUT != 0 {
        Some(Instant::now() + Duration::from_millis(u64::from(timeout)))
    } else {
        None
    };
    let large = option & MACH_RCV_LARGE != 0;
    let mut seen = false;
    loop {
        let step = {
            let Some(state) = guard.as_mut() else {
                return MACH_RCV_INVALID_NAME;
            };
            drain_inbox(state);
            let Some(port) = state.ports.get(&rcv_name) else {
                return if seen {
                    MACH_RCV_PORT_DIED
                } else {
                    MACH_RCV_INVALID_NAME
                };
            };
            if !port.receive {
                return MACH_RCV_INVALID_NAME;
            }
            seen = true;
            let global = port.global;
            let Some(queue) = state.queues.get_mut(&global) else {
                return MACH_RCV_PORT_DIED;
            };
            if let Some(front) = queue.front() {
                let total = HEADER_SIZE + front.body.len();
                if total > rcv_size as usize {
                    if large {
                        return MACH_RCV_TOO_LARGE;
                    }
                    queue.pop_front();
                    return MACH_RCV_TOO_LARGE;
                }
                ReceiveStep::Ready(global)
            } else if LOST.load(Ordering::SeqCst) && state.broker.is_some() {
                return MACH_RCV_PORT_DIED;
            } else {
                ReceiveStep::Wait
            }
        };
        match step {
            ReceiveStep::Ready(global) => {
                let Some(state) = guard.as_mut() else {
                    return MACH_RCV_INVALID_NAME;
                };
                let delivery = match state.queues.get_mut(&global).and_then(VecDeque::pop_front) {
                    Some(delivery) => delivery,
                    None => continue,
                };
                return finish_receive(state, msg, rcv_name, delivery);
            }
            ReceiveStep::Wait => match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return MACH_RCV_TIMED_OUT;
                    }
                    let (next, _) = COND
                        .wait_timeout(guard, deadline - now)
                        .unwrap_or_else(|poison| poison.into_inner());
                    guard = next;
                }
                None => {
                    guard = COND
                        .wait(guard)
                        .unwrap_or_else(|poison| poison.into_inner());
                }
            },
        }
    }
}

fn finish_receive(state: &mut State, msg: *mut Header, rcv_name: u32, delivery: Delivery) -> i32 {
    let reply_type = delivery.bits & 0x1f;
    let reply_name = if delivery.local != 0
        && (reply_type == MACH_MSG_TYPE_MOVE_SEND || reply_type == MACH_MSG_TYPE_MOVE_SEND_ONCE)
    {
        state.alloc_send(delivery.local, reply_type == MACH_MSG_TYPE_MOVE_SEND_ONCE)
    } else {
        MACH_PORT_NULL
    };
    let header = Header {
        bits: delivery.bits,
        size: (HEADER_SIZE + delivery.body.len()) as u32,
        remote: reply_name,
        local: rcv_name,
        voucher: 0,
        id: delivery.id,
    };
    unsafe {
        std::ptr::write_unaligned(msg, header);
        if !delivery.body.is_empty() {
            std::ptr::copy_nonoverlapping(
                delivery.body.as_ptr(),
                (msg as *mut u8).add(HEADER_SIZE),
                delivery.body.len(),
            );
        }
    }
    MACH_MSG_SUCCESS
}

extern "sysv64" fn mach_msg(
    msg: *mut Header,
    option: u32,
    send_size: u32,
    rcv_size: u32,
    rcv_name: u32,
    timeout: u32,
    notify: u32,
) -> i32 {
    if msg.is_null() {
        return MACH_SEND_INVALID_DATA;
    }
    let send = option & MACH_SEND_MSG != 0;
    let receive = option & MACH_RCV_MSG != 0;
    if !send && !receive {
        return MACH_SEND_INVALID_OPTIONS;
    }
    if send && notify != MACH_PORT_NULL {
        return MACH_SEND_INVALID_NOTIFY;
    }
    if receive && notify != MACH_PORT_NULL {
        return MACH_RCV_INVALID_NOTIFY;
    }
    let Ok(mut guard) = STATE.lock() else {
        return MACH_SEND_INVALID_DATA;
    };
    if guard.is_none() {
        return MACH_SEND_INVALID_DATA;
    }
    if send {
        let state = guard.as_mut().expect("state checked above");
        let code = send_locked(state, msg, send_size);
        if code != MACH_MSG_SUCCESS {
            return code;
        }
    }
    if !receive {
        return MACH_MSG_SUCCESS;
    }
    receive_locked(guard, msg, option, rcv_size, rcv_name, timeout)
}

extern "sysv64" fn bootstrap_check_in(
    bootstrap: u32,
    service_name: *const c_char,
    service_port: *mut u32,
) -> i32 {
    if bootstrap != BOOTSTRAP_PORT_NAME || service_name.is_null() || service_port.is_null() {
        return BOOTSTRAP_NOT_PRIVILEGED;
    }
    let name = match unsafe { CStr::from_ptr(service_name) }.to_str() {
        Ok(name) if !name.is_empty() && name.len() < 128 => name.to_string(),
        _ => return BOOTSTRAP_NOT_PRIVILEGED,
    };
    let Ok(mut guard) = STATE.lock() else {
        return BOOTSTRAP_NOT_PRIVILEGED;
    };
    let Some(state) = guard.as_mut() else {
        return BOOTSTRAP_NOT_PRIVILEGED;
    };
    if let Some(client) = state.broker.clone() {
        let (local, global) = state.alloc_receive();
        match client.check_in(&name, global) {
            Ok(0) => {
                write_port(service_port, local);
                BOOTSTRAP_SUCCESS
            }
            Ok(status) => {
                state.remove_receive(local);
                status as i32
            }
            Err(error) => {
                state.remove_receive(local);
                if state.trace {
                    eprintln!("adapter-run [mach]: check-in failed: {error}");
                }
                BOOTSTRAP_NOT_PRIVILEGED
            }
        }
    } else {
        if state.services.contains_key(&name) {
            return broker::SERVICE_ACTIVE as i32;
        }
        let (local, global) = state.alloc_receive();
        state.services.insert(name, global);
        write_port(service_port, local);
        BOOTSTRAP_SUCCESS
    }
}

extern "sysv64" fn bootstrap_look_up(
    bootstrap: u32,
    service_name: *const c_char,
    service_port: *mut u32,
) -> i32 {
    if bootstrap != BOOTSTRAP_PORT_NAME || service_name.is_null() || service_port.is_null() {
        return BOOTSTRAP_NOT_PRIVILEGED;
    }
    let name = match unsafe { CStr::from_ptr(service_name) }.to_str() {
        Ok(name) if !name.is_empty() && name.len() < 128 => name.to_string(),
        _ => return BOOTSTRAP_NOT_PRIVILEGED,
    };
    let Ok(mut guard) = STATE.lock() else {
        return BOOTSTRAP_NOT_PRIVILEGED;
    };
    let Some(state) = guard.as_mut() else {
        return BOOTSTRAP_NOT_PRIVILEGED;
    };
    let global = if let Some(client) = state.broker.clone() {
        if LOST.load(Ordering::SeqCst) {
            return BOOTSTRAP_NOT_PRIVILEGED;
        }
        match client.look_up(&name) {
            Ok((0, port)) if port != 0 => port,
            Ok((broker::UNKNOWN_SERVICE, _)) | Ok((_, 0)) => {
                return BOOTSTRAP_UNKNOWN_SERVICE;
            }
            Ok((status, _)) => return status as i32,
            Err(error) => {
                if state.trace {
                    eprintln!("adapter-run [mach]: lookup failed: {error}");
                }
                return BOOTSTRAP_NOT_PRIVILEGED;
            }
        }
    } else {
        match state.services.get(&name) {
            Some(global) => *global,
            None => return BOOTSTRAP_UNKNOWN_SERVICE,
        }
    };
    let local = state.alloc_send(global, false);
    write_port(service_port, local);
    BOOTSTRAP_SUCCESS
}
