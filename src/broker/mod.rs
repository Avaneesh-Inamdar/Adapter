//! Loopback broker for Mach ports that span guest processes.
//! The broker owns the bootstrap namespace and forwards message payloads; it is
//! not a Mach kernel and performs no memory protection.
pub mod job;

use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub const MAGIC: u32 = 0x4144_4252;
pub const VERSION: u16 = 1;
pub const MAX_FRAME: usize = 16 * 1024 * 1024 + 4096;
pub const UNKNOWN_SERVICE: u32 = 1102;
pub const SERVICE_ACTIVE: u32 = 1103;
pub const INVALID_DEST: u32 = 0x1000_0003;

#[derive(Clone, Debug)]
pub enum Frame {
    Hello {
        magic: u32,
        version: u16,
        token: [u8; 16],
    },
    Welcome {
        conn: u64,
    },
    Denied {
        status: u32,
    },
    CheckIn {
        request: u64,
        name: String,
        port: u64,
    },
    CheckInReply {
        request: u64,
        status: u32,
    },
    LookUp {
        request: u64,
        name: String,
    },
    LookUpReply {
        request: u64,
        status: u32,
        port: u64,
    },
    Send {
        request: u64,
        dest: u64,
        id: i32,
        bits: u32,
        local: u64,
        body: Vec<u8>,
    },
    SendReply {
        request: u64,
        status: u32,
    },
    Deliver {
        dest: u64,
        id: i32,
        bits: u32,
        local: u64,
        body: Vec<u8>,
    },
    Retire {
        request: u64,
        port: u64,
    },
    RetireReply {
        request: u64,
    },
}

impl Frame {
    fn tag(&self) -> u8 {
        match self {
            Self::Hello { .. } => 1,
            Self::Welcome { .. } => 2,
            Self::Denied { .. } => 3,
            Self::CheckIn { .. } => 4,
            Self::CheckInReply { .. } => 5,
            Self::LookUp { .. } => 6,
            Self::LookUpReply { .. } => 7,
            Self::Send { .. } => 8,
            Self::SendReply { .. } => 9,
            Self::Deliver { .. } => 10,
            Self::Retire { .. } => 11,
            Self::RetireReply { .. } => 12,
        }
    }

    pub fn request(&self) -> Option<u64> {
        match self {
            Self::CheckInReply { request, .. }
            | Self::LookUpReply { request, .. }
            | Self::SendReply { request, .. }
            | Self::RetireReply { request } => Some(*request),
            _ => None,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.tag()];
        match self {
            Self::Hello {
                magic,
                version,
                token,
            } => {
                put_u32(&mut out, *magic);
                put_u16(&mut out, *version);
                out.extend_from_slice(token);
            }
            Self::Welcome { conn } => put_u64(&mut out, *conn),
            Self::Denied { status } => put_u32(&mut out, *status),
            Self::CheckIn {
                request,
                name,
                port,
            } => {
                put_u64(&mut out, *request);
                put_string(&mut out, name);
                put_u64(&mut out, *port);
            }
            Self::CheckInReply { request, status } => {
                put_u64(&mut out, *request);
                put_u32(&mut out, *status);
            }
            Self::LookUp { request, name } => {
                put_u64(&mut out, *request);
                put_string(&mut out, name);
            }
            Self::LookUpReply {
                request,
                status,
                port,
            } => {
                put_u64(&mut out, *request);
                put_u32(&mut out, *status);
                put_u64(&mut out, *port);
            }
            Self::Send {
                request,
                dest,
                id,
                bits,
                local,
                body,
            } => {
                put_u64(&mut out, *request);
                put_u64(&mut out, *dest);
                put_i32(&mut out, *id);
                put_u32(&mut out, *bits);
                put_u64(&mut out, *local);
                put_bytes(&mut out, body);
            }
            Self::SendReply { request, status } => {
                put_u64(&mut out, *request);
                put_u32(&mut out, *status);
            }
            Self::Deliver {
                dest,
                id,
                bits,
                local,
                body,
            } => {
                put_u64(&mut out, *dest);
                put_i32(&mut out, *id);
                put_u32(&mut out, *bits);
                put_u64(&mut out, *local);
                put_bytes(&mut out, body);
            }
            Self::Retire { request, port } => {
                put_u64(&mut out, *request);
                put_u64(&mut out, *port);
            }
            Self::RetireReply { request } => put_u64(&mut out, *request),
        }
        out
    }

    fn decode(tag: u8, data: &[u8]) -> Result<Self, String> {
        let mut r = Decoder { data, pos: 0 };
        let frame = match tag {
            1 => {
                let magic = r.u32()?;
                let version = r.u16()?;
                let token = r.bytes(16)?;
                let mut value = [0u8; 16];
                value.copy_from_slice(token);
                Self::Hello {
                    magic,
                    version,
                    token: value,
                }
            }
            2 => Self::Welcome { conn: r.u64()? },
            3 => Self::Denied { status: r.u32()? },
            4 => Self::CheckIn {
                request: r.u64()?,
                name: r.string()?,
                port: r.u64()?,
            },
            5 => Self::CheckInReply {
                request: r.u64()?,
                status: r.u32()?,
            },
            6 => Self::LookUp {
                request: r.u64()?,
                name: r.string()?,
            },
            7 => Self::LookUpReply {
                request: r.u64()?,
                status: r.u32()?,
                port: r.u64()?,
            },
            8 => Self::Send {
                request: r.u64()?,
                dest: r.u64()?,
                id: r.i32()?,
                bits: r.u32()?,
                local: r.u64()?,
                body: r.body()?,
            },
            9 => Self::SendReply {
                request: r.u64()?,
                status: r.u32()?,
            },
            10 => Self::Deliver {
                dest: r.u64()?,
                id: r.i32()?,
                bits: r.u32()?,
                local: r.u64()?,
                body: r.body()?,
            },
            11 => Self::Retire {
                request: r.u64()?,
                port: r.u64()?,
            },
            12 => Self::RetireReply { request: r.u64()? },
            _ => return Err(format!("unknown frame tag {tag}")),
        };
        if r.pos != data.len() {
            return Err("trailing bytes in frame".into());
        }
        Ok(frame)
    }
}

#[derive(Clone, Debug)]
pub struct Incoming {
    pub dest: u64,
    pub id: i32,
    pub bits: u32,
    pub local: u64,
    pub body: Vec<u8>,
}

type Reply = std::result::Result<Frame, ()>;
type Pending = Arc<Mutex<BTreeMap<u64, mpsc::Sender<Reply>>>>;

pub struct Client {
    stream: Mutex<TcpStream>,
    pending: Pending,
    next_request: AtomicU64,
    reader: Mutex<Option<JoinHandle<()>>>,
    lost: Arc<AtomicBool>,
}

impl Client {
    pub fn connect(
        endpoint: &Path,
        delivery: Arc<dyn Fn(Incoming) + Send + Sync>,
        lost: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(Arc<Self>, u64), String> {
        let endpoint_text = fs::read_to_string(endpoint)
            .map_err(|e| format!("mach broker endpoint {}: {e}", endpoint.display()))?;
        let endpoint_json: Value = serde_json::from_str(&endpoint_text)
            .map_err(|e| format!("mach broker endpoint: {e}"))?;
        let port: u16 = endpoint_json
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .ok_or("mach broker endpoint has no valid port")?;
        let token_hex = endpoint_json
            .get("token")
            .and_then(Value::as_str)
            .ok_or("mach broker endpoint has no token")?;
        let token = decode_hex(token_hex)?;
        if token.len() != 16 {
            return Err("mach broker token must be 16 bytes".into());
        }
        let mut token_bytes = [0u8; 16];
        token_bytes.copy_from_slice(&token);

        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .map_err(|e| format!("mach broker connect: {e}"))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("mach broker socket: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("mach broker socket: {e}"))?;
        write_frame(
            &mut stream,
            &Frame::Hello {
                magic: MAGIC,
                version: VERSION,
                token: token_bytes,
            },
        )
        .map_err(|e| format!("mach broker handshake: {e}"))?;
        let response = read_frame(&mut stream)
            .map_err(|e| format!("mach broker handshake: {e}"))?
            .ok_or("mach broker closed during handshake")?;
        let conn = match response {
            Frame::Welcome { conn } => conn,
            Frame::Denied { status } => {
                return Err(format!("mach broker denied connection (status {status})"));
            }
            _ => return Err("mach broker returned an unexpected handshake frame".into()),
        };
        stream
            .set_read_timeout(None)
            .map_err(|e| format!("mach broker socket: {e}"))?;
        let read_stream = stream
            .try_clone()
            .map_err(|e| format!("mach broker socket: {e}"))?;

        let pending: Pending = Arc::new(Mutex::new(BTreeMap::new()));
        let lost_flag = Arc::new(AtomicBool::new(false));
        let client = Arc::new(Self {
            stream: Mutex::new(stream),
            pending: pending.clone(),
            next_request: AtomicU64::new(1),
            reader: Mutex::new(None),
            lost: lost_flag.clone(),
        });
        let thread_pending = pending;
        let handle = thread::spawn(move || {
            reader_loop(read_stream, thread_pending, delivery);
            lost_flag.store(true, Ordering::SeqCst);
            lost();
        });
        if let Ok(mut slot) = client.reader.lock() {
            *slot = Some(handle);
        }
        Ok((client, conn))
    }

    fn request(&self, build: impl FnOnce(u64) -> Frame) -> Result<Frame, String> {
        if self.lost.load(Ordering::SeqCst) {
            return Err("mach broker disconnected".into());
        }
        let request = self.next_request.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(request, sender);
        } else {
            return Err("mach broker request table poisoned".into());
        }
        if self.lost.load(Ordering::SeqCst) {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&request);
            }
            return Err("mach broker disconnected".into());
        }
        let write = match self.stream.lock() {
            Ok(mut stream) => write_frame(&mut stream, &build(request)),
            Err(_) => Err(io::Error::other("mach broker stream poisoned")),
        };
        if let Err(error) = write {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&request);
            }
            return Err(format!("mach broker write: {error}"));
        }
        match receiver.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(())) => Err("mach broker disconnected".into()),
            Err(_) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&request);
                }
                Err("mach broker request timed out".into())
            }
        }
    }

    pub fn check_in(&self, name: &str, port: u64) -> Result<u32, String> {
        match self.request(|request| Frame::CheckIn {
            request,
            name: name.to_string(),
            port,
        })? {
            Frame::CheckInReply { status, .. } => Ok(status),
            _ => Err("mach broker returned the wrong check-in reply".into()),
        }
    }

    pub fn look_up(&self, name: &str) -> Result<(u32, u64), String> {
        match self.request(|request| Frame::LookUp {
            request,
            name: name.to_string(),
        })? {
            Frame::LookUpReply { status, port, .. } => Ok((status, port)),
            _ => Err("mach broker returned the wrong lookup reply".into()),
        }
    }

    pub fn send(
        &self,
        dest: u64,
        bits: u32,
        id: i32,
        local: u64,
        body: &[u8],
    ) -> Result<u32, String> {
        match self.request(|request| Frame::Send {
            request,
            dest,
            id,
            bits,
            local,
            body: body.to_vec(),
        })? {
            Frame::SendReply { status, .. } => Ok(status),
            _ => Err("mach broker returned the wrong send reply".into()),
        }
    }

    pub fn retire(&self, port: u64) -> Result<(), String> {
        match self.request(|request| Frame::Retire { request, port })? {
            Frame::RetireReply { .. } => Ok(()),
            _ => Err("mach broker returned the wrong retire reply".into()),
        }
    }

    pub fn close(&self) {
        if let Ok(stream) = self.stream.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Ok(mut slot) = self.reader.lock()
            && let Some(handle) = slot.take()
        {
            let _ = handle.join();
        }
    }
}

fn reader_loop(
    mut stream: TcpStream,
    pending: Pending,
    delivery: Arc<dyn Fn(Incoming) + Send + Sync>,
) {
    loop {
        match read_frame(&mut stream) {
            Ok(Some(Frame::Deliver {
                dest,
                id,
                bits,
                local,
                body,
            })) => delivery(Incoming {
                dest,
                id,
                bits,
                local,
                body,
            }),
            Ok(Some(frame)) => {
                if let Some(request) = frame.request()
                    && let Ok(mut pending) = pending.lock()
                    && let Some(sender) = pending.remove(&request)
                {
                    let _ = sender.send(Ok(frame));
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
    if let Ok(mut pending) = pending.lock() {
        for (_, sender) in std::mem::take(&mut *pending) {
            let _ = sender.send(Err(()));
        }
    }
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_string(out: &mut Vec<u8>, value: &str) {
    put_bytes(out, value.as_bytes());
}
fn put_bytes(out: &mut Vec<u8>, value: &[u8]) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value);
}

struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Decoder<'a> {
    fn bytes(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(count).ok_or("frame length overflow")?;
        if end > self.data.len() {
            return Err("truncated frame".into());
        }
        let value = &self.data[self.pos..end];
        self.pos = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        if len > 4096 {
            return Err("frame string exceeds 4096 bytes".into());
        }
        String::from_utf8(self.bytes(len)?.to_vec()).map_err(|_| "frame string is not UTF-8".into())
    }
    fn body(&mut self) -> Result<Vec<u8>, String> {
        let len = self.u32()? as usize;
        Ok(self.bytes(len)?.to_vec())
    }
}

pub fn write_frame(stream: &mut TcpStream, frame: &Frame) -> io::Result<()> {
    let payload = frame.encode();
    if payload.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame exceeds size limit",
        ));
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    stream.write_all(&out)
}

pub fn read_frame(stream: &mut TcpStream) -> io::Result<Option<Frame>> {
    let mut length = [0u8; 4];
    match stream.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds size limit",
        ));
    }
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload)?;
    if payload.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty frame"));
    }
    Frame::decode(payload[0], &payload[1..])
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("mach broker token has odd length".into());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for pair in bytes.as_chunks::<2>().0 {
        let high = (pair[0] as char)
            .to_digit(16)
            .ok_or("mach broker token is not hexadecimal")?;
        let low = (pair[1] as char)
            .to_digit(16)
            .ok_or("mach broker token is not hexadecimal")?;
        out.push((high * 16 + low) as u8);
    }
    Ok(out)
}
