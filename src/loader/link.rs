//! Resolve a complete two-level link plan before mapping or guest callbacks.
use super::{
    Config, Graph,
    fixups::{self, Fixup, Kind},
};
use crate::{
    Result,
    inspect::{self, ExportTarget, Image},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub enum DependencyTarget {
    Adapter(String),
    Image(usize),
}
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Image { index: usize, address: u64 },
    Absolute(u64),
}
pub struct LinkedImage {
    pub path: PathBuf,
    pub image: Image,
    pub fixups: Vec<Fixup>,
    pub bindings: BTreeMap<usize, Value>,
    pub dependencies: Vec<DependencyTarget>,
}
pub struct Program {
    pub images: Vec<LinkedImage>,
    pub init_order: Vec<usize>,
}

pub fn adapter_symbol(library: &str, symbol: &str) -> Option<u64> {
    match library {
        "/usr/lib/libSystem.B.dylib" | "/usr/lib/libSystem.dylib" => crate::darwin::symbol(symbol),
        /* libiconv is a dependency of many Rust programs; their imports resolve
         * through libSystem, so this adapter currently exposes no symbols. */
        "/usr/lib/libiconv.2.dylib" => None,
        "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation"
        | "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation" => {
            crate::frameworks::corefoundation::symbol(symbol)
        }
        _ => None,
    }
}

pub fn mapped_range(i: &Image, address: u64, size: u64) -> bool {
    address.checked_add(size).is_some_and(|end| {
        i.segments.iter().any(|s| {
            s.name != "__PAGEZERO"
                && s.initprot & 1 != 0
                && address >= s.vmaddr
                && s.vmaddr
                    .checked_add(s.vmsize)
                    .is_some_and(|last| end <= last)
        })
    })
}

pub fn layout(i: &Image) -> Result<(u64, u64)> {
    let segments: Vec<_> = i
        .segments
        .iter()
        .filter(|s| s.name != "__PAGEZERO" && s.vmsize != 0)
        .collect();
    let low = segments
        .iter()
        .map(|s| s.vmaddr)
        .min()
        .ok_or("no loadable segments")?;
    let mut high = low;
    for (idx, s) in segments.iter().enumerate() {
        let end = s
            .vmaddr
            .checked_add(s.vmsize)
            .ok_or("segment address overflow")?;
        high = high.max(end);
        inspect::range(&i.data, s.fileoff, s.filesize)?;
        if s.filesize > s.vmsize || !s.vmaddr.is_multiple_of(4096) {
            return Err("invalid segment size/alignment".into());
        }
        if s.initprot & !7 != 0 || s.initprot & !s.maxprot != 0 || s.initprot & 6 == 6 {
            return Err("invalid or writable-executable segment protection".into());
        }
        let rounded = end.checked_add(4095).ok_or("segment page overflow")? & !4095;
        for t in segments.iter().take(idx) {
            let other = t
                .vmaddr
                .checked_add(t.vmsize)
                .and_then(|v| v.checked_add(4095))
                .ok_or("segment page overflow")?
                & !4095;
            if s.vmaddr < other && t.vmaddr < rounded {
                return Err("overlapping VM segment pages".into());
            }
        }
    }
    let len = high - low;
    if len == 0 || len > 256 * 1024 * 1024 {
        return Err("VM span exceeds 256 MiB limit".into());
    }
    let header = i
        .segments
        .iter()
        .find(|s| s.vmaddr == low && s.fileoff == 0 && s.filesize >= 32 && s.initprot & 1 != 0);
    if header.is_none() {
        return Err("image base must map the Mach-O header".into());
    }
    Ok((low, len))
}

pub fn entry(i: &Image) -> Result<u64> {
    let off = i.entry_file_offset.ok_or("no LC_MAIN")?;
    i.segments
        .iter()
        .find(|s| s.initprot & 4 != 0 && off >= s.fileoff && off - s.fileoff < s.filesize)
        .map(|s| s.vmaddr + off - s.fileoff)
        .ok_or("entry is not in an executable file-backed segment".into())
}

fn dependency_value(
    p: &Program,
    target: &DependencyTarget,
    name: &str,
    seen: &mut BTreeSet<(usize, String)>,
) -> Result<Option<Value>> {
    match target {
        DependencyTarget::Adapter(library) => {
            Ok(adapter_symbol(library, name).map(Value::Absolute))
        }
        DependencyTarget::Image(index) => export_value(p, *index, name, seen),
    }
}
fn export_value(
    p: &Program,
    index: usize,
    name: &str,
    seen: &mut BTreeSet<(usize, String)>,
) -> Result<Option<Value>> {
    if seen.len() >= 256 {
        return Err("export indirection exceeds 256 entries".into());
    }
    let key = (index, name.to_string());
    if !seen.insert(key.clone()) {
        return Err(format!("cyclic export resolution for {name}"));
    }
    let result = (|| {
        let node = &p.images[index];
        if let Ok(export_index) = node
            .image
            .exports
            .binary_search_by(|e| e.name.as_str().cmp(name))
        {
            let e = &node.image.exports[export_index];
            if e.weak {
                return Err(format!("weak export {name} requires coalescing"));
            }
            return match &e.target {
                ExportTarget::Address { address } => {
                    if !mapped_range(&node.image, *address, 1) {
                        return Err(format!("export {name} points outside readable image"));
                    }
                    Ok(Some(Value::Image {
                        index,
                        address: *address,
                    }))
                }
                ExportTarget::Absolute { value } => Ok(Some(Value::Absolute(*value))),
                ExportTarget::Reexport {
                    library,
                    name: import,
                } => {
                    let import = if import.is_empty() { name } else { import };
                    if library.is_empty() {
                        export_value(p, index, import, seen)
                    } else {
                        let ordinal = node
                            .image
                            .dependencies
                            .iter()
                            .position(|d| d.name == *library)
                            .ok_or("reexport library missing from dependencies")?;
                        dependency_value(p, &node.dependencies[ordinal], import, seen)
                    }
                }
                ExportTarget::TlvDescriptor { address } => {
                    if !node.image.sections.iter().any(|s| {
                        s.flags & 0xff == 0x13
                            && *address >= s.address
                            && address
                                .checked_add(24)
                                .is_some_and(|v| v <= s.address + s.size)
                    }) {
                        return Err(format!("invalid TLV descriptor export {name}"));
                    }
                    Ok(Some(Value::Image {
                        index,
                        address: *address,
                    }))
                }
                ExportTarget::Resolver => Err(format!("resolver export {name} unsupported")),
            };
        }
        // LC_REEXPORT_DYLIB makes the dependency's public namespace visible.
        for (ordinal, dep) in node.image.dependencies.iter().enumerate() {
            if dep.command == 0x8000001f
                && let Some(v) = dependency_value(p, &node.dependencies[ordinal], name, seen)?
            {
                return Ok(Some(v));
            }
        }
        Ok(None)
    })();
    seen.remove(&key);
    result
}

pub fn bind_value(
    p: &Program,
    index: usize,
    symbol: &str,
    ordinal: i32,
    addend: i64,
) -> Result<Value> {
    let node = &p.images[index];
    let target = node
        .dependencies
        .get(usize::try_from(ordinal - 1).map_err(|_| "invalid dylib ordinal")?)
        .ok_or("invalid dylib ordinal")?;
    let value = dependency_value(p, target, symbol, &mut BTreeSet::new())?.ok_or_else(|| {
        format!(
            "unimplemented symbol {symbol} from {}",
            node.image.dependencies[(ordinal - 1) as usize].name
        )
    })?;
    match value {
        Value::Absolute(v) => {
            if addend != 0 && matches!(target, DependencyTarget::Adapter(_)) {
                return Err("nonzero adapter addend unsupported".into());
            }
            Ok(Value::Absolute(
                v.checked_add_signed(addend)
                    .ok_or("absolute symbol addend overflow")?,
            ))
        }
        Value::Image { index, address } => {
            let address = address
                .checked_add_signed(addend)
                .ok_or("symbol addend overflow")?;
            if !mapped_range(&p.images[index].image, address, 1) {
                return Err("symbol addend exceeds provider image".into());
            }
            Ok(Value::Image { index, address })
        }
    }
}

fn order(p: &Program) -> Result<Vec<usize>> {
    fn visit(p: &Program, index: usize, state: &mut [u8], out: &mut Vec<usize>) -> Result<()> {
        if state[index] == 2 {
            return Ok(());
        }
        if state[index] == 1 {
            return Err(format!(
                "dependency cycle involving {} is unsupported",
                p.images[index].path.display()
            ));
        }
        state[index] = 1;
        for d in &p.images[index].dependencies {
            if let DependencyTarget::Image(child) = d {
                visit(p, *child, state, out)?;
            }
        }
        state[index] = 2;
        out.push(index);
        Ok(())
    }
    let mut out = Vec::new();
    visit(p, 0, &mut vec![0; p.images.len()], &mut out)?;
    Ok(out)
}

pub fn prepare(path: &Path, cfg: &Config) -> Result<Program> {
    let graph = super::graph(path, cfg)?;
    if graph.unresolved_required != 0 {
        return Err(format!(
            "resolve: {} required dependencies unresolved: {}",
            graph.unresolved_required,
            serde_json::to_string(&graph.edges).unwrap()
        ));
    }
    let mut p = Program {
        images: Vec::new(),
        init_order: Vec::new(),
    };
    let mut total = 0u64;
    for (index, path) in graph.nodes.iter().enumerate() {
        let image = inspect::read(path)?;
        if image.architecture != "x86_64"
            || image.cpu_subtype & 0xffffff != 3
            || !image.unsupported.is_empty()
        {
            return Err(format!(
                "preflight {}: unsupported architecture or metadata: {}",
                path.display(),
                image.unsupported.join(", ")
            ));
        }
        if (index == 0 && (image.filetype != 2 || image.flags & 0x200000 == 0))
            || (index != 0 && image.filetype != 6)
        {
            return Err(format!(
                "preflight {}: requires PIE executable or MH_DYLIB dependency",
                path.display()
            ));
        }
        if image.flags & 0x80 == 0 || image.flags & 0x100 != 0 {
            return Err("flat namespace unsupported".into());
        }
        let (_, span) = layout(&image)?;
        total = total
            .checked_add(span + image.data.len() as u64)
            .ok_or("aggregate image size overflow")?;
        if total > 512 * 1024 * 1024 {
            return Err("aggregate images exceed 512 MiB limit".into());
        }
        let dependencies = dependency_targets(&graph, path, &image)?;
        let fixups = fixups::plan(&image).map_err(|e| format!("fixups {}: {e}", path.display()))?;
        super::tls::plan(&image)?;
        for s in &image.sections {
            let n = match s.flags & 0xff {
                9 | 10 => 8,
                0x16 => 4,
                _ => continue,
            };
            if !s.size.is_multiple_of(n) {
                return Err("invalid initializer/finalizer section size".into());
            }
        }
        p.images.push(LinkedImage {
            path: path.clone(),
            image,
            fixups,
            dependencies,
            bindings: BTreeMap::new(),
        });
    }
    entry(&p.images[0].image)?;
    p.init_order = order(&p)?;
    for idx in 0..p.images.len() {
        let mut values = BTreeMap::new();
        for (slot, f) in p.images[idx].fixups.iter().enumerate() {
            if let Kind::Bind {
                symbol,
                ordinal,
                addend,
            } = &f.kind
            {
                let value = bind_value(&p, idx, symbol, *ordinal, *addend)
                    .map_err(|e| format!("bind {}: {e}", p.images[idx].path.display()))?;
                values.insert(slot, value);
            }
        }
        p.images[idx].bindings = values;
    }
    Ok(p)
}
fn dependency_targets(g: &Graph, path: &Path, image: &Image) -> Result<Vec<DependencyTarget>> {
    let edges: Vec<_> = g.edges.iter().filter(|e| e.from == path).collect();
    if edges.len() != image.dependencies.len() {
        return Err("dependency graph changed during preflight".into());
    }
    edges
        .iter()
        .map(|e| {
            if super::adapter(&e.install_name) {
                Ok(DependencyTarget::Adapter(e.install_name.clone()))
            } else if let Some(path) = &e.resolved {
                Ok(DependencyTarget::Image(
                    g.nodes
                        .iter()
                        .position(|p| p == path)
                        .ok_or("resolved image missing from graph")?,
                ))
            } else {
                Err(format!(
                    "missing weak library {} unsupported",
                    e.install_name
                ))
            }
        })
        .collect()
}
