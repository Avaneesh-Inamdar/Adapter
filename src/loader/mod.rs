use crate::{
    Result,
    inspect::{self, Image},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
mod chained;
pub mod fixups;
pub mod link;
pub mod tls;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub sysroot: Option<PathBuf>,
    #[serde(default)]
    pub library_paths: Vec<PathBuf>,
}
impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("config: {e}"))
    }
}

pub fn discover(path: &Path) -> Result<PathBuf> {
    if !path.is_dir() {
        return std::fs::canonicalize(path)
            .map_err(|e| format!("discover {}: {e}", path.display()));
    }
    let contents = path.join("Contents");
    let plist = plist::Value::from_file(contents.join("Info.plist"))
        .map_err(|e| format!("bundle Info.plist: {e}"))?;
    let executable = plist
        .as_dictionary()
        .and_then(|d| d.get("CFBundleExecutable"))
        .and_then(|v| v.as_string())
        .ok_or("bundle has no string CFBundleExecutable")?;
    // The key names a single executable, never an arbitrary relative path.
    if executable.is_empty()
        || executable.contains(['/', '\\', ':'])
        || executable == "."
        || executable == ".."
    {
        return Err("invalid CFBundleExecutable filename".into());
    }
    let root = std::fs::canonicalize(contents.join("MacOS")).map_err(|e| e.to_string())?;
    let result = std::fs::canonicalize(root.join(executable)).map_err(|e| e.to_string())?;
    if !result.starts_with(root) || !result.is_file() {
        return Err("bundle executable escapes MacOS directory".into());
    }
    Ok(result)
}

pub fn adapter(name: &str) -> bool {
    matches!(
        name,
        "/usr/lib/libSystem.B.dylib"
            | "/usr/lib/libSystem.dylib"
            | "/usr/lib/libiconv.2.dylib"
            | "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"
            | "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
    )
}

#[derive(Debug, Serialize)]
pub struct Edge {
    pub from: PathBuf,
    pub install_name: String,
    pub status: String,
    pub resolved: Option<PathBuf>,
    pub weak: bool,
}
#[derive(Debug, Serialize)]
pub struct Graph {
    pub executable: PathBuf,
    pub nodes: Vec<PathBuf>,
    pub edges: Vec<Edge>,
    pub unresolved_required: usize,
    pub executable_by_current_runtime: bool,
}

fn expand(name: &str, owner: &Path, executable: &Path) -> Option<PathBuf> {
    if name == "@loader_path" {
        Some(owner.parent()?.to_path_buf())
    } else if name == "@executable_path" {
        Some(executable.parent()?.to_path_buf())
    } else if let Some(s) = name.strip_prefix("@loader_path/") {
        Some(owner.parent()?.join(s))
    } else if let Some(s) = name.strip_prefix("@executable_path/") {
        Some(executable.parent()?.join(s))
    } else if name.starts_with('@') {
        None
    } else {
        Some(PathBuf::from(name))
    }
}

pub fn resolve(
    name: &str,
    image: &Image,
    owner: &Path,
    executable: &Path,
    cfg: &Config,
    inherited_rpaths: &[PathBuf],
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(s) = name.strip_prefix("@rpath/") {
        for r in &image.rpaths {
            if let Some(p) = expand(r, owner, executable) {
                candidates.push(p.join(s));
            }
        }
        for r in inherited_rpaths {
            candidates.push(r.join(s));
        }
    } else if name.starts_with('/') {
        if let Some(root) = &cfg.sysroot {
            candidates.push(root.join(name.trim_start_matches('/')));
        }
    } else if let Some(p) = expand(name, owner, executable) {
        candidates.push(if p.is_absolute() {
            p
        } else {
            owner.parent()?.join(p)
        });
    }
    // Explicit search paths are opt-in, not an implicit current-directory lookup.
    for dir in &cfg.library_paths {
        if let Some(base) = Path::new(name).file_name() {
            candidates.push(dir.join(base));
        }
    }
    candidates.into_iter().find_map(|p| {
        if p.is_file() {
            std::fs::canonicalize(p).ok()
        } else {
            None
        }
    })
}

pub fn graph(executable: &Path, cfg: &Config) -> Result<Graph> {
    let executable = discover(executable)?;
    let mut g = Graph {
        executable: executable.clone(),
        nodes: Vec::new(),
        edges: Vec::new(),
        unresolved_required: 0,
        executable_by_current_runtime: true,
    };
    let mut pending = vec![(executable.clone(), Vec::<PathBuf>::new())];
    let mut seen = BTreeSet::new();
    while let Some((path, inherited)) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        if seen.len() > 256 {
            return Err("dependency graph exceeds 256 images".into());
        }
        let image =
            inspect::read(&path).map_err(|e| format!("dependency {}: {e}", path.display()))?;
        if image.architecture != "x86_64" || !image.unsupported.is_empty() {
            g.executable_by_current_runtime = false;
        }
        let mut rpaths: Vec<_> = image
            .rpaths
            .iter()
            .filter_map(|r| expand(r, &path, &executable))
            .collect();
        rpaths.extend(inherited.iter().cloned());
        for dep in &image.dependencies {
            let resolved = if adapter(&dep.name) {
                None
            } else {
                resolve(&dep.name, &image, &path, &executable, cfg, &inherited)
            };
            let status = if adapter(&dep.name) {
                "adapter (subset; symbols checked at bind)"
            } else if resolved.is_some() {
                "found Mach-O (symbols checked at preflight)"
            } else if dep.weak {
                "missing weak (execution unsupported)"
            } else {
                "unresolved required"
            };
            if resolved.is_none() && !adapter(&dep.name) {
                g.executable_by_current_runtime = false;
            }
            if resolved.is_none() && !adapter(&dep.name) && !dep.weak {
                g.unresolved_required += 1;
            }
            if let Some(p) = &resolved {
                pending.push((p.clone(), rpaths.clone()));
            }
            g.edges.push(Edge {
                from: path.clone(),
                install_name: dep.name.clone(),
                status: status.into(),
                resolved,
                weak: dep.weak,
            });
        }
        if path == executable && (image.filetype != 2 || image.entry_file_offset.is_none()) {
            g.executable_by_current_runtime = false;
        }
        if path != executable && image.filetype != 6 {
            g.executable_by_current_runtime = false;
        }
        g.nodes.push(path);
    }
    Ok(g)
}
