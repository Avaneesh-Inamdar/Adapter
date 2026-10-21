//! On-disk TLV layout validation, independent of the host's TLS implementation.
use crate::{Result, inspect::Image};
#[derive(Debug, Clone)]
pub struct Layout {
    pub address: u64,
    pub size: u64,
    pub alignment: usize,
    pub descriptors: Vec<u64>,
}
pub fn plan(i: &Image) -> Result<Option<Layout>> {
    let content: Vec<_> = i
        .sections
        .iter()
        .filter(|s| [0x11, 0x12].contains(&(s.flags & 0xff)) && s.size != 0)
        .collect();
    let descriptors: Vec<_> = i
        .sections
        .iter()
        .filter(|s| s.flags & 0xff == 0x13)
        .collect();
    if content.is_empty() && descriptors.is_empty() {
        return Ok(None);
    }
    let low = content
        .iter()
        .map(|s| s.address)
        .min()
        .ok_or("TLV descriptors have no template")?;
    let high = content
        .iter()
        .map(|s| s.address.checked_add(s.size).ok_or("TLS address overflow"))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap();
    let size = high - low;
    if size == 0 || size > 16 * 1024 * 1024 || !super::link::mapped_range(i, low, size) {
        return Err("TLS template exceeds readable segment or 16 MiB limit".into());
    }
    let mut alignment = 16usize;
    let mut addresses = Vec::new();
    for s in content {
        if s.alignment > 16 {
            return Err("TLS alignment exceeds 64 KiB limit".into());
        }
        alignment = alignment.max(1usize << s.alignment);
        // Preserve each variable's alignment after moving the template.
        if (s.address - low) % (1u64 << s.alignment) != 0 {
            return Err("TLS template alignment cannot be preserved".into());
        }
    }
    for s in descriptors {
        if !s.size.is_multiple_of(24)
            || s.size > 24 * 100_000
            || !super::link::mapped_range(i, s.address, s.size)
        {
            return Err("invalid TLV descriptor section".into());
        }
        let segment = i
            .segments
            .iter()
            .find(|seg| s.address >= seg.vmaddr && s.address + s.size <= seg.vmaddr + seg.vmsize)
            .ok_or("TLV descriptor outside segment")?;
        if segment.initprot & 2 == 0 {
            return Err("TLV descriptor must be writable during loading".into());
        }
        for off in (0..s.size).step_by(24) {
            let fileoff = segment.fileoff + (s.address - segment.vmaddr) + off;
            if s.address - segment.vmaddr + off + 24 > segment.filesize {
                return Err("TLV descriptor not file backed".into());
            }
            if crate::inspect::u64le(&i.data, fileoff as usize + 8)? != 0 {
                return Err("preinitialized/shared-cache TLV descriptor unsupported".into());
            }
            let offset = crate::inspect::u64le(&i.data, fileoff as usize + 16)?;
            if offset >= size {
                return Err("TLV offset exceeds template".into());
            }
            addresses.push(s.address + off);
        }
    }
    Ok(Some(Layout {
        address: low,
        size,
        alignment,
        descriptors: addresses,
    }))
}
