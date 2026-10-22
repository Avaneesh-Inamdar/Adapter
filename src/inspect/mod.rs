//! Bounded Mach-O inspection. `object` supplies an independent format parser;
//! explicit command decoding retains dyld information needed by the loader.
use crate::Result;
use object::{Object, ObjectSymbol};
use serde::Serialize;

pub const MAX_FILE: u64 = 128 * 1024 * 1024;

pub fn range(data: &[u8], off: u64, len: u64) -> Result<&[u8]> {
    let end = off.checked_add(len).ok_or("range overflow")?;
    if end > data.len() as u64 {
        return Err(format!(
            "range {off:#x}..{end:#x} exceeds file ({:#x})",
            data.len()
        ));
    }
    Ok(&data[off as usize..end as usize])
}
pub fn u32le(d: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        range(d, p as u64, 4)?.try_into().unwrap(),
    ))
}
pub fn u64le(d: &[u8], p: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        range(d, p as u64, 8)?.try_into().unwrap(),
    ))
}
fn be32(d: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        range(d, p as u64, 4)?.try_into().unwrap(),
    ))
}
fn be64(d: &[u8], p: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(
        range(d, p as u64, 8)?.try_into().unwrap(),
    ))
}
fn fixed(d: &[u8]) -> String {
    String::from_utf8_lossy(d.split(|&b| b == 0).next().unwrap_or_default()).into_owned()
}
fn cstr(d: &[u8], p: usize) -> Result<String> {
    let tail = d.get(p..).ok_or("string offset out of bounds")?;
    let n = tail
        .iter()
        .position(|&b| b == 0)
        .ok_or("unterminated string")?;
    Ok(String::from_utf8_lossy(&tail[..n]).into_owned())
}

#[derive(Debug, Clone, Serialize)]
pub struct Slice {
    pub cpu: u32,
    pub offset: u64,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct Command {
    pub code: u32,
    pub name: String,
    pub offset: u64,
    pub size: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub name: String,
    pub vmaddr: u64,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
    pub maxprot: u32,
    pub initprot: u32,
    pub flags: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub segment: String,
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub offset: u32,
    pub alignment: u32,
    pub flags: u32,
    pub relocations: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    pub name: String,
    pub weak: bool,
    pub command: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub name: String,
    pub address: u64,
    pub undefined: bool,
}
#[derive(Debug, Clone, Serialize)]
pub enum ExportTarget {
    Address { address: u64 },
    Absolute { value: u64 },
    Reexport { library: String, name: String },
    TlvDescriptor { address: u64 },
    Resolver,
}
#[derive(Debug, Clone, Serialize)]
pub struct Export {
    pub name: String,
    pub weak: bool,
    pub target: ExportTarget,
}
#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct Stream {
    pub offset: u32,
    pub size: u32,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct DyldInfo {
    pub rebase: Stream,
    pub bind: Stream,
    pub weak_bind: Stream,
    pub lazy_bind: Stream,
    pub export: Stream,
}
#[derive(Debug, Clone, Serialize)]
pub struct Image {
    pub inspection_status: String,
    pub architecture: String,
    pub cpu_subtype: u32,
    pub filetype: u32,
    pub flags: u32,
    pub slices: Vec<Slice>,
    pub selected_slice_offset: u64,
    pub commands: Vec<Command>,
    pub segments: Vec<Segment>,
    pub sections: Vec<Section>,
    pub dependencies: Vec<Dependency>,
    pub rpaths: Vec<String>,
    pub symbols: Vec<Symbol>,
    pub exports: Vec<Export>,
    pub entry_file_offset: Option<u64>,
    pub dyld: DyldInfo,
    pub chained_fixups: Option<Stream>,
    pub export_trie: Option<Stream>,
    pub unsupported: Vec<String>,
    #[serde(skip)]
    pub data: Vec<u8>,
}

pub fn read(path: &std::path::Path) -> Result<Image> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    use std::io::Read;
    let mut data = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    parse(&data)
}

pub fn parse(input: &[u8]) -> Result<Image> {
    if input.len() as u64 > MAX_FILE {
        return Err("input exceeds 128 MiB inspection limit".into());
    }
    let magic = be32(input, 0)?;
    let mut slices = Vec::new();
    let mut data = input;
    let mut selected = 0;
    if magic == 0xcafebabe || magic == 0xcafebabf {
        let count = be32(input, 4)? as usize;
        if count == 0 || count > 64 {
            return Err("invalid universal slice count".into());
        }
        let stride = if magic == 0xcafebabf { 32 } else { 20 };
        range(input, 8, (count * stride) as u64)?;
        for i in 0..count {
            let p = 8 + i * stride;
            let cpu = be32(input, p)?;
            let (offset, size, align) = if stride == 32 {
                (
                    be64(input, p + 8)?,
                    be64(input, p + 16)?,
                    be32(input, p + 24)?,
                )
            } else {
                (
                    be32(input, p + 8)? as u64,
                    be32(input, p + 12)? as u64,
                    be32(input, p + 16)?,
                )
            };
            range(input, offset, size)?;
            if size < 32
                || offset < (8 + count * stride) as u64
                || align > 30
                || offset % (1u64 << align) != 0
            {
                return Err("invalid universal slice range/alignment".into());
            }
            if slices
                .iter()
                .any(|s: &Slice| offset < s.offset + s.size && s.offset < offset + size)
            {
                return Err("overlapping universal slices".into());
            }
            slices.push(Slice { cpu, offset, size });
        }
        let s = slices
            .iter()
            .find(|s| s.cpu == 0x01000007)
            .unwrap_or(&slices[0]);
        selected = s.offset;
        data = range(input, s.offset, s.size)?;
        if u32le(data, 4)? != s.cpu {
            return Err("universal slice CPU does not match header".into());
        }
    }
    if u32le(data, 0)? != 0xfeedfacf {
        return Err("unsupported format: expected little-endian 64-bit Mach-O".into());
    }
    range(data, 0, 32)?;
    let cpu = u32le(data, 4)?;
    let ncmds = u32le(data, 16)? as usize;
    let cmdbytes = u32le(data, 20)? as usize;
    if ncmds > 16384 || ncmds > cmdbytes / 8 {
        return Err("invalid load-command count".into());
    }
    range(data, 32, cmdbytes as u64)?;
    let mut image = Image {
        inspection_status: "inspected (execution not established)".into(),
        architecture: match cpu {
            0x01000007 => "x86_64",
            0x0100000c => "arm64",
            _ => "unsupported",
        }
        .into(),
        cpu_subtype: u32le(data, 8)?,
        filetype: u32le(data, 12)?,
        flags: u32le(data, 24)?,
        slices,
        selected_slice_offset: selected,
        commands: Vec::new(),
        segments: Vec::new(),
        sections: Vec::new(),
        dependencies: Vec::new(),
        rpaths: Vec::new(),
        symbols: Vec::new(),
        exports: Vec::new(),
        entry_file_offset: None,
        dyld: DyldInfo::default(),
        chained_fixups: None,
        export_trie: None,
        unsupported: Vec::new(),
        data: data.to_vec(),
    };
    if cpu != 0x01000007 {
        image.unsupported.push("execution requires x86_64".into());
    }
    let mut p = 32;
    for _ in 0..ncmds {
        let cmd = u32le(data, p)?;
        let size = u32le(data, p + 4)? as usize;
        if size < 8 || !size.is_multiple_of(8) || p + size > 32 + cmdbytes {
            return Err(format!("invalid load command at {p:#x}"));
        }
        let c = range(data, p as u64, size as u64)?;
        let name = match cmd {
            0x19 => "LC_SEGMENT_64",
            2 => "LC_SYMTAB",
            0xb => "LC_DYSYMTAB",
            0xc => "LC_LOAD_DYLIB",
            0xd => "LC_ID_DYLIB",
            0xe => "LC_LOAD_DYLINKER",
            0x80000018 => "LC_LOAD_WEAK_DYLIB",
            0x8000001f => "LC_REEXPORT_DYLIB",
            0x80000023 => "LC_LOAD_UPWARD_DYLIB",
            0x8000001c => "LC_RPATH",
            0x1b => "LC_UUID",
            0x1d => "LC_CODE_SIGNATURE",
            0x22 | 0x80000022 => "LC_DYLD_INFO",
            0x80000028 => "LC_MAIN",
            0x80000034 => "LC_DYLD_CHAINED_FIXUPS",
            0x80000033 => "LC_DYLD_EXPORTS_TRIE",
            0x26 => "LC_FUNCTION_STARTS",
            0x29 => "LC_DATA_IN_CODE",
            0x24 => "LC_VERSION_MIN_MACOSX",
            0x32 => "LC_BUILD_VERSION",
            0x2a => "LC_SOURCE_VERSION",
            0x1a => "LC_ROUTINES_64",
            5 => "LC_UNIXTHREAD",
            0x21 | 0x2c => "LC_ENCRYPTION_INFO",
            _ => "unknown",
        };
        image.commands.push(Command {
            code: cmd,
            name: name.into(),
            offset: p as u64,
            size: size as u32,
        });
        match cmd {
            0x19 => {
                range(c, 0, 72)?;
                let ns = u32le(c, 64)? as usize;
                if ns > 65536 || size != 72 + ns * 80 {
                    return Err("invalid segment section count/size".into());
                }
                let seg = Segment {
                    name: fixed(&c[8..24]),
                    vmaddr: u64le(c, 24)?,
                    vmsize: u64le(c, 32)?,
                    fileoff: u64le(c, 40)?,
                    filesize: u64le(c, 48)?,
                    maxprot: u32le(c, 56)?,
                    initprot: u32le(c, 60)?,
                    flags: u32le(c, 68)?,
                };
                if seg.flags & !0x14 != 0 {
                    image.unsupported.push(format!(
                        "unsupported segment flags: {} {:#x}",
                        seg.name, seg.flags
                    ));
                }
                seg.vmaddr
                    .checked_add(seg.vmsize)
                    .ok_or("segment address overflow")?;
                range(data, seg.fileoff, seg.filesize)?;
                // MH_OBJECT may have compact sections that do not model VM layout.
                if image.filetype != 1 && seg.filesize > seg.vmsize {
                    return Err("segment file size exceeds VM size".into());
                }
                for j in 0..ns {
                    let q = 72 + j * 80;
                    let s = Section {
                        name: fixed(&c[q..q + 16]),
                        segment: fixed(&c[q + 16..q + 32]),
                        address: u64le(c, q + 32)?,
                        size: u64le(c, q + 40)?,
                        offset: u32le(c, q + 48)?,
                        alignment: u32le(c, q + 52)?,
                        flags: u32le(c, q + 64)?,
                        relocations: u32le(c, q + 60)?,
                    };
                    let kind = s.flags & 0xff;
                    if ![1, 0xc, 0x12].contains(&kind) {
                        range(data, s.offset as u64, s.size)?;
                    }
                    if image.filetype != 1
                        && (s.address < seg.vmaddr
                            || s.address.checked_add(s.size).ok_or("section overflow")?
                                > seg.vmaddr + seg.vmsize)
                    {
                        return Err("section exceeds segment VM range".into());
                    }
                    range(data, u32le(c, q + 56)? as u64, s.relocations as u64 * 8)?;
                    if s.relocations != 0 {
                        image
                            .unsupported
                            .push(format!("section relocations: {}", s.name));
                    }
                    if kind == 0x15 {
                        image
                            .unsupported
                            .push(format!("thread-local initialization functions: {}", s.name));
                    }
                    if s.name.starts_with("__objc_")
                        && !matches!(
                            s.name.as_str(),
                            "__objc_classlist"
                                | "__objc_nlclslist"
                                | "__objc_classrefs"
                                | "__objc_superrefs"
                                | "__objc_selrefs"
                                | "__objc_imageinfo"
                                | "__objc_const"
                                | "__objc_data"
                                | "__objc_methname"
                                | "__objc_methtype"
                                | "__objc_classname"
                                | "__objc_ivar"
                                | "__objc_catlist"
                                | "__objc_nlcatlist"
                                | "__objc_methlist"
                                | "__objc_protolist"
                                | "__objc_protorefs"
                        )
                    {
                        image
                            .unsupported
                            .push(format!("Objective-C metadata: {}", s.name));
                    }
                    image.sections.push(s);
                }
                image.segments.push(seg);
            }
            0xc | 0xd | 0x80000018 | 0x8000001f | 0x80000023 => {
                range(c, 0, 24)?;
                let off = u32le(c, 8)? as usize;
                if off < 24 {
                    return Err("dylib string overlaps header".into());
                }
                let name = cstr(c, off)?;
                if cmd != 0xd {
                    image.dependencies.push(Dependency {
                        name,
                        weak: cmd == 0x80000018,
                        command: cmd,
                    });
                }
            }
            0x8000001c | 0xe => {
                let off = u32le(c, 8)? as usize;
                if off < 12 {
                    return Err("load-command string overlaps header".into());
                }
                let name = cstr(c, off)?;
                if cmd == 0x8000001c {
                    image.rpaths.push(name);
                }
            }
            0x80000028 => {
                range(c, 0, 24)?;
                if u64le(c, 16)? != 0 {
                    image.unsupported.push("custom LC_MAIN stack size".into());
                }
                if image.entry_file_offset.replace(u64le(c, 8)?).is_some() {
                    return Err("duplicate LC_MAIN".into());
                }
            }
            0x22 | 0x80000022 => {
                range(c, 0, 48)?;
                let stream = |q| -> Result<Stream> {
                    let s = Stream {
                        offset: u32le(c, q)?,
                        size: u32le(c, q + 4)?,
                    };
                    range(data, s.offset as u64, s.size as u64)?;
                    Ok(s)
                };
                image.dyld = DyldInfo {
                    rebase: stream(8)?,
                    bind: stream(16)?,
                    weak_bind: stream(24)?,
                    lazy_bind: stream(32)?,
                    export: stream(40)?,
                };
            }
            2 => {
                range(c, 0, 24)?;
                let count = u32le(c, 12)?;
                if count > 1_000_000 {
                    return Err("symbol count exceeds resource limit".into());
                }
                range(data, u32le(c, 8)? as u64, count as u64 * 16)?;
                let strings = range(data, u32le(c, 16)? as u64, u32le(c, 20)? as u64)?;
                for i in 0..count as usize {
                    let q = u32le(c, 8)? as usize + i * 16;
                    cstr(strings, u32le(data, q)? as usize)?;
                }
            }
            0x1d | 0x26 | 0x29 | 0x80000033 | 0x80000034 => {
                range(c, 0, 16)?;
                range(data, u32le(c, 8)? as u64, u32le(c, 12)? as u64)?;
                if cmd == 0x80000033 {
                    image.export_trie = Some(Stream {
                        offset: u32le(c, 8)?,
                        size: u32le(c, 12)?,
                    });
                }
                if cmd == 0x80000034
                    && image
                        .chained_fixups
                        .replace(Stream {
                            offset: u32le(c, 8)?,
                            size: u32le(c, 12)?,
                        })
                        .is_some()
                {
                    return Err("duplicate chained fixups command".into());
                }
            }
            0x21 | 0x2c => {
                if u32le(c, 16)? != 0 {
                    image.unsupported.push("encrypted image".into());
                }
            }
            0x1a | 5 => image.unsupported.push(name.into()),
            _ => {
                if cmd & 0x80000000 != 0 {
                    image
                        .unsupported
                        .push(format!("unknown required command {cmd:#x}"));
                }
            }
        }
        p += size;
    }
    if p != 32 + cmdbytes {
        return Err("load commands do not fill sizeofcmds".into());
    }
    let file = object::File::parse(data).map_err(|e| format!("object parser: {e}"))?;
    for sym in file.symbols() {
        image.symbols.push(Symbol {
            name: sym.name().map_err(|e| e.to_string())?.into(),
            address: sym.address(),
            undefined: sym.is_undefined(),
        });
    }
    // The maintained parser handles both export tries and the external nlist
    // fallback, preserving reexports, weak definitions and TLV distinctions.
    // An empty explicit trie is valid (e.g. a pure reexport dylib). object 0.40
    // attempts to read a terminal byte even for the empty iterator in this case.
    let empty_trie = if let Some(stream) = image.export_trie {
        stream.size == 0
    } else {
        image
            .commands
            .iter()
            .any(|c| matches!(c.code, 0x22 | 0x80000022))
            && image.dyld.export.size == 0
    };
    if !empty_trie {
        for export in file.exports().map_err(|e| format!("exports: {e}"))? {
            let export = export.map_err(|e| format!("exports: {e}"))?;
            if image.exports.len() >= 100_000 {
                return Err("export count exceeds limit".into());
            }
            let export_name = export.name();
            let name = export_name
                .name()
                .ok_or("export ordinal without name unsupported")?;
            if name.len() > 16384 {
                return Err("export name exceeds limit".into());
            }
            let target = match export.target() {
                object::ExportTarget::Address { address } => ExportTarget::Address { address },
                object::ExportTarget::Absolute { value } => ExportTarget::Absolute { value },
                object::ExportTarget::TlvDescriptor { address } => {
                    ExportTarget::TlvDescriptor { address }
                }
                object::ExportTarget::Resolver { .. } => ExportTarget::Resolver,
                object::ExportTarget::Reexport { library, name } => ExportTarget::Reexport {
                    library: String::from_utf8(library.to_vec())
                        .map_err(|_| "invalid reexport library UTF-8")?,
                    name: String::from_utf8(
                        name.name()
                            .ok_or("reexport ordinal without name unsupported")?
                            .to_vec(),
                    )
                    .map_err(|_| "invalid reexport name UTF-8")?,
                },
                _ => return Err("unsupported export target".into()),
            };
            image.exports.push(Export {
                name: String::from_utf8(name.to_vec()).map_err(|_| "invalid export name UTF-8")?,
                weak: export.is_weak(),
                target,
            });
        }
    }
    image.exports.sort_by(|a, b| a.name.cmp(&b.name));
    if image
        .exports
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err("duplicate public export name".into());
    }
    if image.dyld.weak_bind.size != 0 {
        image.unsupported.push("weak binding/coalescing".into());
    }
    Ok(image)
}

pub fn report(i: &Image) -> String {
    let mut s = format!(
        "{} Mach-O type={} flags={:#x}\n{}\n",
        i.architecture, i.filetype, i.flags, i.inspection_status
    );
    for seg in &i.segments {
        s += &format!(
            "segment {} VM={:#x}+{:#x} file={:#x}+{:#x} prot={}\n",
            seg.name, seg.vmaddr, seg.vmsize, seg.fileoff, seg.filesize, seg.initprot
        );
    }
    for d in &i.dependencies {
        s += &format!(
            "dependency {}{}\n",
            d.name,
            if d.weak { " (weak)" } else { "" }
        );
    }
    for symbol in &i.symbols {
        s += &format!(
            "{} {} {:#x}\n",
            if symbol.undefined { "import" } else { "symbol" },
            symbol.name,
            symbol.address
        );
    }
    for u in &i.unsupported {
        s += &format!("execution unsupported: {u}\n");
    }
    s
}
