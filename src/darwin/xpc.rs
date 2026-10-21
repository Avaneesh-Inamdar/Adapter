//! libxpc subset over the Adapter broker.
//!
//! Objects are immutable values behind opaque handles; connections carry
//! serialized dictionaries between Adapter workers over the same loopback
//! broker that owns the Mach bootstrap namespace. This is an adapter-defined
//! protocol, not Apple's private XPC wire format.
use crate::broker::{self, Client, XpcIncoming};
use crate::darwin::{blocks, mach};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    ffi::{CStr, CString, c_char, c_void},
    sync::{Arc, Condvar, Mutex},
};

const WIRE_MAGIC: u16 = 0x5850;
const WIRE_VERSION: u8 = 1;
const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_STRING: usize = 1024 * 1024;

#[repr(C)]
struct Marker {
    tag: u8,
}

static TYPE_CONNECTION: Marker = Marker { tag: 1 };
static TYPE_ENDPOINT: Marker = Marker { tag: 2 };
static TYPE_NULL: Marker = Marker { tag: 3 };
static TYPE_BOOL: Marker = Marker { tag: 4 };
static TYPE_INT64: Marker = Marker { tag: 5 };
static TYPE_UINT64: Marker = Marker { tag: 6 };
static TYPE_DOUBLE: Marker = Marker { tag: 7 };
static TYPE_DATE: Marker = Marker { tag: 8 };
static TYPE_DATA: Marker = Marker { tag: 9 };
static TYPE_STRING: Marker = Marker { tag: 10 };
static TYPE_UUID: Marker = Marker { tag: 11 };
static TYPE_FD: Marker = Marker { tag: 12 };
static TYPE_SHMEM: Marker = Marker { tag: 13 };
static TYPE_ARRAY: Marker = Marker { tag: 14 };
static TYPE_DICTIONARY: Marker = Marker { tag: 15 };
static TYPE_ERROR: Marker = Marker { tag: 16 };
static TYPE_RICH_ERROR: Marker = Marker { tag: 17 };
static BOOL_TRUE: Marker = Marker { tag: 18 };
static BOOL_FALSE: Marker = Marker { tag: 19 };
static ERROR_INVALID: Marker = Marker { tag: 20 };
static ERROR_INTERRUPTED: Marker = Marker { tag: 21 };
static ERROR_KEY_TEXT: [u8; 20] = *b"XPCErrorDescription\0";

#[repr(transparent)]
struct ConstPtr(*const u8);
unsafe impl Sync for ConstPtr {}
static ERROR_KEY_POINTER: ConstPtr = ConstPtr(ERROR_KEY_TEXT.as_ptr());

fn addr<T>(value: &'static T) -> usize {
    value as *const T as usize
}

fn error_code(key: usize) -> Option<u32> {
    if key == addr(&ERROR_INVALID) {
        Some(broker::XPC_INVALID)
    } else if key == addr(&ERROR_INTERRUPTED) {
        Some(broker::XPC_INTERRUPTED)
    } else {
        None
    }
}

fn error_ptr(code: u32) -> usize {
    if code == broker::XPC_INTERRUPTED {
        addr(&ERROR_INTERRUPTED)
    } else {
        addr(&ERROR_INVALID)
    }
}

fn error_text(code: u32) -> &'static str {
    if code == broker::XPC_INTERRUPTED {
        "Connection interrupted"
    } else {
        "Connection invalid"
    }
}

fn bool_static(key: usize) -> Option<bool> {
    if key == addr(&BOOL_TRUE) {
        Some(true)
    } else if key == addr(&BOOL_FALSE) {
        Some(false)
    } else {
        None
    }
}

fn is_static(key: usize) -> bool {
    error_code(key).is_some() || bool_static(key).is_some()
}

#[derive(Clone, Copy)]
enum Kind {
    Null,
    Bool,
    Int64,
    Uint64,
    Double,
    Data,
    String,
    Array,
    Dictionary,
    Connection,
    Error,
}

fn marker(kind: Kind) -> usize {
    addr(match kind {
        Kind::Null => &TYPE_NULL,
        Kind::Bool => &TYPE_BOOL,
        Kind::Int64 => &TYPE_INT64,
        Kind::Uint64 => &TYPE_UINT64,
        Kind::Double => &TYPE_DOUBLE,
        Kind::Data => &TYPE_DATA,
        Kind::String => &TYPE_STRING,
        Kind::Array => &TYPE_ARRAY,
        Kind::Dictionary => &TYPE_DICTIONARY,
        Kind::Connection => &TYPE_CONNECTION,
        Kind::Error => &TYPE_ERROR,
    })
}

#[derive(Clone, Copy)]
struct Remote {
    connection: usize,
    route: u64,
    token: u64,
}

enum Value {
    Null,
    Bool(bool),
    Int64(i64),
    Uint64(u64),
    Double(f64),
    Data(Vec<u8>),
    String(CString),
    Array(Vec<usize>),
    Dictionary(BTreeMap<String, usize>),
    Connection(Box<Connection>),
}

struct Object {
    _storage: Box<u8>,
    refs: u32,
    value: Value,
    remote: Option<Remote>,
    reply_used: bool,
}

enum Queued {
    Object(usize),
}

enum ReplySink {
    Handler(usize),
    Sync(Arc<(Mutex<Option<usize>>, Condvar)>),
}

struct Outgoing {
    kind: u32,
    token: u64,
    reply_to: Option<u64>,
    body: Vec<u8>,
}

struct Connection {
    id: u64,
    peer: u64,
    name: CString,
    listener: bool,
    active: bool,
    internal: bool,
    suspended: u32,
    cancelled: bool,
    invalid: bool,
    handler: usize,
    target_queue: usize,
    mach_port: u32,
    pending: BTreeMap<u64, ReplySink>,
    queue: VecDeque<Queued>,
    outbox: VecDeque<Outgoing>,
}

struct State {
    objects: HashMap<usize, Object>,
    routes: HashMap<u64, usize>,
    broker: Option<Arc<Client>>,
    dispatcher: bool,
    next_token: u64,
}

enum Event {
    Connect(usize),
    Accept {
        route: u64,
        peer: u64,
        name: String,
    },
    Deliver {
        dest: u64,
        kind: u32,
        reply: u64,
        token: u64,
        body: Vec<u8>,
    },
    Disconnect {
        key: Option<usize>,
        route: u64,
        error: u32,
    },
    Wake,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static INBOX: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());
static INBOX_COND: Condvar = Condvar::new();

fn push_event(event: Event) {
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.push_back(event);
    }
    INBOX_COND.notify_all();
}

fn xpc_delivery(incoming: XpcIncoming) {
    let event = match incoming.kind {
        broker::XPC_ACCEPT => Event::Accept {
            route: incoming.dest,
            peer: incoming.reply,
            name: String::from_utf8(incoming.body).unwrap_or_default(),
        },
        broker::XPC_MESSAGE | broker::XPC_REPLY => Event::Deliver {
            dest: incoming.dest,
            kind: incoming.kind,
            reply: incoming.reply,
            token: incoming.token,
            body: incoming.body,
        },
        broker::XPC_INVALID | broker::XPC_INTERRUPTED => Event::Disconnect {
            key: None,
            route: incoming.dest,
            error: incoming.kind,
        },
        _ => return,
    };
    push_event(event);
}

fn ensure_state() -> bool {
    let mut guard = match STATE.lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };
    if guard.is_none() {
        let broker = mach::broker_client();
        if let Some(client) = &broker {
            client.set_xpc_delivery(Arc::new(xpc_delivery));
        }
        *guard = Some(State {
            objects: HashMap::new(),
            routes: HashMap::new(),
            broker,
            dispatcher: false,
            next_token: 1,
        });
    }
    true
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    if !ensure_state() {
        return None;
    }
    match STATE.lock() {
        Ok(mut guard) => {
            let state = guard.as_mut().expect("state initialized above");
            Some(f(state))
        }
        Err(_) => None,
    }
}

fn ensure_dispatcher() {
    if !ensure_state() {
        return;
    }
    let start = match STATE.lock() {
        Ok(mut guard) => {
            let state = guard.as_mut().expect("state initialized above");
            if state.dispatcher {
                false
            } else {
                state.dispatcher = true;
                true
            }
        }
        Err(_) => false,
    };
    if start {
        std::thread::spawn(dispatcher_loop);
    }
}

fn dispatcher_loop() {
    loop {
        let event = {
            let mut inbox = match INBOX.lock() {
                Ok(inbox) => inbox,
                Err(_) => return,
            };
            while inbox.is_empty() {
                inbox = match INBOX_COND.wait(inbox) {
                    Ok(inbox) => inbox,
                    Err(_) => return,
                };
            }
            inbox.pop_front()
        };
        match event {
            Some(Event::Connect(key)) => process_connect(key),
            Some(Event::Accept { route, peer, name }) => process_accept(route, peer, name),
            Some(Event::Deliver {
                dest,
                kind,
                reply,
                token,
                body,
            }) => process_deliver(dest, kind, reply, token, &body),
            Some(Event::Disconnect { key, route, error }) => {
                process_disconnect(key, route, error);
            }
            Some(Event::Wake) => process_wake(),
            None => return,
        }
    }
}

fn new_object(state: &mut State, value: Value) -> usize {
    let storage = Box::new(0u8);
    let key = &*storage as *const u8 as usize;
    state.objects.insert(
        key,
        Object {
            _storage: storage,
            refs: 1,
            value,
            remote: None,
            reply_used: false,
        },
    );
    key
}

fn connection_mut(state: &mut State, key: usize) -> Option<&mut Connection> {
    match &mut state.objects.get_mut(&key)?.value {
        Value::Connection(connection) => Some(connection),
        _ => None,
    }
}

fn connection_ref(state: &State, key: usize) -> Option<&Connection> {
    match &state.objects.get(&key)?.value {
        Value::Connection(connection) => Some(connection),
        _ => None,
    }
}

fn retain_key(state: &mut State, key: usize) {
    if is_static(key) {
        return;
    }
    if let Some(object) = state.objects.get_mut(&key) {
        object.refs = object.refs.saturating_add(1);
    }
}

fn release_key(state: &mut State, key: usize, depth: usize) {
    if is_static(key) || depth > MAX_DEPTH {
        return;
    }
    let Some(object) = state.objects.get_mut(&key) else {
        return;
    };
    if object.refs > 1 {
        object.refs -= 1;
        return;
    }
    let Some(object) = state.objects.remove(&key) else {
        return;
    };
    let mut children = Vec::new();
    match object.value {
        Value::Array(items) => children = items,
        Value::Dictionary(map) => children = map.values().copied().collect(),
        Value::Connection(connection) => {
            state.routes.retain(|_, mapped| *mapped != key);
            unsafe {
                blocks::release(connection.handler as *mut c_void);
                for sink in connection.pending.values() {
                    if let ReplySink::Handler(block) = sink {
                        blocks::release(*block as *mut c_void);
                    }
                }
            }
            for queued in connection.queue {
                let Queued::Object(child) = queued;
                children.push(child);
            }
        }
        _ => {}
    }
    for child in children {
        release_key(state, child, depth + 1);
    }
}

fn object_remote(state: &State, key: usize) -> Option<Remote> {
    state.objects.get(&key).and_then(|object| object.remote)
}

#[derive(Clone, Debug, PartialEq)]
enum Ser {
    Null,
    Bool(bool),
    Int64(i64),
    Uint64(u64),
    Double(f64),
    String(String),
    Data(Vec<u8>),
    Array(Vec<Ser>),
    Dictionary(Vec<(String, Ser)>),
    Error(u32),
}

fn account(bytes: &mut usize, add: usize) -> Result<(), String> {
    *bytes = bytes.saturating_add(add);
    if *bytes > MAX_PAYLOAD {
        return Err("XPC object exceeds the payload limit".into());
    }
    Ok(())
}

fn snapshot(state: &State, key: usize, depth: usize, bytes: &mut usize) -> Result<Ser, String> {
    if depth > MAX_DEPTH {
        return Err("XPC object graph is too deep".into());
    }
    account(bytes, 8)?;
    if let Some(value) = bool_static(key) {
        return Ok(Ser::Bool(value));
    }
    if let Some(code) = error_code(key) {
        return Ok(Ser::Error(code));
    }
    let object = state
        .objects
        .get(&key)
        .ok_or_else(|| "XPC object is not live".to_string())?;
    match &object.value {
        Value::Null => Ok(Ser::Null),
        Value::Bool(value) => Ok(Ser::Bool(*value)),
        Value::Int64(value) => Ok(Ser::Int64(*value)),
        Value::Uint64(value) => Ok(Ser::Uint64(*value)),
        Value::Double(value) => Ok(Ser::Double(*value)),
        Value::Data(data) => {
            account(bytes, data.len())?;
            Ok(Ser::Data(data.clone()))
        }
        Value::String(text) => {
            let text = text
                .to_str()
                .map_err(|_| "XPC string is not UTF-8".to_string())?
                .to_owned();
            account(bytes, text.len())?;
            Ok(Ser::String(text))
        }
        Value::Array(items) => {
            account(bytes, items.len() * 8)?;
            let mut out = Vec::with_capacity(items.len());
            for child in items {
                out.push(snapshot(state, *child, depth + 1, bytes)?);
            }
            Ok(Ser::Array(out))
        }
        Value::Dictionary(map) => {
            account(bytes, map.len() * 8)?;
            let mut out = Vec::with_capacity(map.len());
            for (name, child) in map {
                account(bytes, name.len())?;
                out.push((name.clone(), snapshot(state, *child, depth + 1, bytes)?));
            }
            Ok(Ser::Dictionary(out))
        }
        Value::Connection(_) => Err("XPC connection objects cannot cross the broker".into()),
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    let len = u32::try_from(bytes.len()).map_err(|_| "XPC value is too large".to_string())?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn encode_ser(value: &Ser, out: &mut Vec<u8>) -> Result<(), String> {
    if out.len() > MAX_PAYLOAD {
        return Err("XPC payload exceeds the limit".into());
    }
    match value {
        Ser::Null => out.push(0),
        Ser::Bool(value) => {
            out.push(1);
            out.push(u8::from(*value));
        }
        Ser::Int64(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Ser::Uint64(value) => {
            out.push(3);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Ser::Double(value) => {
            out.push(4);
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        Ser::String(text) => {
            if text.len() > MAX_STRING {
                return Err("XPC string exceeds 1 MiB".into());
            }
            out.push(5);
            put_bytes(out, text.as_bytes())?;
        }
        Ser::Data(data) => {
            out.push(6);
            put_bytes(out, data)?;
        }
        Ser::Array(items) => {
            out.push(7);
            let count =
                u32::try_from(items.len()).map_err(|_| "XPC array is too large".to_string())?;
            out.extend_from_slice(&count.to_le_bytes());
            for item in items {
                encode_ser(item, out)?;
            }
        }
        Ser::Dictionary(pairs) => {
            out.push(8);
            let count = u32::try_from(pairs.len())
                .map_err(|_| "XPC dictionary is too large".to_string())?;
            out.extend_from_slice(&count.to_le_bytes());
            for (name, item) in pairs {
                if name.len() > MAX_STRING {
                    return Err("XPC dictionary key exceeds 1 MiB".into());
                }
                put_bytes(out, name.as_bytes())?;
                encode_ser(item, out)?;
            }
        }
        Ser::Error(code) => {
            out.push(9);
            out.extend_from_slice(&code.to_le_bytes());
        }
    }
    Ok(())
}

fn encode_object(state: &State, key: usize) -> Result<Vec<u8>, String> {
    let mut bytes = 0;
    let value = snapshot(state, key, 0, &mut bytes)?;
    let mut out = Vec::with_capacity(bytes + 32);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    encode_ser(&value, &mut out)?;
    if out.len() > MAX_PAYLOAD {
        return Err("XPC payload exceeds the limit".into());
    }
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, String> {
        let byte = *self
            .data
            .get(self.pos)
            .ok_or_else(|| "truncated XPC payload".to_string())?;
        self.pos += 1;
        Ok(byte)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let end = self.pos + 4;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| "truncated XPC payload".to_string())?;
        self.pos = end;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn bytes(&mut self, limit: usize) -> Result<&'a [u8], String> {
        let len = self.u32()? as usize;
        if len > limit {
            return Err("XPC value exceeds its limit".into());
        }
        let end = self.pos + len;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| "truncated XPC payload".to_string())?;
        self.pos = end;
        Ok(bytes)
    }

    fn value(&mut self, depth: usize) -> Result<Ser, String> {
        if depth > MAX_DEPTH {
            return Err("XPC object graph is too deep".into());
        }
        match self.u8()? {
            0 => Ok(Ser::Null),
            1 => Ok(Ser::Bool(self.u8()? != 0)),
            2 => {
                let mut bytes = [0u8; 8];
                for slot in &mut bytes {
                    *slot = self.u8()?;
                }
                Ok(Ser::Int64(i64::from_le_bytes(bytes)))
            }
            3 => {
                let mut bytes = [0u8; 8];
                for slot in &mut bytes {
                    *slot = self.u8()?;
                }
                Ok(Ser::Uint64(u64::from_le_bytes(bytes)))
            }
            4 => {
                let mut bytes = [0u8; 8];
                for slot in &mut bytes {
                    *slot = self.u8()?;
                }
                Ok(Ser::Double(f64::from_bits(u64::from_le_bytes(bytes))))
            }
            5 => {
                let bytes = self.bytes(MAX_STRING)?;
                let text = String::from_utf8(bytes.to_vec())
                    .map_err(|_| "XPC string is not UTF-8".to_string())?;
                if text.contains('\0') {
                    return Err("XPC string contains NUL".into());
                }
                Ok(Ser::String(text))
            }
            6 => Ok(Ser::Data(self.bytes(MAX_PAYLOAD)?.to_vec())),
            7 => {
                let count = self.u32()? as usize;
                if count > self.data.len() / 2 {
                    return Err("invalid XPC array count".into());
                }
                let mut items = Vec::with_capacity(count.min(1024));
                for _ in 0..count {
                    items.push(self.value(depth + 1)?);
                }
                Ok(Ser::Array(items))
            }
            8 => {
                let count = self.u32()? as usize;
                if count > self.data.len() / 4 {
                    return Err("invalid XPC dictionary count".into());
                }
                let mut pairs = Vec::with_capacity(count.min(1024));
                for _ in 0..count {
                    let key = self.bytes(MAX_STRING)?;
                    let key = String::from_utf8(key.to_vec())
                        .map_err(|_| "XPC dictionary key is not UTF-8".to_string())?;
                    if key.contains('\0') {
                        return Err("XPC dictionary key contains NUL".into());
                    }
                    pairs.push((key, self.value(depth + 1)?));
                }
                Ok(Ser::Dictionary(pairs))
            }
            9 => Ok(Ser::Error(self.u32()?)),
            tag => Err(format!("unknown XPC value tag {tag}")),
        }
    }
}

fn decode_value(data: &[u8]) -> Result<Ser, String> {
    if data.len() < 3 {
        return Err("truncated XPC payload header".into());
    }
    if u16::from_le_bytes([data[0], data[1]]) != WIRE_MAGIC {
        return Err("bad XPC payload magic".into());
    }
    if data[2] != WIRE_VERSION {
        return Err(format!("unsupported XPC payload version {}", data[2]));
    }
    let mut reader = Reader { data, pos: 3 };
    let value = reader.value(0)?;
    if reader.pos != data.len() {
        return Err("trailing bytes in XPC payload".into());
    }
    Ok(value)
}

fn instantiate(state: &mut State, value: &Ser) -> usize {
    match value {
        Ser::Null => new_object(state, Value::Null),
        Ser::Bool(true) => addr(&BOOL_TRUE),
        Ser::Bool(false) => addr(&BOOL_FALSE),
        Ser::Int64(value) => new_object(state, Value::Int64(*value)),
        Ser::Uint64(value) => new_object(state, Value::Uint64(*value)),
        Ser::Double(value) => new_object(state, Value::Double(*value)),
        Ser::String(text) => {
            let bytes: Vec<u8> = text.bytes().filter(|byte| *byte != 0).collect();
            let text = CString::new(bytes).unwrap_or_default();
            new_object(state, Value::String(text))
        }
        Ser::Data(data) => new_object(state, Value::Data(data.clone())),
        Ser::Array(items) => {
            let mut children = Vec::with_capacity(items.len());
            for item in items {
                children.push(instantiate(state, item));
            }
            new_object(state, Value::Array(children))
        }
        Ser::Dictionary(pairs) => {
            let mut map = BTreeMap::new();
            for (name, item) in pairs {
                let child = instantiate(state, item);
                map.insert(name.clone(), child);
            }
            new_object(state, Value::Dictionary(map))
        }
        Ser::Error(code) => error_ptr(*code),
    }
}

fn alloc_token(state: &mut State) -> u64 {
    let token = state.next_token.max(1);
    state.next_token = state.next_token.wrapping_add(1).max(1);
    token
}

fn invokes_cycle(state: &State, container: usize, candidate: usize, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    if candidate == container {
        return true;
    }
    match state.objects.get(&candidate).map(|object| &object.value) {
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| invokes_cycle(state, container, *item, depth + 1)),
        Some(Value::Dictionary(map)) => map
            .values()
            .any(|item| invokes_cycle(state, container, *item, depth + 1)),
        _ => false,
    }
}

fn dict_insert(state: &mut State, dict: usize, name: &str, child: usize) -> bool {
    match state.objects.get_mut(&dict).map(|object| &mut object.value) {
        Some(Value::Dictionary(map)) => {
            if let Some(old) = map.insert(name.to_string(), child) {
                release_key(state, old, 0);
            }
            true
        }
        _ => false,
    }
}

fn dict_remove(state: &mut State, dict: usize, name: &str) -> bool {
    match state.objects.get_mut(&dict).map(|object| &mut object.value) {
        Some(Value::Dictionary(map)) => {
            if let Some(old) = map.remove(name) {
                release_key(state, old, 0);
            }
            true
        }
        _ => false,
    }
}

fn dict_get(state: &State, dict: usize, name: &str) -> Option<usize> {
    match state.objects.get(&dict).map(|object| &object.value) {
        Some(Value::Dictionary(map)) => map.get(name).copied(),
        _ => None,
    }
}

fn array_get(state: &State, array: usize, index: usize) -> Option<usize> {
    match state.objects.get(&array).map(|object| &object.value) {
        Some(Value::Array(items)) => items.get(index).copied(),
        _ => None,
    }
}

fn array_set(state: &mut State, array: usize, index: usize, child: usize) -> bool {
    let length = match state.objects.get(&array).map(|object| &object.value) {
        Some(Value::Array(items)) => items.len(),
        _ => return false,
    };
    if index > length {
        return false;
    }
    let retained = (index < length).then(|| match &state.objects[&array].value {
        Value::Array(items) => items[index],
        _ => 0,
    });
    match state
        .objects
        .get_mut(&array)
        .map(|object| &mut object.value)
    {
        Some(Value::Array(items)) => {
            if index == length {
                items.push(child);
            } else {
                items[index] = child;
            }
            if let Some(retained) = retained {
                release_key(state, retained, 0);
            }
            true
        }
        _ => false,
    }
}

fn array_push(state: &mut State, array: usize, child: usize) -> bool {
    match state
        .objects
        .get_mut(&array)
        .map(|object| &mut object.value)
    {
        Some(Value::Array(items)) => {
            items.push(child);
            true
        }
        _ => false,
    }
}

fn kind_of(state: &State, key: usize) -> Option<Kind> {
    if error_code(key).is_some() {
        return Some(Kind::Error);
    }
    if bool_static(key).is_some() {
        return Some(Kind::Bool);
    }
    Some(match &state.objects.get(&key)?.value {
        Value::Null => Kind::Null,
        Value::Bool(_) => Kind::Bool,
        Value::Int64(_) => Kind::Int64,
        Value::Uint64(_) => Kind::Uint64,
        Value::Double(_) => Kind::Double,
        Value::Data(_) => Kind::Data,
        Value::String(_) => Kind::String,
        Value::Array(_) => Kind::Array,
        Value::Dictionary(_) => Kind::Dictionary,
        Value::Connection(_) => Kind::Connection,
    })
}

fn c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

fn key_ptr(key: usize) -> *mut c_void {
    key as *mut c_void
}

fn set_dict_value(dict: usize, name: &str, value: usize, retain: bool) -> bool {
    with_state(|state| {
        let kind = kind_of(state, dict);
        if !matches!(kind, Some(Kind::Dictionary)) {
            return false;
        }
        if invokes_cycle(state, dict, value, 0) {
            return false;
        }
        if retain {
            retain_key(state, value);
        }
        dict_insert(state, dict, name, value)
    })
    .unwrap_or(false)
}

fn dict_set_owned(dict: usize, name: &str, value: Value) -> bool {
    with_state(|state| {
        if !matches!(kind_of(state, dict), Some(Kind::Dictionary)) {
            return false;
        }
        let child = new_object(state, value);
        dict_insert(state, dict, name, child)
    })
    .unwrap_or(false)
}

fn array_append_value(array: usize, value: usize, retain: bool) -> bool {
    with_state(|state| {
        if !matches!(kind_of(state, array), Some(Kind::Array)) {
            return false;
        }
        if invokes_cycle(state, array, value, 0) {
            return false;
        }
        if retain {
            retain_key(state, value);
        }
        array_push(state, array, value)
    })
    .unwrap_or(false)
}

fn array_set_value(array: usize, index: usize, value: usize, retain: bool) -> bool {
    with_state(|state| {
        if !matches!(kind_of(state, array), Some(Kind::Array)) {
            return false;
        }
        if invokes_cycle(state, array, value, 0) {
            return false;
        }
        if retain {
            retain_key(state, value);
        }
        array_set(state, array, index, value)
    })
    .unwrap_or(false)
}

fn array_set_owned(array: usize, index: usize, value: Value) -> bool {
    with_state(|state| {
        if !matches!(kind_of(state, array), Some(Kind::Array)) {
            return false;
        }
        let child = new_object(state, value);
        array_set(state, array, index, child)
    })
    .unwrap_or(false)
}

fn connection_name_pointer(connection: &Connection) -> *const c_char {
    connection.name.as_ptr()
}

struct Prepared {
    from: u64,
    dest: u64,
    kind: u32,
    token: u64,
    body: Vec<u8>,
}

fn prepare_send(
    state: &mut State,
    connection: usize,
    message: usize,
    sink: Option<ReplySink>,
) -> Result<Option<Prepared>, ()> {
    let remote = object_remote(state, message);
    let body = encode_object(state, message).map_err(|_| ())?;
    let token = match remote {
        Some(remote) => remote.token,
        None if sink.is_some() => alloc_token(state),
        None => 0,
    };
    let (kind, reply_to) = match remote {
        Some(remote) => (broker::XPC_REPLY, Some(remote.route)),
        None => (broker::XPC_MESSAGE, None),
    };
    let conn = connection_mut(state, connection).ok_or(())?;
    if conn.cancelled || conn.invalid {
        return Err(());
    }
    if let Some(sink) = sink {
        conn.pending.insert(token, sink);
    }
    if conn.id == 0 {
        conn.outbox.push_back(Outgoing {
            kind,
            token,
            reply_to,
            body,
        });
        return Ok(None);
    }
    Ok(Some(Prepared {
        from: conn.id,
        dest: reply_to.unwrap_or(conn.peer),
        kind,
        token,
        body,
    }))
}

fn send_prepared(client: &Client, prepared: Vec<Prepared>) -> bool {
    for message in prepared {
        if message.dest == 0 {
            return false;
        }
        if client
            .xpc_send(
                message.from,
                message.dest,
                message.kind,
                message.token,
                &message.body,
            )
            .is_err()
        {
            return false;
        }
    }
    true
}

fn fail_connection(connection: usize, error: u32) {
    push_event(Event::Disconnect {
        key: Some(connection),
        route: 0,
        error,
    });
}

fn send_message(connection: usize, message: usize, sink: Option<ReplySink>) {
    let prepared = match with_state(|state| prepare_send(state, connection, message, sink)) {
        Some(Ok(prepared)) => prepared,
        Some(Err(())) => {
            fail_connection(connection, broker::XPC_INVALID);
            return;
        }
        None => {
            fail_connection(connection, broker::XPC_INVALID);
            return;
        }
    };
    let broker = with_state(|state| state.broker.clone()).flatten();
    match (broker, prepared) {
        (Some(client), Some(message)) => {
            if !send_prepared(&client, vec![message]) {
                fail_connection(connection, broker::XPC_INVALID);
            }
        }
        (Some(_), None) => {}
        (None, _) => fail_connection(connection, broker::XPC_INVALID),
    }
}

fn flush_outbox(state: &mut State, connection: usize) -> Vec<Prepared> {
    let Some(conn) = connection_mut(state, connection) else {
        return Vec::new();
    };
    let from = conn.id;
    let peer = conn.peer;
    let pending: Vec<Outgoing> = conn.outbox.drain(..).collect();
    pending
        .into_iter()
        .map(|outgoing| Prepared {
            from,
            dest: outgoing.reply_to.unwrap_or(peer),
            kind: outgoing.kind,
            token: outgoing.token,
            body: outgoing.body,
        })
        .collect()
}

fn process_connect(connection: usize) {
    let info = with_state(|state| {
        let conn = connection_ref(state, connection)?;
        Some((conn.name.clone(), state.broker.clone()))
    })
    .flatten();
    let Some((name, broker)) = info else {
        return;
    };
    let Some(client) = broker else {
        fail_connection(connection, broker::XPC_INVALID);
        return;
    };
    let name = match name.to_str() {
        Ok(name) => name.to_owned(),
        Err(_) => {
            fail_connection(connection, broker::XPC_INVALID);
            return;
        }
    };
    match client.xpc_connect(&name) {
        Ok((0, id, peer)) => {
            let flushed = with_state(|state| {
                let cancelled = connection_ref(state, connection).map(|conn| conn.cancelled);
                if cancelled != Some(false) {
                    return Some((true, Vec::new()));
                }
                if let Some(conn) = connection_mut(state, connection) {
                    conn.id = id;
                    conn.peer = peer;
                    state.routes.insert(id, connection);
                }
                Some((false, flush_outbox(state, connection)))
            })
            .flatten()
            .unwrap_or((true, Vec::new()));
            if flushed.0 {
                let _ = client.xpc_close(id);
                return;
            }
            if !send_prepared(&client, flushed.1) {
                fail_connection(connection, broker::XPC_INVALID);
            }
        }
        Ok(_) | Err(_) => fail_connection(connection, broker::XPC_INVALID),
    }
}

fn process_accept(route: u64, peer: u64, name: String) {
    let prepared = with_state(|state| {
        let listener_key = state.objects.iter().find_map(|(key, object)| {
            let Value::Connection(connection) = &object.value else {
                return None;
            };
            (connection.listener
                && connection.active
                && !connection.cancelled
                && !connection.invalid
                && connection.name.to_str().ok() == Some(name.as_str()))
            .then_some(*key)
        })?;
        let listener = connection_ref(state, listener_key)?;
        let ready = listener.active && listener.suspended == 0 && listener.handler != 0;
        let handler = listener.handler;
        let name = CString::new(name).unwrap_or_default();
        let accepted = Connection {
            id: route,
            peer,
            name,
            listener: false,
            active: false,
            internal: true,
            suspended: 0,
            cancelled: false,
            invalid: false,
            handler: 0,
            target_queue: 0,
            mach_port: 0,
            pending: BTreeMap::new(),
            queue: VecDeque::new(),
            outbox: VecDeque::new(),
        };
        let key = new_object(state, Value::Connection(Box::new(accepted)));
        state.routes.insert(route, key);
        if ready {
            Some((Some(handler), key))
        } else {
            if let Some(listener) = connection_mut(state, listener_key) {
                listener.queue.push_back(Queued::Object(key));
            }
            Some((None, key))
        }
    })
    .flatten();
    if let Some((Some(handler), key)) = prepared {
        unsafe {
            blocks::invoke_one(handler as *mut c_void, key as *mut c_void);
        }
    }
}

fn reply_sink(sink: ReplySink, object: usize) {
    match sink {
        ReplySink::Handler(block) => {
            unsafe {
                blocks::invoke_one(block as *mut c_void, object as *mut c_void);
                blocks::release(block as *mut c_void);
            }
            with_state(|state| release_key(state, object, 0));
        }
        ReplySink::Sync(slot) => {
            if let Ok(mut value) = slot.0.lock() {
                *value = Some(object);
            }
            slot.1.notify_all();
        }
    }
}

fn error_sink(sink: ReplySink, code: u32) {
    match sink {
        ReplySink::Handler(block) => unsafe {
            blocks::invoke_one(block as *mut c_void, error_ptr(code) as *mut c_void);
            blocks::release(block as *mut c_void);
        },
        ReplySink::Sync(slot) => {
            if let Ok(mut value) = slot.0.lock() {
                *value = Some(error_ptr(code));
            }
            slot.1.notify_all();
        }
    }
}

fn process_deliver(dest: u64, kind: u32, reply: u64, token: u64, body: &[u8]) {
    let value = match decode_value(body) {
        Ok(value) => value,
        Err(_) => return,
    };
    let key = match with_state(|state| state.routes.get(&dest).copied()).flatten() {
        Some(key) => key,
        None => return,
    };
    let mut sink = None;
    let delivered = with_state(|state| {
        let (cancelled, id, handler, active, suspended) = {
            let connection = connection_mut(state, key)?;
            (
                connection.cancelled || connection.invalid,
                connection.id,
                connection.handler,
                connection.active,
                connection.suspended,
            )
        };
        if cancelled {
            if id != 0 {
                state.routes.remove(&id);
            }
            return Some(None);
        }
        if kind == broker::XPC_REPLY
            && let Some(connection) = connection_mut(state, key)
        {
            sink = connection.pending.remove(&token);
        }
        Some(Some((handler, active, suspended)))
    })
    .flatten();
    let Some(Some((handler, active, suspended))) = delivered else {
        return;
    };
    let object = with_state(|state| {
        let object = instantiate(state, &value);
        if let Some(entry) = state.objects.get_mut(&object) {
            entry.remote = Some(Remote {
                connection: key,
                route: reply,
                token,
            });
        }
        object
    });
    let Some(object) = object else {
        return;
    };
    if let Some(sink) = sink {
        reply_sink(sink, object);
        return;
    }
    if handler != 0 && active && suspended == 0 {
        unsafe {
            blocks::invoke_one(handler as *mut c_void, object as *mut c_void);
        }
        with_state(|state| release_key(state, object, 0));
    } else {
        with_state(|state| {
            if let Some(connection) = connection_mut(state, key) {
                connection.queue.push_back(Queued::Object(object));
            } else {
                release_key(state, object, 0);
            }
        });
    }
}

fn process_disconnect(connection: Option<usize>, route: u64, error: u32) {
    let key = match connection {
        Some(key) => key,
        None => match with_state(|state| state.routes.get(&route).copied()).flatten() {
            Some(key) => key,
            None => return,
        },
    };
    let pending = with_state(|state| {
        let (pending, queued, internal, close_port, id, handler) = {
            let conn = connection_mut(state, key)?;
            conn.invalid = true;
            conn.cancelled = true;
            let pending = std::mem::take(&mut conn.pending);
            let queued = std::mem::take(&mut conn.queue);
            let internal = conn.internal;
            conn.internal = false;
            let close_port = conn.mach_port;
            conn.mach_port = 0;
            let id = conn.id;
            conn.id = 0;
            (pending, queued, internal, close_port, id, conn.handler)
        };
        if id != 0 {
            state.routes.remove(&id);
        }
        for item in queued {
            let Queued::Object(object) = item;
            release_key(state, object, 0);
        }
        Some((pending, internal, close_port, handler))
    })
    .flatten();
    let Some((pending, internal, close_port, handler)) = pending else {
        return;
    };
    if close_port != 0 {
        mach::release_port(close_port);
    }
    for (_, sink) in pending {
        error_sink(sink, error);
    }
    if handler != 0 {
        unsafe {
            blocks::invoke_one(handler as *mut c_void, error_ptr(error) as *mut c_void);
        }
    }
    if internal {
        with_state(|state| release_key(state, key, 0));
    }
    push_event(Event::Wake);
}

fn process_wake() {
    let next = with_state(|state| {
        let mut chosen = None;
        let mut remaining = false;
        for (key, object) in state.objects.iter_mut() {
            let Value::Connection(connection) = &mut object.value else {
                continue;
            };
            if connection.handler == 0 || !connection.active || connection.suspended != 0 {
                continue;
            }
            let deliverable = match connection.queue.front() {
                Some(Queued::Object(_)) => !connection.invalid && !connection.cancelled,
                None => false,
            };
            if !deliverable {
                continue;
            }
            if chosen.is_none() {
                chosen = Some(*key);
            } else {
                remaining = true;
                break;
            }
        }
        let key = chosen?;
        let connection = connection_mut(state, key)?;
        let handler = connection.handler;
        let item = connection.queue.pop_front()?;
        if !remaining
            && !connection.invalid
            && !connection.cancelled
            && matches!(connection.queue.front(), Some(Queued::Object(_)))
        {
            remaining = true;
        }
        Some((handler, item, remaining))
    })
    .flatten();
    let Some((handler, item, remaining)) = next else {
        return;
    };
    let Queued::Object(object) = item;
    unsafe {
        blocks::invoke_one(handler as *mut c_void, object as *mut c_void);
    }
    with_state(|state| release_key(state, object, 0));
    if remaining {
        push_event(Event::Wake);
    }
}

fn activate_connection(connection: usize) {
    let info = with_state(|state| {
        let (listener, name, needs_retain, route) = {
            let conn = connection_mut(state, connection)?;
            if conn.active || conn.cancelled {
                return None;
            }
            conn.active = true;
            let needs_retain = !conn.internal;
            conn.internal = true;
            (conn.listener, conn.name.clone(), needs_retain, conn.id)
        };
        if needs_retain {
            retain_key(state, connection);
        }
        Some((listener, name, route))
    })
    .flatten();
    let Some((listener, name, route)) = info else {
        return;
    };
    ensure_dispatcher();
    if listener {
        let name = match name.to_str() {
            Ok(name) => name.to_owned(),
            Err(_) => {
                fail_connection(connection, broker::XPC_INVALID);
                return;
            }
        };
        match mach::register_service(&name) {
            Ok(port) => {
                with_state(|state| {
                    if let Some(conn) = connection_mut(state, connection) {
                        conn.mach_port = port;
                    }
                });
            }
            Err(_) => fail_connection(connection, broker::XPC_INVALID),
        }
    } else if route == 0 {
        push_event(Event::Connect(connection));
    }
}

fn cancel_connection(connection: usize) {
    let action = with_state(|state| {
        let (id, port, active) = {
            let conn = connection_mut(state, connection)?;
            if conn.cancelled {
                return None;
            }
            conn.cancelled = true;
            let id = conn.id;
            conn.id = 0;
            let port = conn.mach_port;
            conn.mach_port = 0;
            (id, port, conn.active)
        };
        if id != 0 {
            state.routes.remove(&id);
        }
        Some((id, port, active))
    })
    .flatten();
    let Some((id, port, active)) = action else {
        return;
    };
    if port != 0 {
        mach::release_port(port);
    }
    if id != 0
        && let Some(client) = with_state(|state| state.broker.clone()).flatten()
    {
        let _ = client.xpc_close(id);
    }
    if active {
        fail_connection(connection, broker::XPC_INVALID);
    }
}

unsafe extern "sysv64" fn xpc_retain(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        return object;
    }
    with_state(|state| retain_key(state, object as usize));
    object
}

unsafe extern "sysv64" fn xpc_release(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    with_state(|state| release_key(state, object as usize, 0));
}

unsafe extern "sysv64" fn xpc_get_type(object: *mut c_void) -> *mut c_void {
    let kind = with_state(|state| kind_of(state, object as usize)).flatten();
    match kind {
        Some(kind) => key_ptr(marker(kind)),
        None => key_ptr(addr(&TYPE_NULL)),
    }
}

unsafe extern "sysv64" fn xpc_copy(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        return std::ptr::null_mut();
    }
    let key = object as usize;
    if is_static(key) {
        return object;
    }
    let copied = with_state(|state| {
        let kind = kind_of(state, key)?;
        if matches!(kind, Kind::Connection) {
            retain_key(state, key);
            return Some(key);
        }
        let mut bytes = 0;
        let value = snapshot(state, key, 0, &mut bytes).ok()?;
        Some(instantiate(state, &value))
    })
    .flatten();
    copied.map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_equal(first: *mut c_void, second: *mut c_void) -> i32 {
    let (first, second) = (first as usize, second as usize);
    if first == second {
        return 1;
    }
    if first == 0 || second == 0 {
        return 0;
    }
    let equal = with_state(|state| {
        let (Some(a), Some(b)) = (kind_of(state, first), kind_of(state, second)) else {
            return false;
        };
        if matches!(a, Kind::Connection) || matches!(b, Kind::Connection) {
            return false;
        }
        if matches!(a, Kind::Error) || matches!(b, Kind::Error) {
            return false;
        }
        let mut first_bytes = 0;
        let mut second_bytes = 0;
        match (
            snapshot(state, first, 0, &mut first_bytes),
            snapshot(state, second, 0, &mut second_bytes),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    })
    .unwrap_or(false);
    i32::from(equal)
}

unsafe extern "sysv64" fn xpc_hash(object: *mut c_void) -> u64 {
    use std::hash::{Hash, Hasher};
    if object.is_null() {
        return 0;
    }
    let key = object as usize;
    if is_static(key) {
        return key as u64;
    }
    let encoded = with_state(|state| {
        if matches!(kind_of(state, key), Some(Kind::Connection)) {
            return Some(key.to_le_bytes().to_vec());
        }
        encode_object(state, key).ok()
    })
    .flatten();
    match encoded {
        Some(bytes) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            hasher.finish()
        }
        None => 0,
    }
}

fn describe(value: &Ser, out: &mut String) {
    match value {
        Ser::Null => out.push_str("<XPC null>"),
        Ser::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Ser::Int64(value) => out.push_str(&value.to_string()),
        Ser::Uint64(value) => out.push_str(&value.to_string()),
        Ser::Double(value) => out.push_str(&value.to_string()),
        Ser::String(text) => {
            out.push('"');
            out.push_str(text);
            out.push('"');
        }
        Ser::Data(data) => {
            out.push_str(&format!("<{} bytes of data>", data.len()));
        }
        Ser::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push_str(", ");
                }
                describe(item, out);
            }
            out.push(']');
        }
        Ser::Dictionary(pairs) => {
            out.push_str("{ ");
            for (index, (name, item)) in pairs.iter().enumerate() {
                if index != 0 {
                    out.push_str(", ");
                }
                out.push('"');
                out.push_str(name);
                out.push_str("\" => ");
                describe(item, out);
            }
            out.push_str(" }");
        }
        Ser::Error(code) => {
            out.push_str(error_text(*code));
        }
    }
}

unsafe extern "sysv64" fn xpc_copy_description(object: *mut c_void) -> *mut c_char {
    if object.is_null() {
        return std::ptr::null_mut();
    }
    let key = object as usize;
    let text = with_state(|state| {
        if let Some(code) = error_code(key) {
            return Some(format!("XPC error: {}", error_text(code)));
        }
        if let Some(value) = bool_static(key) {
            return Some(if value { "true" } else { "false" }.to_string());
        }
        let mut bytes = 0;
        let value = snapshot(state, key, 0, &mut bytes).ok()?;
        let mut out = String::new();
        describe(&value, &mut out);
        Some(out)
    })
    .flatten();
    let Some(text) = text else {
        return std::ptr::null_mut();
    };
    let allocation = crate::darwin::guest_alloc(text.len() + 1);
    if allocation.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), allocation, text.len());
        *allocation.add(text.len()) = 0;
    }
    allocation.cast()
}

unsafe extern "sysv64" fn xpc_null_create() -> *mut c_void {
    with_state(|state| new_object(state, Value::Null)).map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_bool_create(value: bool) -> *mut c_void {
    key_ptr(addr(if value { &BOOL_TRUE } else { &BOOL_FALSE }))
}

unsafe extern "sysv64" fn xpc_bool_get_value(object: *mut c_void) -> i32 {
    if object.is_null() {
        return 0;
    }
    let key = object as usize;
    let value = with_state(|state| {
        if let Some(value) = bool_static(key) {
            return Some(value);
        }
        match &state.objects.get(&key)?.value {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    })
    .flatten()
    .unwrap_or(false);
    i32::from(value)
}

macro_rules! scalar_create {
    ($name:ident, $value:ty, $variant:ident, $getter:ident) => {
        unsafe extern "sysv64" fn $name(value: $value) -> *mut c_void {
            with_state(|state| new_object(state, Value::$variant(value)))
                .map_or(std::ptr::null_mut(), key_ptr)
        }

        unsafe extern "sysv64" fn $getter(object: *mut c_void) -> $value {
            if object.is_null() {
                return 0 as $value;
            }
            with_state(
                |state| match &state.objects.get(&(object as usize))?.value {
                    Value::$variant(value) => Some(*value),
                    _ => None,
                },
            )
            .flatten()
            .unwrap_or(0 as $value)
        }
    };
}

scalar_create!(xpc_int64_create, i64, Int64, xpc_int64_get_value);
scalar_create!(xpc_uint64_create, u64, Uint64, xpc_uint64_get_value);
scalar_create!(xpc_double_create, f64, Double, xpc_double_get_value);

unsafe extern "sysv64" fn xpc_data_create(bytes: *const c_void, length: usize) -> *mut c_void {
    if bytes.is_null() && length != 0 {
        return std::ptr::null_mut();
    }
    if length > MAX_PAYLOAD {
        return std::ptr::null_mut();
    }
    let data = if length == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), length) }.to_vec()
    };
    with_state(|state| new_object(state, Value::Data(data))).map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_data_get_length(object: *mut c_void) -> usize {
    if object.is_null() {
        return 0;
    }
    with_state(
        |state| match &state.objects.get(&(object as usize))?.value {
            Value::Data(data) => Some(data.len()),
            _ => None,
        },
    )
    .flatten()
    .unwrap_or(0)
}

unsafe extern "sysv64" fn xpc_data_get_bytes_ptr(object: *mut c_void) -> *const c_void {
    if object.is_null() {
        return std::ptr::null();
    }
    with_state(
        |state| match &state.objects.get(&(object as usize))?.value {
            Value::Data(data) => Some(data.as_ptr().cast()),
            _ => None,
        },
    )
    .flatten()
    .unwrap_or(std::ptr::null())
}

unsafe extern "sysv64" fn xpc_data_get_bytes(
    object: *mut c_void,
    buffer: *mut c_void,
    offset: usize,
    length: usize,
) -> usize {
    if object.is_null() || buffer.is_null() {
        return 0;
    }
    with_state(|state| {
        let data = match &state.objects.get(&(object as usize))?.value {
            Value::Data(data) => data,
            _ => return None,
        };
        if offset >= data.len() {
            return Some(0);
        }
        let available = data.len() - offset;
        let count = length.min(available);
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr().add(offset), buffer.cast::<u8>(), count);
        }
        Some(count)
    })
    .flatten()
    .unwrap_or(0)
}

unsafe extern "sysv64" fn xpc_string_create(text: *const c_char) -> *mut c_void {
    let Some(text) = c_string(text) else {
        return std::ptr::null_mut();
    };
    let owned = match CString::new(text) {
        Ok(text) => text,
        Err(_) => return std::ptr::null_mut(),
    };
    with_state(|state| new_object(state, Value::String(owned)))
        .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_string_get_length(object: *mut c_void) -> usize {
    if object.is_null() {
        return 0;
    }
    with_state(
        |state| match &state.objects.get(&(object as usize))?.value {
            Value::String(text) => Some(text.as_bytes().len()),
            _ => None,
        },
    )
    .flatten()
    .unwrap_or(0)
}

unsafe extern "sysv64" fn xpc_string_get_string_ptr(object: *mut c_void) -> *const c_char {
    if object.is_null() {
        return std::ptr::null();
    }
    with_state(
        |state| match &state.objects.get(&(object as usize))?.value {
            Value::String(text) => Some(text.as_ptr()),
            _ => None,
        },
    )
    .flatten()
    .unwrap_or(std::ptr::null())
}

unsafe extern "sysv64" fn xpc_array_create(
    values: *const *mut c_void,
    count: usize,
) -> *mut c_void {
    if count != 0 && values.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|state| {
        let mut items = Vec::with_capacity(count.min(4096));
        for index in 0..count {
            let value = unsafe { *values.add(index) } as usize;
            if kind_of(state, value).is_some() {
                retain_key(state, value);
                items.push(value);
            } else {
                items.push(new_object(state, Value::Null));
            }
        }
        new_object(state, Value::Array(items))
    })
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_array_create_empty() -> *mut c_void {
    with_state(|state| new_object(state, Value::Array(Vec::new())))
        .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_array_set_value(
    array: *mut c_void,
    index: usize,
    value: *mut c_void,
) {
    if array.is_null() || value.is_null() {
        return;
    }
    array_set_value(array as usize, index, value as usize, true);
}

unsafe extern "sysv64" fn xpc_array_append_value(array: *mut c_void, value: *mut c_void) {
    if array.is_null() || value.is_null() {
        return;
    }
    array_append_value(array as usize, value as usize, true);
}

unsafe extern "sysv64" fn xpc_array_get_count(array: *mut c_void) -> usize {
    if array.is_null() {
        return 0;
    }
    with_state(|state| match &state.objects.get(&(array as usize))?.value {
        Value::Array(items) => Some(items.len()),
        _ => None,
    })
    .flatten()
    .unwrap_or(0)
}

unsafe extern "sysv64" fn xpc_array_get_value(array: *mut c_void, index: usize) -> *mut c_void {
    if array.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|state| array_get(state, array as usize, index))
        .flatten()
        .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_array_apply(array: *mut c_void, applier: *mut c_void) -> i32 {
    if array.is_null() || applier.is_null() {
        return 0;
    }
    let items = with_state(|state| {
        let items = match &state.objects.get(&(array as usize))?.value {
            Value::Array(items) => items.clone(),
            _ => return None,
        };
        for item in &items {
            retain_key(state, *item);
        }
        Some(items)
    })
    .flatten();
    let Some(items) = items else {
        return 0;
    };
    let mut completed = true;
    for (index, item) in items.iter().enumerate() {
        if !unsafe { blocks::invoke_two(applier, index, *item as *mut c_void) } {
            completed = false;
            break;
        }
    }
    with_state(|state| {
        for item in items {
            release_key(state, item, 0);
        }
    });
    i32::from(completed)
}

unsafe extern "sysv64" fn xpc_array_set_bool(array: *mut c_void, index: usize, value: bool) {
    if array.is_null() {
        return;
    }
    array_set_owned(array as usize, index, Value::Bool(value));
}

unsafe extern "sysv64" fn xpc_array_set_int64(array: *mut c_void, index: usize, value: i64) {
    if array.is_null() {
        return;
    }
    array_set_owned(array as usize, index, Value::Int64(value));
}

unsafe extern "sysv64" fn xpc_array_set_uint64(array: *mut c_void, index: usize, value: u64) {
    if array.is_null() {
        return;
    }
    array_set_owned(array as usize, index, Value::Uint64(value));
}

unsafe extern "sysv64" fn xpc_array_set_double(array: *mut c_void, index: usize, value: f64) {
    if array.is_null() {
        return;
    }
    array_set_owned(array as usize, index, Value::Double(value));
}

unsafe extern "sysv64" fn xpc_array_set_string(
    array: *mut c_void,
    index: usize,
    text: *const c_char,
) {
    let Some(text) = c_string(text) else {
        return;
    };
    if array.is_null() {
        return;
    }
    let value = match CString::new(text) {
        Ok(text) => text,
        Err(_) => return,
    };
    array_set_owned(array as usize, index, Value::String(value));
}

unsafe extern "sysv64" fn xpc_array_set_data(
    array: *mut c_void,
    index: usize,
    bytes: *const c_void,
    length: usize,
) {
    if array.is_null() || (bytes.is_null() && length != 0) || length > MAX_PAYLOAD {
        return;
    }
    let data = if length == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), length) }.to_vec()
    };
    array_set_owned(array as usize, index, Value::Data(data));
}

unsafe extern "sysv64" fn xpc_array_get_bool(array: *mut c_void, index: usize) -> i32 {
    if array.is_null() {
        return 0;
    }
    let value = with_state(|state| {
        let value = array_get(state, array as usize, index)?;
        match state.objects.get(&value).map(|object| &object.value) {
            Some(Value::Bool(value)) => Some(*value),
            _ => bool_static(value),
        }
    })
    .flatten()
    .unwrap_or(false);
    i32::from(value)
}

macro_rules! array_scalar_getter {
    ($name:ident, $value:ty, $variant:ident) => {
        unsafe extern "sysv64" fn $name(array: *mut c_void, index: usize) -> $value {
            if array.is_null() {
                return 0 as $value;
            }
            with_state(|state| {
                let value = array_get(state, array as usize, index)?;
                match &state.objects.get(&value)?.value {
                    Value::$variant(value) => Some(*value),
                    _ => None,
                }
            })
            .flatten()
            .unwrap_or(0 as $value)
        }
    };
}

array_scalar_getter!(xpc_array_get_int64, i64, Int64);
array_scalar_getter!(xpc_array_get_uint64, u64, Uint64);
array_scalar_getter!(xpc_array_get_double, f64, Double);

unsafe extern "sysv64" fn xpc_array_get_string(array: *mut c_void, index: usize) -> *const c_char {
    if array.is_null() {
        return std::ptr::null();
    }
    with_state(|state| {
        let value = array_get(state, array as usize, index)?;
        match &state.objects.get(&value)?.value {
            Value::String(text) => Some(text.as_ptr()),
            _ => None,
        }
    })
    .flatten()
    .unwrap_or(std::ptr::null())
}

unsafe extern "sysv64" fn xpc_array_get_data(
    array: *mut c_void,
    index: usize,
    length: *mut usize,
) -> *const c_void {
    if array.is_null() {
        return std::ptr::null();
    }
    with_state(|state| {
        let value = array_get(state, array as usize, index)?;
        match &state.objects.get(&value)?.value {
            Value::Data(data) => {
                if !length.is_null() {
                    unsafe {
                        *length = data.len();
                    }
                }
                Some(data.as_ptr().cast())
            }
            _ => None,
        }
    })
    .flatten()
    .unwrap_or(std::ptr::null())
}

unsafe extern "sysv64" fn xpc_array_get_array(array: *mut c_void, index: usize) -> *mut c_void {
    if array.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|state| {
        let value = array_get(state, array as usize, index)?;
        matches!(
            state.objects.get(&value).map(|object| &object.value),
            Some(Value::Array(_))
        )
        .then_some(value)
    })
    .flatten()
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_array_get_dictionary(
    array: *mut c_void,
    index: usize,
) -> *mut c_void {
    if array.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|state| {
        let value = array_get(state, array as usize, index)?;
        matches!(
            state.objects.get(&value).map(|object| &object.value),
            Some(Value::Dictionary(_))
        )
        .then_some(value)
    })
    .flatten()
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_create(
    keys: *const *const c_char,
    values: *const *mut c_void,
    count: usize,
) -> *mut c_void {
    if count != 0 && (keys.is_null() || values.is_null()) {
        return std::ptr::null_mut();
    }
    with_state(|state| {
        let mut map = BTreeMap::new();
        for index in 0..count {
            let key = unsafe { *keys.add(index) };
            let Some(name) = c_string(key) else {
                continue;
            };
            let value = unsafe { *values.add(index) } as usize;
            let child = if kind_of(state, value).is_some() {
                retain_key(state, value);
                value
            } else {
                new_object(state, Value::Null)
            };
            if let Some(old) = map.insert(name, child) {
                release_key(state, old, 0);
            }
        }
        new_object(state, Value::Dictionary(map))
    })
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_create_empty() -> *mut c_void {
    with_state(|state| new_object(state, Value::Dictionary(BTreeMap::new())))
        .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_create_reply(original: *mut c_void) -> *mut c_void {
    if original.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|state| {
        let remote = object_remote(state, original as usize)?;
        {
            let object = state.objects.get_mut(&(original as usize))?;
            if object.reply_used {
                return None;
            }
            object.reply_used = true;
        }
        let reply = new_object(state, Value::Dictionary(BTreeMap::new()));
        if let Some(object) = state.objects.get_mut(&reply) {
            object.remote = Some(remote);
        }
        Some(reply)
    })
    .flatten()
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_set_value(
    dict: *mut c_void,
    key: *const c_char,
    value: *mut c_void,
) {
    if dict.is_null() {
        return;
    }
    let Some(name) = c_string(key) else {
        return;
    };
    if value.is_null() {
        with_state(|state| dict_remove(state, dict as usize, &name));
        return;
    }
    set_dict_value(dict as usize, &name, value as usize, true);
}

unsafe extern "sysv64" fn xpc_dictionary_get_value(
    dict: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    if dict.is_null() {
        return std::ptr::null_mut();
    }
    let Some(name) = c_string(key) else {
        return std::ptr::null_mut();
    };
    with_state(|state| dict_get(state, dict as usize, &name))
        .flatten()
        .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_get_count(dict: *mut c_void) -> usize {
    if dict.is_null() {
        return 0;
    }
    with_state(|state| match &state.objects.get(&(dict as usize))?.value {
        Value::Dictionary(map) => Some(map.len()),
        _ => None,
    })
    .flatten()
    .unwrap_or(0)
}

unsafe extern "sysv64" fn xpc_dictionary_apply(dict: *mut c_void, applier: *mut c_void) -> i32 {
    if dict.is_null() || applier.is_null() {
        return 0;
    }
    let pairs = with_state(|state| {
        let pairs = match &state.objects.get(&(dict as usize))?.value {
            Value::Dictionary(map) => map
                .iter()
                .map(|(name, value)| (name.clone(), *value))
                .collect::<Vec<_>>(),
            _ => return None,
        };
        for (_, value) in &pairs {
            retain_key(state, *value);
        }
        Some(pairs)
    })
    .flatten();
    let Some(pairs) = pairs else {
        return 0;
    };
    let mut completed = true;
    for (name, value) in &pairs {
        let Ok(name) = CString::new(name.as_str()) else {
            completed = false;
            break;
        };
        if !unsafe { blocks::invoke_two(applier, name.as_ptr() as usize, *value as *mut c_void) } {
            completed = false;
            break;
        }
    }
    with_state(|state| {
        for (_, value) in pairs {
            release_key(state, value, 0);
        }
    });
    i32::from(completed)
}

unsafe extern "sysv64" fn xpc_dictionary_set_bool(
    dict: *mut c_void,
    key: *const c_char,
    value: bool,
) {
    let Some(name) = c_string(key) else {
        return;
    };
    if !dict.is_null() {
        dict_set_owned(dict as usize, &name, Value::Bool(value));
    }
}

unsafe extern "sysv64" fn xpc_dictionary_set_int64(
    dict: *mut c_void,
    key: *const c_char,
    value: i64,
) {
    let Some(name) = c_string(key) else {
        return;
    };
    if !dict.is_null() {
        dict_set_owned(dict as usize, &name, Value::Int64(value));
    }
}

unsafe extern "sysv64" fn xpc_dictionary_set_uint64(
    dict: *mut c_void,
    key: *const c_char,
    value: u64,
) {
    let Some(name) = c_string(key) else {
        return;
    };
    if !dict.is_null() {
        dict_set_owned(dict as usize, &name, Value::Uint64(value));
    }
}

unsafe extern "sysv64" fn xpc_dictionary_set_double(
    dict: *mut c_void,
    key: *const c_char,
    value: f64,
) {
    let Some(name) = c_string(key) else {
        return;
    };
    if !dict.is_null() {
        dict_set_owned(dict as usize, &name, Value::Double(value));
    }
}

unsafe extern "sysv64" fn xpc_dictionary_set_string(
    dict: *mut c_void,
    key: *const c_char,
    value: *const c_char,
) {
    let Some(name) = c_string(key) else {
        return;
    };
    let Some(value) = c_string(value) else {
        return;
    };
    if dict.is_null() {
        return;
    }
    let Ok(value) = CString::new(value) else {
        return;
    };
    dict_set_owned(dict as usize, &name, Value::String(value));
}

unsafe extern "sysv64" fn xpc_dictionary_set_data(
    dict: *mut c_void,
    key: *const c_char,
    bytes: *const c_void,
    length: usize,
) {
    let Some(name) = c_string(key) else {
        return;
    };
    if dict.is_null() || (bytes.is_null() && length != 0) || length > MAX_PAYLOAD {
        return;
    }
    let data = if length == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), length) }.to_vec()
    };
    dict_set_owned(dict as usize, &name, Value::Data(data));
}

unsafe extern "sysv64" fn xpc_dictionary_get_bool(dict: *mut c_void, key: *const c_char) -> i32 {
    if dict.is_null() {
        return 0;
    }
    let Some(name) = c_string(key) else {
        return 0;
    };
    let value = with_state(|state| {
        let value = dict_get(state, dict as usize, &name)?;
        match state.objects.get(&value).map(|object| &object.value) {
            Some(Value::Bool(value)) => Some(*value),
            _ => bool_static(value),
        }
    })
    .flatten()
    .unwrap_or(false);
    i32::from(value)
}

macro_rules! dict_scalar_getter {
    ($name:ident, $value:ty, $variant:ident) => {
        unsafe extern "sysv64" fn $name(dict: *mut c_void, key: *const c_char) -> $value {
            if dict.is_null() {
                return 0 as $value;
            }
            let Some(name) = c_string(key) else {
                return 0 as $value;
            };
            with_state(|state| {
                let value = dict_get(state, dict as usize, &name)?;
                match &state.objects.get(&value)?.value {
                    Value::$variant(value) => Some(*value),
                    _ => None,
                }
            })
            .flatten()
            .unwrap_or(0 as $value)
        }
    };
}

dict_scalar_getter!(xpc_dictionary_get_int64, i64, Int64);
dict_scalar_getter!(xpc_dictionary_get_uint64, u64, Uint64);
dict_scalar_getter!(xpc_dictionary_get_double, f64, Double);

unsafe extern "sysv64" fn xpc_dictionary_get_string(
    dict: *mut c_void,
    key: *const c_char,
) -> *const c_char {
    if dict.is_null() {
        return std::ptr::null();
    }
    let Some(name) = c_string(key) else {
        return std::ptr::null();
    };
    if error_code(dict as usize).is_some() {
        return if name == "XPCErrorDescription" {
            ERROR_KEY_TEXT.as_ptr().cast()
        } else {
            std::ptr::null()
        };
    }
    with_state(|state| {
        let value = dict_get(state, dict as usize, &name)?;
        match &state.objects.get(&value)?.value {
            Value::String(text) => Some(text.as_ptr()),
            _ => None,
        }
    })
    .flatten()
    .unwrap_or(std::ptr::null())
}

unsafe extern "sysv64" fn xpc_dictionary_get_data(
    dict: *mut c_void,
    key: *const c_char,
    length: *mut usize,
) -> *const c_void {
    if dict.is_null() {
        return std::ptr::null();
    }
    let Some(name) = c_string(key) else {
        return std::ptr::null();
    };
    with_state(|state| {
        let value = dict_get(state, dict as usize, &name)?;
        match &state.objects.get(&value)?.value {
            Value::Data(data) => {
                if !length.is_null() {
                    unsafe {
                        *length = data.len();
                    }
                }
                Some(data.as_ptr().cast())
            }
            _ => None,
        }
    })
    .flatten()
    .unwrap_or(std::ptr::null())
}

unsafe extern "sysv64" fn xpc_dictionary_get_array(
    dict: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    if dict.is_null() {
        return std::ptr::null_mut();
    }
    let Some(name) = c_string(key) else {
        return std::ptr::null_mut();
    };
    with_state(|state| {
        let value = dict_get(state, dict as usize, &name)?;
        matches!(
            state.objects.get(&value).map(|object| &object.value),
            Some(Value::Array(_))
        )
        .then_some(value)
    })
    .flatten()
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_get_dictionary(
    dict: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    if dict.is_null() {
        return std::ptr::null_mut();
    }
    let Some(name) = c_string(key) else {
        return std::ptr::null_mut();
    };
    with_state(|state| {
        let value = dict_get(state, dict as usize, &name)?;
        matches!(
            state.objects.get(&value).map(|object| &object.value),
            Some(Value::Dictionary(_))
        )
        .then_some(value)
    })
    .flatten()
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_dictionary_get_remote_connection(dict: *mut c_void) -> *mut c_void {
    if dict.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|state| object_remote(state, dict as usize).map(|remote| remote.connection))
        .flatten()
        .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_connection_create_mach_service(
    name: *const c_char,
    target_queue: *mut c_void,
    flags: u64,
) -> *mut c_void {
    let Some(name) = c_string(name) else {
        return std::ptr::null_mut();
    };
    if name.is_empty() || name.len() >= 128 {
        return std::ptr::null_mut();
    }
    let Ok(name) = CString::new(name) else {
        return std::ptr::null_mut();
    };
    let listener = flags & 1 != 0;
    with_state(|state| {
        let connection = Connection {
            id: 0,
            peer: 0,
            name,
            listener,
            active: false,
            internal: false,
            suspended: 0,
            cancelled: false,
            invalid: false,
            handler: 0,
            target_queue: target_queue as usize,
            mach_port: 0,
            pending: BTreeMap::new(),
            queue: VecDeque::new(),
            outbox: VecDeque::new(),
        };
        new_object(state, Value::Connection(Box::new(connection)))
    })
    .map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_connection_set_event_handler(
    connection: *mut c_void,
    handler: *mut c_void,
) {
    if connection.is_null() {
        return;
    }
    let copied = unsafe { blocks::copy(handler.cast_const()) } as usize;
    let previous = with_state(|state| {
        let conn = connection_mut(state, connection as usize)?;
        Some(std::mem::replace(&mut conn.handler, copied))
    })
    .flatten();
    if let Some(previous) = previous
        && previous != 0
        && previous != copied
    {
        unsafe {
            blocks::release(previous as *mut c_void);
        }
    }
    push_event(Event::Wake);
}

unsafe extern "sysv64" fn xpc_connection_set_target_queue(
    connection: *mut c_void,
    queue: *mut c_void,
) {
    if connection.is_null() {
        return;
    }
    with_state(|state| {
        if let Some(conn) = connection_mut(state, connection as usize) {
            conn.target_queue = queue as usize;
        }
    });
}

unsafe extern "sysv64" fn xpc_connection_activate(connection: *mut c_void) {
    if !connection.is_null() {
        activate_connection(connection as usize);
    }
}

unsafe extern "sysv64" fn xpc_connection_resume(connection: *mut c_void) {
    if connection.is_null() {
        return;
    }
    let active =
        with_state(|state| connection_ref(state, connection as usize).map(|conn| conn.active))
            .flatten()
            .unwrap_or(false);
    if !active {
        activate_connection(connection as usize);
        return;
    }
    let resumed = with_state(|state| {
        let conn = connection_mut(state, connection as usize)?;
        if conn.suspended > 0 {
            conn.suspended -= 1;
            return Some(true);
        }
        Some(false)
    })
    .flatten();
    if resumed == Some(true) {
        push_event(Event::Wake);
    }
}

unsafe extern "sysv64" fn xpc_connection_suspend(connection: *mut c_void) {
    if connection.is_null() {
        return;
    }
    with_state(|state| {
        if let Some(conn) = connection_mut(state, connection as usize) {
            conn.suspended = conn.suspended.saturating_add(1);
        }
    });
}

unsafe extern "sysv64" fn xpc_connection_cancel(connection: *mut c_void) {
    if !connection.is_null() {
        cancel_connection(connection as usize);
    }
}

unsafe extern "sysv64" fn xpc_connection_send_message(
    connection: *mut c_void,
    message: *mut c_void,
) {
    if connection.is_null() || message.is_null() {
        return;
    }
    send_message(connection as usize, message as usize, None);
}

unsafe extern "sysv64" fn xpc_connection_send_message_with_reply(
    connection: *mut c_void,
    message: *mut c_void,
    target_queue: *mut c_void,
    handler: *mut c_void,
) {
    let _ = target_queue;
    if connection.is_null() || message.is_null() || handler.is_null() {
        return;
    }
    let copied = unsafe { blocks::copy(handler.cast_const()) } as usize;
    send_message(
        connection as usize,
        message as usize,
        Some(ReplySink::Handler(copied)),
    );
}

unsafe extern "sysv64" fn xpc_connection_send_message_with_reply_sync(
    connection: *mut c_void,
    message: *mut c_void,
) -> *mut c_void {
    if connection.is_null() || message.is_null() {
        return key_ptr(error_ptr(broker::XPC_INVALID));
    }
    let slot = Arc::new((Mutex::new(None), Condvar::new()));
    send_message(
        connection as usize,
        message as usize,
        Some(ReplySink::Sync(slot.clone())),
    );
    let mut value = match slot.0.lock() {
        Ok(value) => value,
        Err(_) => return key_ptr(error_ptr(broker::XPC_INVALID)),
    };
    while value.is_none() {
        value = match slot.1.wait(value) {
            Ok(value) => value,
            Err(_) => return key_ptr(error_ptr(broker::XPC_INVALID)),
        };
    }
    (*value).map_or(std::ptr::null_mut(), key_ptr)
}

unsafe extern "sysv64" fn xpc_connection_get_name(connection: *mut c_void) -> *const c_char {
    if connection.is_null() {
        return std::ptr::null();
    }
    with_state(|state| connection_ref(state, connection as usize).map(connection_name_pointer))
        .flatten()
        .unwrap_or(std::ptr::null())
}

fn static_symbol(name: &str) -> Option<u64> {
    Some(match name {
        "__xpc_type_connection" => addr(&TYPE_CONNECTION),
        "__xpc_type_endpoint" => addr(&TYPE_ENDPOINT),
        "__xpc_type_null" => addr(&TYPE_NULL),
        "__xpc_type_bool" => addr(&TYPE_BOOL),
        "__xpc_type_int64" => addr(&TYPE_INT64),
        "__xpc_type_uint64" => addr(&TYPE_UINT64),
        "__xpc_type_double" => addr(&TYPE_DOUBLE),
        "__xpc_type_date" => addr(&TYPE_DATE),
        "__xpc_type_data" => addr(&TYPE_DATA),
        "__xpc_type_string" => addr(&TYPE_STRING),
        "__xpc_type_uuid" => addr(&TYPE_UUID),
        "__xpc_type_fd" => addr(&TYPE_FD),
        "__xpc_type_shmem" => addr(&TYPE_SHMEM),
        "__xpc_type_array" => addr(&TYPE_ARRAY),
        "__xpc_type_dictionary" => addr(&TYPE_DICTIONARY),
        "__xpc_type_error" => addr(&TYPE_ERROR),
        "__xpc_type_rich_error" => addr(&TYPE_RICH_ERROR),
        "__xpc_bool_true" => addr(&BOOL_TRUE),
        "__xpc_bool_false" => addr(&BOOL_FALSE),
        "__xpc_error_connection_invalid" => addr(&ERROR_INVALID),
        "__xpc_error_connection_interrupted" => addr(&ERROR_INTERRUPTED),
        "__xpc_error_key_description" => addr(&ERROR_KEY_POINTER),
        _ => return None,
    } as u64)
}

pub fn symbol(name: &str) -> Option<u64> {
    if let Some(value) = static_symbol(name) {
        return Some(value);
    }
    Some(match name {
        "_xpc_retain" => xpc_retain as *const () as u64,
        "_xpc_release" => xpc_release as *const () as u64,
        "_xpc_get_type" => xpc_get_type as *const () as u64,
        "_xpc_copy" => xpc_copy as *const () as u64,
        "_xpc_equal" => xpc_equal as *const () as u64,
        "_xpc_hash" => xpc_hash as *const () as u64,
        "_xpc_copy_description" => xpc_copy_description as *const () as u64,
        "_xpc_null_create" => xpc_null_create as *const () as u64,
        "_xpc_bool_create" => xpc_bool_create as *const () as u64,
        "_xpc_bool_get_value" => xpc_bool_get_value as *const () as u64,
        "_xpc_int64_create" => xpc_int64_create as *const () as u64,
        "_xpc_int64_get_value" => xpc_int64_get_value as *const () as u64,
        "_xpc_uint64_create" => xpc_uint64_create as *const () as u64,
        "_xpc_uint64_get_value" => xpc_uint64_get_value as *const () as u64,
        "_xpc_double_create" => xpc_double_create as *const () as u64,
        "_xpc_double_get_value" => xpc_double_get_value as *const () as u64,
        "_xpc_data_create" => xpc_data_create as *const () as u64,
        "_xpc_data_get_length" => xpc_data_get_length as *const () as u64,
        "_xpc_data_get_bytes_ptr" => xpc_data_get_bytes_ptr as *const () as u64,
        "_xpc_data_get_bytes" => xpc_data_get_bytes as *const () as u64,
        "_xpc_string_create" => xpc_string_create as *const () as u64,
        "_xpc_string_get_length" => xpc_string_get_length as *const () as u64,
        "_xpc_string_get_string_ptr" => xpc_string_get_string_ptr as *const () as u64,
        "_xpc_array_create" => xpc_array_create as *const () as u64,
        "_xpc_array_create_empty" => xpc_array_create_empty as *const () as u64,
        "_xpc_array_set_value" => xpc_array_set_value as *const () as u64,
        "_xpc_array_append_value" => xpc_array_append_value as *const () as u64,
        "_xpc_array_get_count" => xpc_array_get_count as *const () as u64,
        "_xpc_array_get_value" => xpc_array_get_value as *const () as u64,
        "_xpc_array_apply" => xpc_array_apply as *const () as u64,
        "_xpc_array_set_bool" => xpc_array_set_bool as *const () as u64,
        "_xpc_array_set_int64" => xpc_array_set_int64 as *const () as u64,
        "_xpc_array_set_uint64" => xpc_array_set_uint64 as *const () as u64,
        "_xpc_array_set_double" => xpc_array_set_double as *const () as u64,
        "_xpc_array_set_string" => xpc_array_set_string as *const () as u64,
        "_xpc_array_set_data" => xpc_array_set_data as *const () as u64,
        "_xpc_array_get_bool" => xpc_array_get_bool as *const () as u64,
        "_xpc_array_get_int64" => xpc_array_get_int64 as *const () as u64,
        "_xpc_array_get_uint64" => xpc_array_get_uint64 as *const () as u64,
        "_xpc_array_get_double" => xpc_array_get_double as *const () as u64,
        "_xpc_array_get_string" => xpc_array_get_string as *const () as u64,
        "_xpc_array_get_data" => xpc_array_get_data as *const () as u64,
        "_xpc_array_get_array" => xpc_array_get_array as *const () as u64,
        "_xpc_array_get_dictionary" => xpc_array_get_dictionary as *const () as u64,
        "_xpc_dictionary_create" => xpc_dictionary_create as *const () as u64,
        "_xpc_dictionary_create_empty" => xpc_dictionary_create_empty as *const () as u64,
        "_xpc_dictionary_create_reply" => xpc_dictionary_create_reply as *const () as u64,
        "_xpc_dictionary_set_value" => xpc_dictionary_set_value as *const () as u64,
        "_xpc_dictionary_get_value" => xpc_dictionary_get_value as *const () as u64,
        "_xpc_dictionary_get_count" => xpc_dictionary_get_count as *const () as u64,
        "_xpc_dictionary_apply" => xpc_dictionary_apply as *const () as u64,
        "_xpc_dictionary_set_bool" => xpc_dictionary_set_bool as *const () as u64,
        "_xpc_dictionary_set_int64" => xpc_dictionary_set_int64 as *const () as u64,
        "_xpc_dictionary_set_uint64" => xpc_dictionary_set_uint64 as *const () as u64,
        "_xpc_dictionary_set_double" => xpc_dictionary_set_double as *const () as u64,
        "_xpc_dictionary_set_string" => xpc_dictionary_set_string as *const () as u64,
        "_xpc_dictionary_set_data" => xpc_dictionary_set_data as *const () as u64,
        "_xpc_dictionary_get_bool" => xpc_dictionary_get_bool as *const () as u64,
        "_xpc_dictionary_get_int64" => xpc_dictionary_get_int64 as *const () as u64,
        "_xpc_dictionary_get_uint64" => xpc_dictionary_get_uint64 as *const () as u64,
        "_xpc_dictionary_get_double" => xpc_dictionary_get_double as *const () as u64,
        "_xpc_dictionary_get_string" => xpc_dictionary_get_string as *const () as u64,
        "_xpc_dictionary_get_data" => xpc_dictionary_get_data as *const () as u64,
        "_xpc_dictionary_get_array" => xpc_dictionary_get_array as *const () as u64,
        "_xpc_dictionary_get_dictionary" => xpc_dictionary_get_dictionary as *const () as u64,
        "_xpc_dictionary_get_remote_connection" => {
            xpc_dictionary_get_remote_connection as *const () as u64
        }
        "_xpc_connection_create_mach_service" => {
            xpc_connection_create_mach_service as *const () as u64
        }
        "_xpc_connection_set_event_handler" => xpc_connection_set_event_handler as *const () as u64,
        "_xpc_connection_set_target_queue" => xpc_connection_set_target_queue as *const () as u64,
        "_xpc_connection_activate" => xpc_connection_activate as *const () as u64,
        "_xpc_connection_resume" => xpc_connection_resume as *const () as u64,
        "_xpc_connection_suspend" => xpc_connection_suspend as *const () as u64,
        "_xpc_connection_cancel" => xpc_connection_cancel as *const () as u64,
        "_xpc_connection_send_message" => xpc_connection_send_message as *const () as u64,
        "_xpc_connection_send_message_with_reply" => {
            xpc_connection_send_message_with_reply as *const () as u64
        }
        "_xpc_connection_send_message_with_reply_sync" => {
            xpc_connection_send_message_with_reply_sync as *const () as u64
        }
        "_xpc_connection_get_name" => xpc_connection_get_name as *const () as u64,
        _ => return None,
    })
}
