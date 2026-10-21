use adapter::{
    inspect,
    loader::fixups::{sleb, uleb},
};
fn minimal() -> Vec<u8> {
    let mut v = vec![0u8; 32];
    for (p, value) in [(0, 0xfeedfacfu32), (4, 0x01000007), (8, 3), (12, 2)] {
        v[p..p + 4].copy_from_slice(&value.to_le_bytes());
    }
    v
}
#[test]
fn valid_header() {
    let i = inspect::parse(&minimal()).unwrap();
    assert_eq!(i.architecture, "x86_64");
    assert!(i.entry_file_offset.is_none());
}
#[test]
fn every_truncated_header_is_error() {
    let v = minimal();
    for n in 0..32 {
        assert!(inspect::parse(&v[..n]).is_err(), "accepted length {n}");
    }
}
#[test]
fn forged_command_count() {
    let mut v = minimal();
    v[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(inspect::parse(&v).is_err());
}
#[test]
fn invalid_command_size() {
    for n in [0u32, 4, 9, 24] {
        let mut v = minimal();
        v[16..20].copy_from_slice(&1u32.to_le_bytes());
        v[20..24].copy_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&[0; 8]);
        v[36..40].copy_from_slice(&n.to_le_bytes());
        assert!(inspect::parse(&v).is_err());
    }
}
#[test]
fn command_table_must_be_exact() {
    let mut v = minimal();
    v[20..24].copy_from_slice(&8u32.to_le_bytes());
    v.extend([0; 8]);
    assert!(inspect::parse(&v).is_err());
}
#[test]
fn range_overflow() {
    assert!(inspect::range(&[0; 8], u64::MAX, 2).is_err());
    assert!(inspect::range(&[0; 8], 7, 2).is_err());
}
#[test]
fn uleb_valid() {
    assert_eq!(uleb(&[0xe5, 0x8e, 0x26], &mut 0).unwrap(), 624485);
    assert_eq!(
        uleb(
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1],
            &mut 0
        )
        .unwrap(),
        u64::MAX
    );
}
#[test]
fn sleb_valid() {
    assert_eq!(sleb(&[0x9b, 0xf1, 0x59], &mut 0).unwrap(), -624485);
    assert_eq!(sleb(&[0x7f], &mut 0).unwrap(), -1);
    assert_eq!(sleb(&[0], &mut 0).unwrap(), 0);
}
#[test]
fn leb_reject_truncation_and_overflow() {
    for v in [
        vec![0x80],
        vec![0xff; 11],
        vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2],
    ] {
        assert!(uleb(&v, &mut 0).is_err());
    }
    assert!(sleb(&[0x80; 11], &mut 0).is_err());
}
#[test]
fn deterministic_mutation_probe() {
    let original = minimal();
    let mut state = 0x12345678u32;
    for _ in 0..5000 {
        let mut v = original.clone();
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let pos = state as usize % v.len();
        v[pos] ^= (state >> 24) as u8;
        let _ = inspect::parse(&v);
    }
}

#[test]
fn fat64_selection() {
    let thin = minimal();
    let mut data = vec![0u8; 4096];
    for (p, v) in [
        (0, 0xcafebabfu32),
        (4, 1),
        (8, 0x01000007),
        (12, 3),
        (32, 12),
    ] {
        data[p..p + 4].copy_from_slice(&v.to_be_bytes());
    }
    data[16..24].copy_from_slice(&4096u64.to_be_bytes());
    data[24..32].copy_from_slice(&(thin.len() as u64).to_be_bytes());
    data.extend(thin);
    let i = inspect::parse(&data).unwrap();
    assert_eq!(i.selected_slice_offset, 4096);
    assert_eq!(i.architecture, "x86_64");
}

fn chained_image(pointer_format: u16, import_format: u32, raw: u64) -> inspect::Image {
    let mut i = inspect::parse(&minimal()).unwrap();
    i.data = vec![0; 256];
    i.segments.push(inspect::Segment {
        name: "__DATA".into(),
        vmaddr: 0x100000000,
        vmsize: 4096,
        fileoff: 128,
        filesize: 128,
        maxprot: 3,
        initprot: 3,
        flags: 0,
    });
    i.dependencies.push(inspect::Dependency {
        name: "/usr/lib/libSystem.B.dylib".into(),
        weak: false,
        command: 12,
    });
    i.chained_fixups = Some(inspect::Stream {
        offset: 0,
        size: 80,
    });
    for (p, v) in [
        (4, 28u32),
        (8, 60),
        (12, 68),
        (16, 1),
        (20, import_format),
        (28, 1),
        (32, 8),
        (36, 24),
        (60, 1),
    ] {
        i.data[p..p + 4].copy_from_slice(&v.to_le_bytes());
    }
    i.data[40..42].copy_from_slice(&4096u16.to_le_bytes());
    i.data[42..44].copy_from_slice(&pointer_format.to_le_bytes());
    i.data[56..58].copy_from_slice(&1u16.to_le_bytes());
    i.data[68..74].copy_from_slice(b"_puts\0");
    i.data[128..136].copy_from_slice(&raw.to_le_bytes());
    i
}
#[test]
fn chained_absolute_and_offset_rebases() {
    for (format, raw) in [(2, 0x100000010u64), (6, 16)] {
        let i = chained_image(format, 1, raw);
        let p = adapter::loader::fixups::plan(&i).unwrap();
        assert_eq!(
            p[0].kind,
            adapter::loader::fixups::Kind::RebaseTo {
                target: 0x100000010
            }
        );
    }
}
#[test]
fn chained_import_and_addend_table() {
    for format in [1, 2] {
        let mut i = chained_image(2, format, 1u64 << 63);
        if format == 2 {
            i.data[64..68].copy_from_slice(&(-7i32).to_le_bytes());
        }
        let p = adapter::loader::fixups::plan(&i).unwrap();
        assert_eq!(
            p[0].kind,
            adapter::loader::fixups::Kind::Bind {
                symbol: "_puts".into(),
                ordinal: 1,
                addend: if format == 2 { -7 } else { 0 }
            }
        );
    }
}
#[test]
fn chained_bounds_and_reserved_bits() {
    for raw in [(1u64 << 63) | 1, 1u64 << 44, (1u64 << 51) | 16] {
        let i = chained_image(2, 1, raw);
        assert!(adapter::loader::fixups::plan(&i).is_err());
    }
}
#[cfg(windows)]
#[test]
fn windows_final_protection_and_release() {
    use windows_sys::Win32::System::Memory::*;
    let mut i = chained_image(2, 1, 16);
    i.data.resize(4096, 0);
    i.segments[0].filesize = 4096;
    i.segments[0].fileoff = 0;
    i.segments[0].flags = 0x10;
    let mapping = adapter::platform::windows::Mapping::new(&i).unwrap();
    let p = mapping.pointer(0x100000000, 1).unwrap();
    mapping.protect(&i).unwrap();
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { VirtualQuery(p as *const _, &mut info, std::mem::size_of_val(&info)) },
        0
    );
    assert_eq!(info.Protect, PAGE_READONLY);
    drop(mapping);
    assert_ne!(
        unsafe { VirtualQuery(p as *const _, &mut info, std::mem::size_of_val(&info)) },
        0
    );
    assert_eq!(info.State, MEM_FREE);
}
