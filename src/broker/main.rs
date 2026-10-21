//! `adapter-broker` owns the Mach bootstrap namespace shared by guest processes.
use adapter::broker::{
    Frame, INVALID_DEST, MAGIC, SERVICE_ACTIVE, UNKNOWN_SERVICE, VERSION, read_frame, write_frame,
};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    hash::{BuildHasher, Hasher},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

struct Shared {
    state: Mutex<State>,
}

struct State {
    next_conn: u64,
    conns: HashMap<u64, mpsc::Sender<Frame>>,
    services: HashMap<String, (u64, u64)>,
    retired: HashSet<u64>,
}

fn run() -> Result<(), String> {
    let mut endpoint = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => {
                endpoint = Some(PathBuf::from(args.next().ok_or("--endpoint needs a file")?));
            }
            "--help" | "-h" => {
                println!("adapter-broker --endpoint FILE");
                return Ok(());
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let endpoint = endpoint.ok_or("usage: adapter-broker --endpoint FILE")?;
    let token = random_token();
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("bootstrap namespace bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("bootstrap namespace address: {e}"))?
        .port();
    write_endpoint(&endpoint, port, &token)?;
    eprintln!("adapter-broker: mach namespace on 127.0.0.1:{port}");
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            next_conn: 1,
            conns: HashMap::new(),
            services: HashMap::new(),
            retired: HashSet::new(),
        }),
    });
    for stream in listener.incoming() {
        let stream = stream.map_err(|e| format!("bootstrap namespace accept: {e}"))?;
        let shared = shared.clone();
        thread::spawn(move || {
            if let Err(error) = handle_connection(stream, &shared, token) {
                eprintln!("adapter-broker: connection: {error}");
            }
        });
    }
    Ok(())
}

fn write_endpoint(path: &PathBuf, port: u16, token: &[u8; 16]) -> Result<(), String> {
    let text = format!(
        "{{\"port\":{port},\"token\":\"{}\"}}\n",
        token
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, text)
        .map_err(|e| format!("write endpoint {}: {e}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|e| format!("publish endpoint {}: {e}", path.display()))
}

fn random_token() -> [u8; 16] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = process::id() as u128;
    let mut first = std::collections::hash_map::RandomState::new().build_hasher();
    first.write_u128(time);
    first.write_u128(pid);
    let mut second = std::collections::hash_map::RandomState::new().build_hasher();
    second.write_u128(!time);
    second.write_u128(pid ^ 0x9e37_79b9_7f4a_7c15);
    let mut token = [0u8; 16];
    token[..8].copy_from_slice(&first.finish().to_le_bytes());
    token[8..].copy_from_slice(&second.finish().to_le_bytes());
    token
}

fn handle_connection(
    mut stream: TcpStream,
    shared: &Shared,
    token: [u8; 16],
) -> Result<(), String> {
    stream
        .set_nodelay(true)
        .map_err(|e| format!("socket: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("socket: {e}"))?;
    let hello = match read_frame(&mut stream) {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(()),
        Err(error) => return Err(format!("handshake: {error}")),
    };
    let accepted = matches!(
        hello,
        Frame::Hello {
            magic,
            version,
            token: supplied,
        } if magic == MAGIC && version == VERSION && supplied == token
    );
    if !accepted {
        let _ = write_frame(&mut stream, &Frame::Denied { status: 1 });
        return Ok(());
    }
    let (sender, receiver) = mpsc::channel();
    let conn = {
        let mut state = shared
            .state
            .lock()
            .map_err(|_| "broker state poisoned".to_string())?;
        let conn = state.next_conn;
        state.next_conn += 1;
        state.conns.insert(conn, sender.clone());
        conn
    };
    let mut writer = stream.try_clone().map_err(|e| format!("socket: {e}"))?;
    let writer_thread = thread::spawn(move || {
        for frame in receiver {
            if write_frame(&mut writer, &frame).is_err() {
                break;
            }
        }
    });
    if sender.send(Frame::Welcome { conn }).is_err() {
        return Ok(());
    }
    stream
        .set_read_timeout(None)
        .map_err(|e| format!("socket: {e}"))?;
    while let Ok(Some(frame)) = read_frame(&mut stream) {
        if !dispatch(frame, conn, shared, &sender) {
            break;
        }
    }
    {
        let mut state = shared
            .state
            .lock()
            .map_err(|_| "broker state poisoned".to_string())?;
        state.conns.remove(&conn);
        let dead: Vec<u64> = state
            .services
            .iter()
            .filter(|(_, (owner, _))| *owner == conn)
            .map(|(_, (_, port))| *port)
            .collect();
        state.services.retain(|_, (owner, _)| *owner != conn);
        for port in dead {
            state.retired.insert(port);
        }
    }
    drop(sender);
    let _ = writer_thread.join();
    Ok(())
}

fn dispatch(frame: Frame, conn: u64, shared: &Shared, sender: &mpsc::Sender<Frame>) -> bool {
    match frame {
        Frame::CheckIn {
            request,
            name,
            port,
        } => {
            if name.is_empty() || name.len() >= 128 || port == 0 {
                return reply(sender, Frame::CheckInReply { request, status: 1 });
            }
            let status = match shared.state.lock() {
                Ok(mut state) => match state.services.get(&name) {
                    Some((owner, existing))
                        if state.conns.contains_key(owner) && !state.retired.contains(existing) =>
                    {
                        SERVICE_ACTIVE
                    }
                    _ => {
                        state.services.insert(name, (conn, port));
                        0
                    }
                },
                Err(_) => 1,
            };
            reply(sender, Frame::CheckInReply { request, status })
        }
        Frame::LookUp { request, name } => {
            let (status, port) = match shared.state.lock() {
                Ok(mut state) => match state.services.get(&name) {
                    Some((owner, port))
                        if state.conns.contains_key(owner) && !state.retired.contains(port) =>
                    {
                        (0, *port)
                    }
                    Some(_) => {
                        state.services.remove(&name);
                        (UNKNOWN_SERVICE, 0)
                    }
                    None => (UNKNOWN_SERVICE, 0),
                },
                Err(_) => (UNKNOWN_SERVICE, 0),
            };
            reply(
                sender,
                Frame::LookUpReply {
                    request,
                    status,
                    port,
                },
            )
        }
        Frame::Send {
            request,
            dest,
            id,
            bits,
            local,
            body,
        } => {
            let dest_conn = dest >> 32;
            let destination = if dest == 0 || dest_conn == 0 {
                None
            } else {
                match shared.state.lock() {
                    Ok(state) => {
                        if state.retired.contains(&dest) {
                            None
                        } else {
                            state.conns.get(&dest_conn).cloned()
                        }
                    }
                    Err(_) => None,
                }
            };
            let status = match destination {
                Some(target) => {
                    if target
                        .send(Frame::Deliver {
                            dest,
                            id,
                            bits,
                            local,
                            body,
                        })
                        .is_ok()
                    {
                        0
                    } else {
                        INVALID_DEST
                    }
                }
                None => INVALID_DEST,
            };
            reply(sender, Frame::SendReply { request, status })
        }
        Frame::Retire { request, port } => {
            if let Ok(mut state) = shared.state.lock() {
                state.retired.insert(port);
                state.services.retain(|_, (_, service)| *service != port);
            }
            reply(sender, Frame::RetireReply { request })
        }
        Frame::Welcome { .. }
        | Frame::Denied { .. }
        | Frame::CheckInReply { .. }
        | Frame::LookUpReply { .. }
        | Frame::SendReply { .. }
        | Frame::Deliver { .. }
        | Frame::RetireReply { .. }
        | Frame::Hello { .. } => false,
    }
}

fn reply(sender: &mpsc::Sender<Frame>, frame: Frame) -> bool {
    sender.send(frame).is_ok()
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("adapter-broker: {error}");
            process::exit(125);
        }
    }
}
