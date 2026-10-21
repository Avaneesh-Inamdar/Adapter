//! `adapter-broker` owns the Mach bootstrap namespace shared by guest processes
//! and starts registered launchd-style jobs on demand.
use adapter::broker::{
    Frame, INVALID_DEST, MAGIC, SERVICE_ACTIVE, UNKNOWN_SERVICE, VERSION,
    job::{Job, host_path},
    read_frame, write_frame,
};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    hash::{BuildHasher, Hasher},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(15);

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    endpoint: PathBuf,
    launcher: PathBuf,
}

struct State {
    next_conn: u64,
    conns: HashMap<u64, mpsc::Sender<Frame>>,
    services: HashMap<String, (u64, u64)>,
    retired: HashSet<u64>,
    jobs: Vec<RuntimeJob>,
    job_services: HashMap<String, usize>,
}

struct RuntimeJob {
    config: Job,
    state: JobState,
}

enum JobState {
    Idle,
    Starting,
    Running { pid: u32 },
    Failed,
}

enum Activation {
    Inactive,
    InProgress,
}

fn run() -> Result<(), String> {
    let mut endpoint = None;
    let mut job_paths = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => {
                endpoint = Some(PathBuf::from(args.next().ok_or("--endpoint needs a file")?));
            }
            "--job" => job_paths.push(PathBuf::from(
                args.next().ok_or("--job needs a property list")?,
            )),
            "--help" | "-h" => {
                println!("adapter-broker --endpoint FILE [--job PLIST ...]");
                return Ok(());
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let endpoint = endpoint.ok_or("usage: adapter-broker --endpoint FILE [--job PLIST ...]")?;
    let jobs = load_jobs(&job_paths)?;
    let launcher = env::current_exe()
        .map_err(|e| format!("broker executable: {e}"))?
        .parent()
        .ok_or("broker executable has no directory")?
        .join(if cfg!(windows) {
            "adapter-run.exe"
        } else {
            "adapter-run"
        });
    if !jobs.is_empty() && !launcher.is_file() {
        return Err(format!(
            "launchd jobs need adapter-run next to the broker: {}",
            launcher.display()
        ));
    }
    let token = random_token();
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("bootstrap namespace bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("bootstrap namespace address: {e}"))?
        .port();
    write_endpoint(&endpoint, port, &token)?;
    eprintln!("adapter-broker: mach namespace on 127.0.0.1:{port}");
    let mut job_services = HashMap::new();
    for (index, job) in jobs.iter().enumerate() {
        for service in &job.config.services {
            job_services.insert(service.clone(), index);
        }
    }
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            next_conn: 1,
            conns: HashMap::new(),
            services: HashMap::new(),
            retired: HashSet::new(),
            jobs,
            job_services,
        }),
        changed: Condvar::new(),
        endpoint,
        launcher,
    });
    let run_at_load: Vec<usize> = shared
        .state
        .lock()
        .map_err(|_| "broker state poisoned".to_string())?
        .jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.config.run_at_load)
        .map(|(index, _)| index)
        .collect();
    for index in run_at_load {
        activate(&shared, index);
    }
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

fn load_jobs(paths: &[PathBuf]) -> Result<Vec<RuntimeJob>, String> {
    let mut jobs = Vec::new();
    let mut labels = HashSet::new();
    let mut services = HashSet::new();
    for path in paths {
        let config = Job::load(path)?;
        if !labels.insert(config.label.clone()) {
            return Err(format!("duplicate launchd job label {}", config.label));
        }
        for service in &config.services {
            if !services.insert(service.clone()) {
                return Err(format!("service {service} is advertised by two jobs"));
            }
        }
        jobs.push(RuntimeJob {
            config,
            state: JobState::Idle,
        });
    }
    Ok(jobs)
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

fn activate(shared: &Arc<Shared>, index: usize) -> bool {
    let job = {
        let Ok(mut state) = shared.state.lock() else {
            return false;
        };
        if !matches!(&state.jobs[index].state, JobState::Idle | JobState::Failed) {
            return false;
        }
        state.jobs[index].state = JobState::Starting;
        state.jobs[index].config.clone()
    };
    spawn_job(shared, index, job);
    true
}

fn spawn_job(shared: &Arc<Shared>, index: usize, job: Job) {
    let stdout = match open_log(job.stdout.as_deref()) {
        Ok(file) => file,
        Err(error) => return fail(shared, index, error),
    };
    let stderr = match open_log(job.stderr.as_deref()) {
        Ok(file) => file,
        Err(error) => return fail(shared, index, error),
    };
    let mut command = Command::new(&shared.launcher);
    command
        .arg(&job.program)
        .arg("--")
        .args(&job.arguments)
        .env("ADAPTER_MACH_BROKER", &shared.endpoint)
        .stdin(Stdio::null())
        .stdout(stdout.map_or_else(Stdio::null, Stdio::from))
        .stderr(stderr.map_or_else(Stdio::null, Stdio::from));
    for (name, value) in &job.environment {
        command.env(name, value);
    }
    eprintln!(
        "adapter-broker: spawning {} ({} {})",
        job.label,
        job.program,
        job.arguments.join(" ")
    );
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return fail(shared, index, format!("spawn {}: {error}", job.program)),
    };
    let pid = child.id();
    if let Ok(mut state) = shared.state.lock()
        && matches!(&state.jobs[index].state, JobState::Starting)
    {
        state.jobs[index].state = JobState::Running { pid };
    }
    shared.changed.notify_all();
    let shared = shared.clone();
    thread::spawn(move || {
        let mut child = child;
        let status = child.wait();
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        if matches!(&state.jobs[index].state, JobState::Running { pid: running } if *running == pid)
        {
            state.jobs[index].state = match &status {
                Ok(status) => {
                    eprintln!(
                        "adapter-broker: job {} exited ({status})",
                        state.jobs[index].config.label
                    );
                    JobState::Idle
                }
                Err(error) => {
                    eprintln!("adapter-broker: wait for job: {error}");
                    JobState::Failed
                }
            };
        }
        drop(state);
        shared.changed.notify_all();
    });
}

fn fail(shared: &Arc<Shared>, index: usize, error: String) {
    eprintln!("adapter-broker: job activation failed: {error}");
    if let Ok(mut state) = shared.state.lock() {
        state.jobs[index].state = JobState::Failed;
    }
    shared.changed.notify_all();
}

fn open_log(path: Option<&str>) -> Result<Option<fs::File>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(host_path(path))
        .map(Some)
        .map_err(|e| format!("open log {path}: {e}"))
}

fn look_up(shared: &Arc<Shared>, name: &str) -> (u32, u64) {
    let deadline = Instant::now() + ACTIVATION_TIMEOUT;
    let Ok(mut state) = shared.state.lock() else {
        return (UNKNOWN_SERVICE, 0);
    };
    let mut activates = 0;
    let mut observed_running = false;
    loop {
        if let Some((owner, port)) = state.services.get(name).copied() {
            if state.conns.contains_key(&owner) && !state.retired.contains(&port) {
                return (0, port);
            }
            state.services.remove(name);
            state.retired.insert(port);
        }
        let Some(index) = state.job_services.get(name).copied() else {
            return (UNKNOWN_SERVICE, 0);
        };
        let activation = match &state.jobs[index].state {
            JobState::Idle | JobState::Failed => Activation::Inactive,
            JobState::Running { .. } => {
                observed_running = true;
                Activation::InProgress
            }
            JobState::Starting => Activation::InProgress,
        };
        match activation {
            Activation::Inactive if activates < 2 && (activates == 0 || observed_running) => {
                drop(state);
                if activate(shared, index) {
                    observed_running = true;
                }
                activates += 1;
                let Ok(next) = shared.state.lock() else {
                    return (UNKNOWN_SERVICE, 0);
                };
                state = next;
                continue;
            }
            Activation::Inactive => return (UNKNOWN_SERVICE, 0),
            Activation::InProgress => {}
        }
        let now = Instant::now();
        if now >= deadline {
            return (UNKNOWN_SERVICE, 0);
        }
        let Ok((next, _)) = shared.changed.wait_timeout(state, deadline - now) else {
            return (UNKNOWN_SERVICE, 0);
        };
        state = next;
    }
}

fn handle_connection(
    mut stream: TcpStream,
    shared: &Arc<Shared>,
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
    shared.changed.notify_all();
    drop(sender);
    let _ = writer_thread.join();
    Ok(())
}

fn dispatch(frame: Frame, conn: u64, shared: &Arc<Shared>, sender: &mpsc::Sender<Frame>) -> bool {
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
            shared.changed.notify_all();
            reply(sender, Frame::CheckInReply { request, status })
        }
        Frame::LookUp { request, name } => {
            let (status, port) = look_up(shared, &name);
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
            shared.changed.notify_all();
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
