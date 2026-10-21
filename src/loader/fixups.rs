//! Classic dyld opcode decoding to a platform-independent fixup plan.
use crate::{
    Result,
    inspect::{Image, Stream, range},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Rebase,
    RebaseTo {
        target: u64,
    },
    Bind {
        symbol: String,
        ordinal: i32,
        addend: i64,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct Fixup {
    pub segment: usize,
    pub offset: u64,
    pub kind: Kind,
}

pub fn uleb(data: &[u8], p: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let b = *data.get(*p).ok_or("truncated ULEB128")?;
        *p += 1;
        if shift == 63 && b & 0x7f > 1 {
            return Err("ULEB128 overflow".into());
        }
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("ULEB128 overflow".into())
}
pub fn sleb(data: &[u8], p: &mut usize) -> Result<i64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let b = *data.get(*p).ok_or("truncated SLEB128")?;
        *p += 1;
        if shift == 63 && b != 0 && b != 0x7f {
            return Err("SLEB128 overflow".into());
        }
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            if shift < 63 && b & 0x40 != 0 {
                value |= u64::MAX << (shift + 7);
            }
            return Ok(value as i64);
        }
    }
    Err("SLEB128 overflow".into())
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or("dyld offset overflow".into())
}
fn push(out: &mut Vec<Fixup>, i: &Image, segment: usize, offset: u64, kind: Kind) -> Result<()> {
    let s = i
        .segments
        .get(segment)
        .ok_or("dyld segment index out of range")?;
    if s.name == "__PAGEZERO" || offset.checked_add(8).ok_or("fixup overflow")? > s.vmsize {
        return Err("fixup exceeds segment".into());
    }
    if s.initprot & 2 == 0 {
        return Err("fixup target must be a writable segment".into());
    }
    if out.len() >= 1_000_000 {
        return Err("fixup count exceeds limit".into());
    }
    out.push(Fixup {
        segment,
        offset,
        kind,
    });
    Ok(())
}

fn rebase(i: &Image, out: &mut Vec<Fixup>) -> Result<()> {
    let d = range(
        &i.data,
        i.dyld.rebase.offset as u64,
        i.dyld.rebase.size as u64,
    )?;
    let (mut p, mut seg, mut off, mut typ) = (0, usize::MAX, 0u64, 0);
    while p < d.len() {
        let b = d[p];
        p += 1;
        let imm = b & 15;
        match b & 0xf0 {
            0 => break,
            0x10 => typ = imm,
            0x20 => {
                seg = imm as usize;
                off = uleb(d, &mut p)?;
            }
            0x30 => off = add(off, uleb(d, &mut p)?)?,
            0x40 => off = add(off, imm as u64 * 8)?,
            code @ (0x50 | 0x60 | 0x70 | 0x80) => {
                if typ != 1 {
                    return Err("only pointer rebases supported".into());
                }
                let (count, skip) = match code {
                    0x50 => (imm as u64, 0),
                    0x60 => (uleb(d, &mut p)?, 0),
                    0x70 => (1, uleb(d, &mut p)?),
                    _ => (uleb(d, &mut p)?, uleb(d, &mut p)?),
                };
                if count > 1_000_000 {
                    return Err("excessive rebase count".into());
                }
                for _ in 0..count {
                    push(out, i, seg, off, Kind::Rebase)?;
                    off = add(off, add(8, skip)?)?;
                }
            }
            _ => return Err(format!("unsupported rebase opcode {b:#x}")),
        }
    }
    Ok(())
}

fn bind(i: &Image, stream: Stream, lazy: bool, out: &mut Vec<Fixup>) -> Result<()> {
    let d = range(&i.data, stream.offset as u64, stream.size as u64)?;
    let (mut p, mut seg, mut off, mut typ, mut ordinal, mut addend) =
        (0, usize::MAX, 0u64, 1, 0, 0i64);
    let mut symbol = String::new();
    while p < d.len() {
        let b = d[p];
        p += 1;
        let imm = b & 15;
        match b & 0xf0 {
            0 => {
                if !lazy {
                    break;
                }
                seg = usize::MAX;
                off = 0;
                typ = 1;
                ordinal = 0;
                addend = 0;
                symbol.clear();
            }
            0x10 => ordinal = imm as i32,
            0x20 => ordinal = i32::try_from(uleb(d, &mut p)?).map_err(|_| "ordinal overflow")?,
            0x30 => {
                ordinal = if imm == 0 {
                    0
                } else {
                    (imm | 0xf0) as i8 as i32
                }
            }
            0x40 => {
                if imm != 0 {
                    return Err("weak/non-default bind flags unsupported".into());
                }
                let len = d[p..]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or("unterminated dyld symbol")?;
                symbol = std::str::from_utf8(&d[p..p + len])
                    .map_err(|_| "invalid dyld symbol")?
                    .into();
                p += len + 1;
            }
            0x50 => typ = imm,
            0x60 => addend = sleb(d, &mut p)?,
            0x70 => {
                seg = imm as usize;
                off = uleb(d, &mut p)?;
            }
            0x80 => off = add(off, uleb(d, &mut p)?)?,
            code @ (0x90 | 0xa0 | 0xb0 | 0xc0) => {
                if typ != 1
                    || symbol.is_empty()
                    || ordinal <= 0
                    || ordinal as usize > i.dependencies.len()
                {
                    return Err(
                        "bind requires pointer type, symbol and valid two-level dylib ordinal"
                            .into(),
                    );
                }
                let (count, skip) = match code {
                    0x90 => (1, 0),
                    0xa0 => (1, uleb(d, &mut p)?),
                    0xb0 => (1, imm as u64 * 8),
                    _ => (uleb(d, &mut p)?, uleb(d, &mut p)?),
                };
                if count > 1_000_000 {
                    return Err("excessive bind count".into());
                }
                for _ in 0..count {
                    push(
                        out,
                        i,
                        seg,
                        off,
                        Kind::Bind {
                            symbol: symbol.clone(),
                            ordinal,
                            addend,
                        },
                    )?;
                    off = add(off, add(8, skip)?)?;
                }
            }
            _ => return Err(format!("unsupported bind opcode {b:#x}")),
        }
    }
    Ok(())
}
pub fn plan(i: &Image) -> Result<Vec<Fixup>> {
    if i.dyld.weak_bind.size != 0 {
        return Err("weak binding/coalescing unsupported".into());
    }
    if let Some(stream) = i.chained_fixups {
        if i.dyld.rebase.size != 0 || i.dyld.bind.size != 0 || i.dyld.lazy_bind.size != 0 {
            return Err("mixed classic and chained fixups unsupported".into());
        }
        return super::chained::plan(i, stream);
    }
    let mut out = Vec::new();
    rebase(i, &mut out)?;
    bind(i, i.dyld.bind, false, &mut out)?;
    bind(i, i.dyld.lazy_bind, true, &mut out)?;
    Ok(out)
}
