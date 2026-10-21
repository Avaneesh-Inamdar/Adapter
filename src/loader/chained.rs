//! dyld chained pointer formats 2 and 6, from Apple's fixup-chains.h.
//! Each chain uses a 4-byte stride. Imports are a separate indexed table.
use super::fixups::{Fixup, Kind};
use crate::{
    Result,
    inspect::{Image, Stream, range, u32le, u64le},
};
fn u16le(d: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        range(d, p as u64, 2)?.try_into().unwrap(),
    ))
}

pub fn plan(i: &Image, stream: Stream) -> Result<Vec<Fixup>> {
    let d = range(&i.data, stream.offset as u64, stream.size as u64)?;
    range(d, 0, 28)?;
    if u32le(d, 0)? != 0 || u32le(d, 24)? != 0 {
        return Err("unsupported chained fixups version or compressed symbols".into());
    }
    let starts = u32le(d, 4)? as usize;
    let imports = u32le(d, 8)? as usize;
    let strings = u32le(d, 12)? as usize;
    let count = u32le(d, 16)? as usize;
    let format = u32le(d, 20)?;
    let stride = match format {
        1 => 4,
        2 => 8,
        _ => return Err(format!("unsupported chained import format {format}")),
    };
    if count > 1_000_000 || starts < 28 || imports < 28 || strings < 28 {
        return Err("invalid chained table offsets/count".into());
    }
    range(d, imports as u64, (count * stride) as u64)?;
    let strings = range(
        d,
        strings as u64,
        (d.len() as u64)
            .checked_sub(strings as u64)
            .ok_or("chained symbols offset out of bounds")?,
    )?;
    let mut symbols = Vec::new();
    for idx in 0..count {
        let raw = u32le(d, imports + idx * stride)?;
        let ordinal = (raw & 255) as i32;
        if ordinal == 0
            || ordinal > 240
            || ordinal as usize > i.dependencies.len()
            || raw & 256 != 0
        {
            return Err("chained import requires non-weak two-level dylib ordinal".into());
        }
        let nameoff = (raw >> 9) as usize;
        let tail = strings
            .get(nameoff..)
            .ok_or("chained symbol offset out of bounds")?;
        let n = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or("unterminated chained symbol")?;
        let name = std::str::from_utf8(&tail[..n])
            .map_err(|_| "invalid chained symbol UTF-8")?
            .to_owned();
        let addend = if format == 2 {
            u32le(d, imports + idx * stride + 4)? as i32 as i64
        } else {
            0
        };
        symbols.push((name, ordinal, addend));
    }
    let seg_count = u32le(d, starts)? as usize;
    if seg_count > i.segments.len() {
        return Err("chained segment count exceeds Mach-O segments".into());
    }
    range(d, (starts + 4) as u64, seg_count as u64 * 4)?;
    let low = i
        .segments
        .iter()
        .filter(|s| s.name != "__PAGEZERO" && s.vmsize != 0)
        .map(|s| s.vmaddr)
        .min()
        .ok_or("no mapped image base")?;
    let mut out = Vec::new();
    for seg in 0..seg_count {
        let relative = u32le(d, starts + 4 + seg * 4)? as usize;
        if relative == 0 {
            continue;
        }
        if relative < 4 + seg_count * 4 {
            return Err("chained segment info overlaps offset table".into());
        }
        let p = starts
            .checked_add(relative)
            .ok_or("chained offset overflow")?;
        let size = u32le(d, p)? as usize;
        let info = range(d, p as u64, size as u64)?;
        range(info, 0, 22)?;
        let page_size = u16le(info, 4)? as u64;
        let pointer_format = u16le(info, 6)?;
        let s = &i.segments[seg];
        if ![4096, 16384].contains(&page_size) || ![2, 6].contains(&pointer_format) {
            return Err(format!(
                "unsupported chained page size or pointer format {pointer_format}"
            ));
        }
        if s.name == "__PAGEZERO"
            || s.vmaddr.checked_sub(low) != Some(u64le(info, 8)?)
            || u32le(info, 16)? != 0
        {
            return Err("invalid chained segment offset/max pointer".into());
        }
        let pages = u16le(info, 20)? as u64;
        range(info, 22, pages * 2)?;
        if pages > s.vmsize.div_ceil(page_size) {
            return Err("chained page count exceeds segment".into());
        }
        for page in 0..pages {
            let start = u16le(info, 22 + page as usize * 2)?;
            if start == 0xffff {
                continue;
            }
            if start & 0x8000 != 0 {
                return Err("multi-start chained pages unsupported".into());
            }
            let mut off = page * page_size + start as u64;
            loop {
                if off + 8 > (page + 1) * page_size
                    || off + 8 > s.filesize
                    || off + 8 > s.vmsize
                    || s.initprot & 2 == 0
                {
                    return Err("chained fixup exceeds writable file-backed page".into());
                }
                let raw = u64le(&i.data, (s.fileoff + off) as usize)?;
                let next = ((raw >> 51) & 0xfff) * 4;
                let kind = if raw >> 63 == 1 {
                    if (raw >> 32) & 0x7ffff != 0 {
                        return Err("reserved chained bind bits set".into());
                    }
                    let (name, ordinal, addend) = symbols
                        .get((raw & 0xffffff) as usize)
                        .ok_or("chained import index out of bounds")?;
                    let addend = addend
                        .checked_add(((raw >> 24) & 0xff) as i64)
                        .ok_or("chained addend overflow")?;
                    Kind::Bind {
                        symbol: name.clone(),
                        ordinal: *ordinal,
                        addend,
                    }
                } else {
                    if (raw >> 44) & 0x7f != 0 || (raw >> 36) & 0xff != 0 {
                        return Err(
                            "reserved chained rebase bits or high8 pointer unsupported".into()
                        );
                    }
                    let target = raw & 0xfffffffff;
                    Kind::RebaseTo {
                        target: if pointer_format == 6 {
                            low.checked_add(target)
                                .ok_or("chained rebase target overflow")?
                        } else {
                            target
                        },
                    }
                };
                if out.len() >= 1_000_000 {
                    return Err("chained fixup count exceeds limit".into());
                }
                out.push(Fixup {
                    segment: seg,
                    offset: off,
                    kind,
                });
                if next == 0 {
                    break;
                }
                if next < 8 {
                    return Err("overlapping chained pointers".into());
                }
                off = off.checked_add(next).ok_or("chained offset overflow")?;
            }
        }
    }
    Ok(out)
}
