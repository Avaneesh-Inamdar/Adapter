//! Optional guest address records for a Windows-side debugger.
use crate::{
    Result,
    inspect::ExportTarget,
    loader::link::{self, Program},
    platform::windows::Mapping,
};
use serde_json::json;
use std::{collections::BTreeMap, fs::OpenOptions, io::Write};

const MAX_SYMBOLS: usize = 16_384;

/// Flush mapped image addresses before any guest constructor or entry point.
/// Setting this environment variable explicitly opts into file I/O and failures.
pub fn record(plan: &Program, mappings: &[Mapping]) -> Result<()> {
    let Some(path) = std::env::var_os("ADAPTER_DEBUG_MAP") else {
        return Ok(());
    };
    if plan.images.len() != mappings.len() {
        return Err("debug map: image/mapping count mismatch".into());
    }
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("debug map: {e}"))?;
    for (index, (node, map)) in plan.images.iter().zip(mappings).enumerate() {
        let text = node
            .image
            .segments
            .iter()
            .find(|s| s.name == "__TEXT")
            .ok_or("debug map: no __TEXT segment")?;
        let header = map.relocated(text.vmaddr)?;
        let slide = i64::try_from(i128::from(header) - i128::from(text.vmaddr))
            .map_err(|_| "debug map: slide does not fit i64")?;
        let mut segments = Vec::new();
        for segment in &node.image.segments {
            if segment.name == "__PAGEZERO" || segment.vmsize == 0 {
                continue;
            }
            let start = map.relocated(segment.vmaddr)?;
            segments.push(json!({
                "name": segment.name,
                "vmaddr": segment.vmaddr,
                "size": segment.vmsize,
                "start": start,
                "end": start.checked_add(segment.vmsize).ok_or("debug map: segment overflow")?,
                "protection": if segment.flags & 0x10 != 0 { segment.initprot & !2 } else { segment.initprot },
            }));
        }
        let mut symbols = BTreeMap::new();
        let mut truncated = false;
        // Record local definitions when present. These labels are address
        // hints; nlist does not provide reliable function extents.
        for symbol in node.image.symbols.iter().filter(|s| !s.undefined) {
            if symbols.len() >= MAX_SYMBOLS {
                truncated = true;
                break;
            }
            if let Ok(address) = map.relocated(symbol.address) {
                let name: String = symbol.name.chars().take(2048).collect();
                symbols.insert((address, name), "nlist");
            }
        }
        for export in &node.image.exports {
            if symbols.len() >= MAX_SYMBOLS {
                truncated = true;
                break;
            }
            let address = match export.target {
                ExportTarget::Address { address } | ExportTarget::TlvDescriptor { address } => {
                    address
                }
                _ => continue,
            };
            if let Ok(address) = map.relocated(address) {
                let name: String = export.name.chars().take(2048).collect();
                symbols.insert((address, name), "export");
            }
        }
        let symbols: Vec<_> = symbols
            .into_iter()
            .map(|((address, name), source)| json!({"name": name, "address": address, "source": source}))
            .collect();
        let entry_vmaddr = link::entry(&node.image).ok();
        let entry = entry_vmaddr.and_then(|address| map.relocated(address).ok());
        let record = json!({
            "schema": 1,
            "event": "image",
            "pid": std::process::id(),
            "index": index,
            "path": node.path,
            "header": header,
            "vm_header": text.vmaddr,
            "slide": slide,
            "entry": entry,
            "entry_vmaddr": entry_vmaddr,
            "segments": segments,
            "symbols": symbols,
            "symbols_truncated": truncated,
        });
        serde_json::to_writer(&mut output, &record).map_err(|e| format!("debug map: {e}"))?;
        output
            .write_all(b"\n")
            .map_err(|e| format!("debug map: {e}"))?;
    }
    output.flush().map_err(|e| format!("debug map: {e}"))?;
    adapter_debug_mapped();
    Ok(())
}

/// Debuggers stop here after the JSONL records are flushed and before guest code.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn adapter_debug_mapped() {
    // Keep a unique observable body: an empty hook can be folded with unrelated
    // functions by the release linker's identical-code folding, stopping LLDB
    // before mapping instead of at this call.
    use std::sync::atomic::{AtomicU64, Ordering};
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}
