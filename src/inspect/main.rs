use adapter::{Result, inspect, loader};
use std::path::PathBuf;
fn run() -> Result<()> {
    let mut json = false;
    let mut deps = false;
    let mut path = None;
    let mut cfg = loader::Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--json" => json = true,
            "--deps" => deps = true,
            "--config" => {
                cfg = loader::Config::read(&PathBuf::from(
                    args.next().ok_or("--config needs a path")?,
                ))?
            }
            "--help" | "-h" => {
                println!("adapter-inspect [--json] [--deps] [--config FILE] MACH-O|APP");
                return Ok(());
            }
            _ if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ => {
                if path.replace(PathBuf::from(a)).is_some() {
                    return Err("only one input accepted".into());
                }
            }
        }
    }
    let path =
        loader::discover(&path.ok_or("usage: adapter-inspect [--json] [--deps] MACH-O|APP")?)?;
    if deps {
        println!(
            "{}",
            serde_json::to_string_pretty(&loader::graph(&path, &cfg)?).unwrap()
        );
    } else {
        let image = inspect::read(&path)?;
        println!(
            "{}",
            if json {
                serde_json::to_string_pretty(&image).unwrap()
            } else {
                inspect::report(&image)
            }
        );
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("adapter-inspect: {e}");
        std::process::exit(2);
    }
}
