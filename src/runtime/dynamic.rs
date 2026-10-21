//! Runtime `dlopen` state. Loaded programs are kept mapped for the rest of the
//! process; `dlclose` runs image destructors but never unmaps code because the
//! first slice does not track every pointer guests may have retained.
use super::native::LoadedProgram;
use crate::{
    Result,
    loader::{Config, link},
};
use std::{
    collections::BTreeMap,
    ffi::c_char,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct ImageRecord {
    pub path: PathBuf,
    pub header: u64,
    pub slide: i64,
    pub exports: BTreeMap<String, u64>,
    pub executable: Vec<(u64, u64)>,
}

struct Group {
    handle: usize,
    root: Option<usize>,
    images: Vec<usize>,
    adapters: Vec<String>,
    global: bool,
    first: bool,
    refs: usize,
    program: usize,
    finalized: bool,
    active: bool,
}

struct State {
    cfg: Config,
    trace: bool,
    argc: i32,
    argv: usize,
    envp: usize,
    apple: usize,
    images: Vec<ImageRecord>,
    groups: Vec<Group>,
    handles: BTreeMap<usize, usize>,
    paths: BTreeMap<PathBuf, usize>,
    adapters: BTreeMap<String, usize>,
    next_handle: usize,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub struct Session;

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(mut state) = STATE.lock() {
            *state = None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn begin(
    cfg: Config,
    records: Vec<ImageRecord>,
    order: Vec<usize>,
    adapters: Vec<String>,
    argc: i32,
    argv: usize,
    envp: usize,
    apple: usize,
    trace: bool,
) -> Result<Session> {
    if order.first() != Some(&0) || records.is_empty() {
        return Err("dynamic registry: invalid main image order".into());
    }
    let mut state = STATE.lock().map_err(|_| "dynamic registry poisoned")?;
    if state.is_some() {
        return Err("concurrent guest execution in one process unsupported".into());
    }
    *state = Some(State {
        cfg,
        trace,
        argc,
        argv,
        envp,
        apple,
        images: records,
        groups: vec![Group {
            handle: 0,
            root: Some(0),
            images: order,
            adapters,
            global: true,
            first: false,
            refs: 1,
            program: 0,
            finalized: false,
            active: true,
        }],
        handles: BTreeMap::new(),
        paths: BTreeMap::new(),
        adapters: BTreeMap::new(),
        next_handle: 1,
    });
    Ok(Session)
}

fn resolve_path(name: &str, cfg: &Config) -> Result<PathBuf> {
    if name.starts_with('@') {
        return Err(format!(
            "dlopen: @rpath/@loader_path paths are unsupported: {name}"
        ));
    }
    let mut candidates = Vec::new();
    let leaf = Path::new(name).file_name().map(PathBuf::from);
    if name.starts_with('/') {
        if let Some(root) = &cfg.sysroot {
            candidates.push(root.join(name.trim_start_matches('/')));
        }
    } else {
        let path = Path::new(name);
        if path.is_absolute() || name.contains(['/', '\\']) {
            candidates.push(path.to_path_buf());
        } else {
            if let Ok(cwd) = std::env::current_dir() {
                candidates.push(cwd.join(name));
            }
            if let Some(root) = &cfg.sysroot {
                candidates.push(root.join("usr/lib").join(name));
            }
        }
    }
    // Apple's dyld consults DYLD_LIBRARY_PATH for the leaf name even when the
    // caller supplied a path; `library_paths` is the opt-in equivalent here.
    if let Some(leaf) = leaf {
        for directory in &cfg.library_paths {
            candidates.push(directory.join(&leaf));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .and_then(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(|| format!("dlopen: image not found: {name}"))
}

fn image_loaded(state: &State, path: &Path) -> Option<usize> {
    for group in &state.groups {
        if !group.active {
            continue;
        }
        for &id in &group.images {
            if state.images[id].path == path {
                return Some(id);
            }
        }
    }
    None
}

pub fn load(name: &str, mode: i32) -> Result<usize> {
    let mut guard = STATE.lock().map_err(|_| "dynamic registry poisoned")?;
    let state = guard
        .as_mut()
        .ok_or("dlopen: dynamic loading outside guest execution")?;
    let trace = state.trace;
    let global = mode & super::RTLD_LOCAL == 0;
    let first = mode & super::RTLD_FIRST != 0;
    let noload = mode & super::RTLD_NOLOAD != 0;

    if crate::loader::adapter(name) {
        if let Some(&index) = state.adapters.get(name) {
            state.groups[index].refs += 1;
            return Ok(state.groups[index].handle);
        }
        if noload {
            return Err(format!("dlopen: image not loaded: {name}"));
        }
        let handle = state.next_handle;
        state.next_handle += 1;
        let index = state.groups.len();
        state.groups.push(Group {
            handle,
            root: None,
            images: Vec::new(),
            adapters: vec![name.to_string()],
            global,
            first,
            refs: 1,
            program: 0,
            finalized: false,
            active: true,
        });
        state.adapters.insert(name.to_string(), index);
        state.handles.insert(handle, index);
        return Ok(handle);
    }

    let path = resolve_path(name, &state.cfg)?;
    if let Some(&index) = state.paths.get(&path) {
        state.groups[index].refs += 1;
        return Ok(state.groups[index].handle);
    }
    if let Some(id) = image_loaded(state, &path) {
        return Err(format!(
            "dlopen: {} is already loaded as a dependency; handle aliasing unsupported",
            state.images[id].path.display()
        ));
    }
    if noload {
        return Err(format!("dlopen: image not loaded: {name}"));
    }

    let plan = link::prepare_library(&path, &state.cfg)?;
    for dependency in plan.images.iter().skip(1) {
        if image_loaded(state, &dependency.path).is_some() {
            return Err(format!(
                "dlopen: dependency {} is already loaded; shared dylibs across load groups are unsupported",
                dependency.path.display()
            ));
        }
    }
    let loaded = Box::new(LoadedProgram::load(plan, trace)?);
    let records = loaded.records()?;
    let order = loaded.search_order();
    let adapters = loaded.adapter_names();
    let ranges = records
        .iter()
        .flat_map(|record| record.executable.iter().copied())
        .collect();
    // Refuse to publish a handle unless guest callbacks will accept code in
    // the new images.
    crate::darwin::callbacks::add_ranges(ranges)?;

    let handle = state.next_handle;
    state.next_handle += 1;
    let base = state.images.len();
    let image_ids: Vec<usize> = order.iter().map(|index| base + index).collect();
    let dyld_images = records
        .iter()
        .map(|record| {
            (
                record.path.display().to_string(),
                record.header,
                record.slide,
            )
        })
        .collect();
    state.images.extend(records);
    let index = state.groups.len();
    let program = Box::into_raw(loaded) as usize;
    state.groups.push(Group {
        handle,
        root: Some(base),
        images: image_ids,
        adapters,
        global,
        first,
        refs: 1,
        program,
        finalized: false,
        active: true,
    });
    state.paths.insert(path, index);
    state.handles.insert(handle, index);
    let (argc, argv, envp, apple) = (state.argc, state.argv, state.envp, state.apple);
    drop(guard);

    crate::darwin::dyld::append(dyld_images);
    let program = unsafe { &*(program as *const LoadedProgram) };
    program.initialize(
        argc,
        argv as *const *const c_char,
        envp as *const *const c_char,
        apple as *const *const c_char,
        trace,
    );
    Ok(handle)
}

fn search_group(state: &State, group: &Group, symbol: &str) -> Option<u64> {
    if let Some(root) = group.root {
        if let Some(&value) = state.images[root].exports.get(symbol) {
            return Some(value);
        }
        if !group.first {
            for &id in &group.images {
                if id != root
                    && let Some(&value) = state.images[id].exports.get(symbol)
                {
                    return Some(value);
                }
            }
        }
    }
    if !group.first || group.root.is_none() {
        for library in &group.adapters {
            if let Some(value) = link::adapter_symbol(library, symbol) {
                return Some(value);
            }
        }
    }
    None
}

pub fn symbol(handle: isize, name: &str) -> Result<Option<u64>> {
    let symbol = format!("_{name}");
    let guard = STATE.lock().map_err(|_| "dynamic registry poisoned")?;
    let state = guard
        .as_ref()
        .ok_or("dlsym: dynamic loading outside guest execution")?;
    match handle {
        -2 => Ok(state
            .groups
            .iter()
            .filter(|group| group.active && group.global)
            .find_map(|group| search_group(state, group, &symbol))),
        -5 => Ok(state.images[0].exports.get(&symbol).copied()),
        -1 | -3 => Err(format!("dlsym: special handle {handle} unsupported")),
        value if value > 0 => {
            let index = *state
                .handles
                .get(&(value as usize))
                .ok_or("dlsym: invalid handle")?;
            let group = &state.groups[index];
            if !group.active {
                return Err("dlsym: invalid handle".into());
            }
            Ok(search_group(state, group, &symbol))
        }
        _ => Err("dlsym: invalid handle".into()),
    }
}

pub fn close(handle: isize) -> Result<()> {
    if handle <= 0 {
        return Err("dlclose: invalid handle".into());
    }
    let (program, trace, finalized, dsos) = {
        let mut guard = STATE.lock().map_err(|_| "dynamic registry poisoned")?;
        let state = guard
            .as_mut()
            .ok_or("dlclose: dynamic loading outside guest execution")?;
        let index = *state
            .handles
            .get(&(handle as usize))
            .ok_or("dlclose: invalid handle")?;
        let dsos: Vec<usize> = state.groups[index]
            .images
            .iter()
            .rev()
            .map(|&id| state.images[id].header as usize)
            .collect();
        let group = &mut state.groups[index];
        if group.refs > 1 {
            group.refs -= 1;
            return Ok(());
        }
        let program = group.program;
        let finalized = std::mem::replace(&mut group.finalized, true);
        group.refs = 0;
        group.active = false;
        state.handles.remove(&(handle as usize));
        state.paths.retain(|_, value| *value != index);
        state.adapters.retain(|_, value| *value != index);
        (program, state.trace, finalized, dsos)
    };
    if program != 0 && !finalized {
        // Apple compilers register C/C++ attribute destructors through
        // `__cxa_atexit` with the image base as the DSO handle.
        for dso in dsos {
            crate::darwin::callbacks::finalize(dso)?;
        }
        let program = unsafe { &*(program as *const LoadedProgram) };
        program.finalize(trace);
    }
    Ok(())
}

pub fn finalize_all() -> Result<()> {
    let mut pending = Vec::new();
    {
        let mut guard = STATE.lock().map_err(|_| "dynamic registry poisoned")?;
        let Some(state) = guard.as_mut() else {
            return Ok(());
        };
        for group in state.groups.iter_mut().rev() {
            if group.program != 0 && !std::mem::replace(&mut group.finalized, true) {
                pending.push((group.program, state.trace));
            }
        }
    }
    for (program, trace) in pending {
        let program = unsafe { &*(program as *const LoadedProgram) };
        program.finalize(trace);
    }
    Ok(())
}
