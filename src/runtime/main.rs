use adapter::{
    Result,
    loader::{self, Config},
    runtime,
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn run() -> Result<i32> {
    let original: Vec<String> = std::env::args().skip(1).collect();
    let mut args = original.iter();
    let (mut worker, mut check, mut trace) = (false, false, false);
    let mut timeout = 10u64;
    let mut cfg = Config::default();
    let mut path = None;
    let mut guest = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--worker" => worker = true,
            "--check" => check = true,
            "--trace" => trace = true,
            "--config" => {
                cfg = Config::read(&PathBuf::from(args.next().ok_or("--config needs a file")?))?
            }
            "--timeout" => {
                timeout = args
                    .next()
                    .ok_or("--timeout needs seconds")?
                    .parse()
                    .map_err(|_| "invalid timeout")?;
                if timeout == 0 || timeout > 3600 {
                    return Err("timeout must be 1..3600 seconds".into());
                }
            }
            "--help" | "-h" => {
                println!(
                    "adapter-run [--check] [--trace] [--timeout SECONDS] [--config FILE] MACH-O|APP [-- GUEST_ARGS...]"
                );
                return Ok(0);
            }
            "--" => {
                guest.extend(args.cloned());
                break;
            }
            _ if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ => {
                if path.replace(PathBuf::from(a)).is_some() {
                    return Err("separate guest arguments with --".into());
                }
            }
        }
    }
    let path = path.ok_or("usage: adapter-run [--check] MACH-O|APP [-- ARGS...]")?;
    if check {
        let plan = runtime::preflight(&path, &cfg)?;
        println!(
            "preflight passed: {} fixups, {} images; execution not attempted",
            plan.images.iter().map(|i| i.fixups.len()).sum::<usize>(),
            plan.images.len()
        );
        return Ok(0);
    }
    if worker {
        return runtime::execute(&path, &cfg, &guest, trace);
    }
    loader::discover(&path)?;
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--worker")
        .args(original)
        .stdin(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return status
                .code()
                .ok_or("guest worker terminated without an exit code".into());
        }
        if start.elapsed() >= Duration::from_secs(timeout) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("execute: worker timed out after {timeout}s"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("adapter-run: {e}");
            std::process::exit(125);
        }
    }
}
