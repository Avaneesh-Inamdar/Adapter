//! `sysctl`, `sysctlbyname`, `sysctlnametomib` and a small `libproc` surface.
//!
//! Values come from the Windows host where the guest could plausibly rely on
//! them (processor count, cache sizes, memory, boot time, CPUID strings) and are
//! otherwise fixed Darwin 24.6 answers that match `uname`. Every name is
//! read-only. Names that Darwin allocates dynamically (`hw.logicalcpu`,
//! `machdep.cpu.*`, ...) get adapter-assigned second components starting at
//! 1000; statically numbered names use the positions in xnu's `sys/sysctl.h`,
//! which `tools/update_progress.py` cross-checks against the pinned header.
use crate::darwin::{libc, thread_state};
use std::{
    ffi::{CStr, c_char},
    sync::OnceLock,
};

const EPERM: i32 = 1;
const ENOENT: i32 = 2;
const ESRCH: i32 = 3;
const ENOMEM: i32 = 12;
const EINVAL: i32 = 22;

const CTL_KERN: i32 = 1;
const CTL_VM: i32 = 2;
const CTL_HW: i32 = 6;
const CTL_MACHDEP: i32 = 7;
const CTL_USER: i32 = 8;
const FIRST_DYNAMIC: i32 = 1000;

const OS_RELEASE: &str = "24.6.0";
const OS_VERSION_STRING: &str = "Darwin Kernel Version 24.6.0: Adapter compatibility layer";
const OS_BUILD: &str = "24G84";
const OS_PRODUCT: &str = "15.6";
const MODEL: &str = "AdapterHost1,1";

enum Value {
    Int(i32),
    Quad(u64),
    Text(String),
    /// `struct timeval` (16 bytes on x86-64).
    Time(i64, i32),
    Raw(Vec<u8>),
}

struct Entry {
    name: &'static str,
    mib: &'static [i32],
    value: fn() -> Option<Value>,
}

struct Resolved {
    name: &'static str,
    mib: Vec<i32>,
    value: fn() -> Option<Value>,
}

fn clamp_int(value: u64) -> Value {
    Value::Int(i32::try_from(value).unwrap_or(i32::MAX))
}

#[derive(Default, Clone, Copy)]
struct Cpu {
    logical: u32,
    physical: u32,
    line: u64,
    l1i: u64,
    l1d: u64,
    l2: u64,
    l3: u64,
}

fn cpu() -> Cpu {
    static CPU: OnceLock<Cpu> = OnceLock::new();
    *CPU.get_or_init(|| {
        let mut info = Cpu {
            logical: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
            ..Cpu::default()
        };
        info.physical = info.logical;
        info.line = 64;
        #[cfg(windows)]
        host_cpu(&mut info);
        info
    })
}

#[cfg(windows)]
fn host_cpu(info: &mut Cpu) {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformation, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
    };
    let mut length = 0u32;
    unsafe { GetLogicalProcessorInformation(std::ptr::null_mut(), &mut length) };
    let size = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
    if length == 0 || !(length as usize).is_multiple_of(size) {
        return;
    }
    let mut records = vec![
        unsafe { std::mem::zeroed::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>() };
        length as usize / size
    ];
    if unsafe { GetLogicalProcessorInformation(records.as_mut_ptr(), &mut length) } == 0 {
        return;
    }
    let (mut cores, mut logical) = (0u32, 0u32);
    for record in &records {
        match record.Relationship {
            0 => {
                cores += 1;
                logical += record.ProcessorMask.count_ones();
            }
            2 => {
                let cache = unsafe { record.Anonymous.Cache };
                let bytes = u64::from(cache.Size);
                // Caches repeat per core group; the first of each kind is enough.
                let slot = match (cache.Level, cache.Type) {
                    (1, 1) => &mut info.l1i,
                    (1, _) => &mut info.l1d,
                    (2, _) => &mut info.l2,
                    (3, _) => &mut info.l3,
                    _ => continue,
                };
                if *slot == 0 {
                    *slot = bytes;
                }
                if cache.LineSize != 0 {
                    info.line = u64::from(cache.LineSize);
                }
            }
            _ => {}
        }
    }
    if cores != 0 {
        info.physical = cores;
    }
    if logical != 0 {
        info.logical = logical;
    }
}

/// `(total physical, available physical, total pagefile, available pagefile)`.
fn memory() -> (u64, u64, u64, u64) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            return (
                status.ullTotalPhys,
                status.ullAvailPhys,
                status.ullTotalPageFile,
                status.ullAvailPageFile,
            );
        }
    }
    (8 << 30, 4 << 30, 8 << 30, 4 << 30)
}

fn uptime_millis() -> u64 {
    #[cfg(windows)]
    {
        unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

fn boot_time() -> (i64, i32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let boot = now - uptime_millis() as i64;
    (boot.div_euclid(1000), (boot.rem_euclid(1000) * 1000) as i32)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

/// Nominal processor frequency in Hz from the registry, when Windows reports it.
fn cpu_hertz() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{
            HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
        };
        let key: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\0"
            .encode_utf16()
            .collect();
        let value: Vec<u16> = "~MHz\0".encode_utf16().collect();
        let mut mhz = 0u32;
        let mut size = 4u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut mhz as *mut u32).cast(),
                &mut size,
            )
        };
        if status == 0 && mhz != 0 {
            return Some(u64::from(mhz) * 1_000_000);
        }
    }
    None
}

#[allow(unused_unsafe)]
fn cpuid_leaf(leaf: u32) -> [u32; 4] {
    let result = unsafe { std::arch::x86_64::__cpuid(leaf) };
    [result.eax, result.ebx, result.ecx, result.edx]
}

fn cpu_vendor() -> String {
    let leaf = cpuid_leaf(0);
    let mut bytes = Vec::new();
    for register in [leaf[1], leaf[3], leaf[2]] {
        bytes.extend_from_slice(&register.to_le_bytes());
    }
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .to_string()
}

fn cpu_brand() -> String {
    let mut bytes = Vec::new();
    for leaf in 0x8000_0002u32..=0x8000_0004 {
        for register in cpuid_leaf(leaf) {
            bytes.extend_from_slice(&register.to_le_bytes());
        }
    }
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

fn signature() -> (u32, u32, u32) {
    let eax = cpuid_leaf(1)[0];
    let mut family = (eax >> 8) & 0xf;
    let mut model = (eax >> 4) & 0xf;
    if family == 0xf {
        family += (eax >> 20) & 0xff;
    }
    if family == 0x6 || family == 0xf {
        model += ((eax >> 16) & 0xf) << 4;
    }
    (family, model, eax & 0xf)
}

fn flag(supported: bool) -> Option<Value> {
    Some(Value::Int(i32::from(supported)))
}

fn loadavg() -> Option<Value> {
    // struct loadavg { fixpt_t ldavg[3]; long fscale; }: no load average exists here.
    let mut bytes = vec![0u8; 24];
    bytes[16..24].copy_from_slice(&2048i64.to_le_bytes());
    Some(Value::Raw(bytes))
}

fn swapusage() -> Option<Value> {
    let (_, _, total, available) = memory();
    let physical = memory().0;
    let swap_total = total.saturating_sub(physical);
    let swap_available = available.saturating_sub(memory().1).min(swap_total);
    // struct xsw_usage { u64 total, avail, used; u32 pagesize; bool encrypted; }
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(&swap_total.to_le_bytes());
    bytes.extend_from_slice(&swap_available.to_le_bytes());
    bytes.extend_from_slice(&swap_total.saturating_sub(swap_available).to_le_bytes());
    bytes.extend_from_slice(&4096u32.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    Some(Value::Raw(bytes))
}

macro_rules! e {
    ($name:literal, $mib:expr, $value:expr) => {
        Entry {
            name: $name,
            mib: $mib,
            value: $value,
        }
    };
}

fn entries() -> Vec<Entry> {
    vec![
        e!("kern.ostype", &[CTL_KERN, 1], || Some(Value::Text(
            "Darwin".into()
        ))),
        e!("kern.osrelease", &[CTL_KERN, 2], || Some(Value::Text(
            OS_RELEASE.into()
        ))),
        e!("kern.osrevision", &[CTL_KERN, 3], || Some(Value::Int(
            199506
        ))),
        e!("kern.version", &[CTL_KERN, 4], || Some(Value::Text(
            OS_VERSION_STRING.into()
        ))),
        e!("kern.maxvnodes", &[CTL_KERN, 5], || Some(Value::Int(
            263168
        ))),
        e!("kern.maxproc", &[CTL_KERN, 6], || Some(Value::Int(12368))),
        e!("kern.maxfiles", &[CTL_KERN, 7], || Some(Value::Int(245760))),
        e!("kern.argmax", &[CTL_KERN, 8], || Some(Value::Int(
            1_048_576
        ))),
        e!("kern.securelevel", &[CTL_KERN, 9], || Some(Value::Int(0))),
        e!("kern.hostname", &[CTL_KERN, 10], || Some(Value::Text(
            hostname()
        ))),
        e!("kern.hostid", &[CTL_KERN, 11], || Some(Value::Int(0))),
        e!("kern.posix1version", &[CTL_KERN, 17], || Some(Value::Int(
            200112
        ))),
        e!("kern.ngroups", &[CTL_KERN, 18], || Some(Value::Int(16))),
        e!("kern.job_control", &[CTL_KERN, 19], || Some(Value::Int(1))),
        e!("kern.saved_ids", &[CTL_KERN, 20], || Some(Value::Int(1))),
        e!("kern.boottime", &[CTL_KERN, 21], || {
            let (seconds, micros) = boot_time();
            Some(Value::Time(seconds, micros))
        }),
        e!("kern.maxfilesperproc", &[CTL_KERN, 29], || Some(
            Value::Int(122880)
        )),
        e!("kern.maxprocperuid", &[CTL_KERN, 30], || Some(Value::Int(
            8000
        ))),
        e!("kern.nx", &[CTL_KERN, 60], || Some(Value::Int(1))),
        e!("kern.osversion", &[CTL_KERN, 65], || Some(Value::Text(
            OS_BUILD.into()
        ))),
        e!("kern.safeboot", &[CTL_KERN, 66], || Some(Value::Int(0))),
        e!("kern.osproductversion", &[], || Some(Value::Text(
            OS_PRODUCT.into()
        ))),
        e!("vm.loadavg", &[CTL_VM, 2], loadavg),
        e!("vm.swapusage", &[], swapusage),
        e!("hw.machine", &[CTL_HW, 1], || Some(Value::Text(
            "x86_64".into()
        ))),
        e!("hw.model", &[CTL_HW, 2], || Some(Value::Text(MODEL.into()))),
        e!("hw.ncpu", &[CTL_HW, 3], || Some(Value::Int(
            cpu().logical as i32
        ))),
        e!("hw.byteorder", &[CTL_HW, 4], || Some(Value::Int(1234))),
        e!("hw.physmem", &[CTL_HW, 5], || Some(clamp_int(memory().0))),
        e!("hw.usermem", &[CTL_HW, 6], || Some(clamp_int(memory().1))),
        e!("hw.pagesize", &[CTL_HW, 7], || Some(Value::Int(4096))),
        e!("hw.floatingpoint", &[CTL_HW, 11], || Some(Value::Int(1))),
        e!("hw.vectorunit", &[CTL_HW, 13], || Some(Value::Int(1))),
        e!("hw.cpufrequency", &[CTL_HW, 15], || cpu_hertz()
            .map(Value::Quad)),
        e!("hw.cachelinesize", &[CTL_HW, 16], || Some(Value::Quad(
            cpu().line
        ))),
        e!("hw.l1icachesize", &[CTL_HW, 17], || Some(Value::Quad(
            cpu().l1i
        ))),
        e!("hw.l1dcachesize", &[CTL_HW, 18], || Some(Value::Quad(
            cpu().l1d
        ))),
        e!("hw.l2cachesize", &[CTL_HW, 20], || Some(Value::Quad(
            cpu().l2
        ))),
        e!("hw.l3cachesize", &[CTL_HW, 22], || Some(Value::Quad(
            cpu().l3
        ))),
        e!("hw.tbfrequency", &[CTL_HW, 23], || Some(Value::Quad(
            1_000_000_000
        ))),
        e!("hw.memsize", &[CTL_HW, 24], || Some(Value::Quad(
            memory().0
        ))),
        e!("hw.availcpu", &[CTL_HW, 25], || Some(Value::Int(
            cpu().logical as i32
        ))),
        e!("hw.target", &[CTL_HW, 26], || Some(Value::Text(
            "adapter".into()
        ))),
        e!("hw.product", &[CTL_HW, 27], || Some(Value::Text(
            MODEL.into()
        ))),
        e!("hw.activecpu", &[], || Some(Value::Int(
            cpu().logical as i32
        ))),
        e!("hw.logicalcpu", &[], || Some(Value::Int(
            cpu().logical as i32
        ))),
        e!("hw.logicalcpu_max", &[], || Some(Value::Int(
            cpu().logical as i32
        ))),
        e!("hw.physicalcpu", &[], || Some(Value::Int(
            cpu().physical as i32
        ))),
        e!("hw.physicalcpu_max", &[], || Some(Value::Int(
            cpu().physical as i32
        ))),
        e!("hw.packages", &[], || Some(Value::Int(1))),
        e!("hw.cpu64bit_capable", &[], || Some(Value::Int(1))),
        e!("hw.cputype", &[], || Some(Value::Int(7))),
        e!("hw.cpusubtype", &[], || Some(Value::Int(3))),
        e!("hw.cpufrequency_max", &[], || cpu_hertz().map(Value::Quad)),
        e!("hw.cpufrequency_min", &[], || cpu_hertz().map(Value::Quad)),
        e!("hw.optional.x86_64", &[], || flag(true)),
        e!("hw.optional.sse", &[], || flag(
            std::arch::is_x86_feature_detected!("sse")
        )),
        e!("hw.optional.sse2", &[], || flag(
            std::arch::is_x86_feature_detected!("sse2")
        )),
        e!("hw.optional.sse3", &[], || flag(
            std::arch::is_x86_feature_detected!("sse3")
        )),
        e!("hw.optional.supplementalsse3", &[], || flag(
            std::arch::is_x86_feature_detected!("ssse3")
        )),
        e!("hw.optional.sse4_1", &[], || flag(
            std::arch::is_x86_feature_detected!("sse4.1")
        )),
        e!("hw.optional.sse4_2", &[], || flag(
            std::arch::is_x86_feature_detected!("sse4.2")
        )),
        e!("hw.optional.aes", &[], || flag(
            std::arch::is_x86_feature_detected!("aes")
        )),
        e!("hw.optional.avx1_0", &[], || flag(
            std::arch::is_x86_feature_detected!("avx")
        )),
        e!("hw.optional.avx2_0", &[], || flag(
            std::arch::is_x86_feature_detected!("avx2")
        )),
        e!("machdep.cpu.vendor", &[], || Some(
            Value::Text(cpu_vendor())
        )),
        e!("machdep.cpu.brand_string", &[], || Some(Value::Text(
            cpu_brand()
        ))),
        e!("machdep.cpu.family", &[], || Some(Value::Int(
            signature().0 as i32
        ))),
        e!("machdep.cpu.model", &[], || Some(Value::Int(
            signature().1 as i32
        ))),
        e!("machdep.cpu.stepping", &[], || Some(Value::Int(
            signature().2 as i32
        ))),
        e!("machdep.cpu.core_count", &[], || Some(Value::Int(
            cpu().physical as i32
        ))),
        e!("machdep.cpu.thread_count", &[], || Some(Value::Int(
            cpu().logical as i32
        ))),
        e!("user.cs_path", &[CTL_USER, 1], || Some(Value::Text(
            "/usr/bin:/bin:/usr/sbin:/sbin".into()
        ))),
        e!("user.bc_base_max", &[CTL_USER, 2], || Some(Value::Int(99))),
        e!("user.bc_dim_max", &[CTL_USER, 3], || Some(Value::Int(2048))),
        e!("user.bc_scale_max", &[CTL_USER, 4], || Some(Value::Int(99))),
        e!("user.bc_string_max", &[CTL_USER, 5], || Some(Value::Int(
            1000
        ))),
        e!("user.coll_weights_max", &[CTL_USER, 6], || Some(
            Value::Int(2)
        )),
        e!("user.expr_nest_max", &[CTL_USER, 7], || Some(Value::Int(
            32
        ))),
        e!("user.re_dup_max", &[CTL_USER, 9], || Some(Value::Int(255))),
        e!("user.posix2_version", &[CTL_USER, 10], || Some(Value::Int(
            200112
        ))),
        e!("user.posix2_c_bind", &[CTL_USER, 11], || Some(Value::Int(
            200112
        ))),
        e!("user.posix2_c_dev", &[CTL_USER, 12], || Some(Value::Int(
            200112
        ))),
        e!("user.posix2_char_term", &[CTL_USER, 13], || Some(
            Value::Int(200112)
        )),
        e!("hw.optional.floatingpoint", &[], || flag(true)),
        e!("hw.optional.mmx", &[], || flag(
            std::arch::is_x86_feature_detected!("mmx")
        )),
        e!("user.line_max", &[CTL_USER, 8], || Some(Value::Int(2048))),
        e!("user.stream_max", &[CTL_USER, 19], || Some(Value::Int(16))),
        e!("user.tzname_max", &[CTL_USER, 20], || Some(Value::Int(255))),
    ]
}

fn table() -> &'static Vec<Resolved> {
    static TABLE: OnceLock<Vec<Resolved>> = OnceLock::new();
    TABLE.get_or_init(|| {
        entries()
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let mib = if entry.mib.is_empty() {
                    let top = match entry.name.split('.').next() {
                        Some("kern") => CTL_KERN,
                        Some("vm") => CTL_VM,
                        Some("hw") => CTL_HW,
                        Some("machdep") => CTL_MACHDEP,
                        _ => CTL_USER,
                    };
                    vec![top, FIRST_DYNAMIC + index as i32]
                } else {
                    entry.mib.to_vec()
                };
                Resolved {
                    name: entry.name,
                    mib,
                    value: entry.value,
                }
            })
            .collect()
    })
}

fn encode(value: &Value) -> Vec<u8> {
    match value {
        Value::Int(v) => v.to_le_bytes().to_vec(),
        Value::Quad(v) => v.to_le_bytes().to_vec(),
        Value::Text(text) => {
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(0);
            bytes
        }
        Value::Time(seconds, micros) => {
            let mut bytes = seconds.to_le_bytes().to_vec();
            bytes.extend_from_slice(&micros.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]);
            bytes
        }
        Value::Raw(bytes) => bytes.clone(),
    }
}

/// Copies a value out following `sysctl(3)`: a NULL buffer asks for the size, a
/// short buffer receives what fits and the full size is reported with ENOMEM.
fn serve(
    entry: &Resolved,
    old: *mut u8,
    old_length: *mut usize,
    new: *const u8,
    new_length: usize,
) -> i32 {
    let Some(value) = (entry.value)() else {
        return ENOENT;
    };
    if !new.is_null() || new_length != 0 {
        return EPERM;
    }
    let data = encode(&value);
    if old_length.is_null() {
        return if old.is_null() { 0 } else { EINVAL };
    }
    unsafe {
        if old.is_null() {
            *old_length = data.len();
            return 0;
        }
        let capacity = *old_length;
        let copied = capacity.min(data.len());
        std::ptr::copy_nonoverlapping(data.as_ptr(), old, copied);
        *old_length = data.len();
        if capacity < data.len() { ENOMEM } else { 0 }
    }
}

fn finish(code: i32) -> i32 {
    if code == 0 {
        0
    } else {
        thread_state::set_errno(code);
        -1
    }
}

fn find_name(name: &str) -> Option<&'static Resolved> {
    table().iter().find(|entry| entry.name == name)
}

extern "sysv64" fn sysctlbyname(
    name: *const c_char,
    old: *mut u8,
    old_length: *mut usize,
    new: *const u8,
    new_length: usize,
) -> i32 {
    if name.is_null() {
        return finish(EINVAL);
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    match find_name(&name) {
        Some(entry) => finish(serve(entry, old, old_length, new, new_length)),
        None => finish(ENOENT),
    }
}

extern "sysv64" fn sysctl(
    mib: *const i32,
    length: u32,
    old: *mut u8,
    old_length: *mut usize,
    new: *const u8,
    new_length: usize,
) -> i32 {
    if mib.is_null() || !(2..=12).contains(&length) {
        return finish(EINVAL);
    }
    let mib = unsafe { std::slice::from_raw_parts(mib, length as usize) };
    // sysctl.name2oid: the call libc itself uses to implement sysctlbyname.
    if mib == [0, 3] {
        return finish(name_to_oid(old, old_length, new, new_length));
    }
    match table().iter().find(|entry| entry.mib == mib) {
        Some(entry) => finish(serve(entry, old, old_length, new, new_length)),
        None => finish(ENOENT),
    }
}

fn name_to_oid(old: *mut u8, old_length: *mut usize, new: *const u8, new_length: usize) -> i32 {
    if new.is_null() || new_length == 0 || old_length.is_null() {
        return EINVAL;
    }
    let bytes = unsafe { std::slice::from_raw_parts(new, new_length) };
    let name = String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or(&[]));
    let Some(entry) = find_name(&name) else {
        return ENOENT;
    };
    let needed = entry.mib.len() * 4;
    unsafe {
        if old.is_null() {
            *old_length = needed;
            return 0;
        }
        let capacity = *old_length;
        for (index, part) in entry.mib.iter().enumerate() {
            if (index + 1) * 4 <= capacity {
                std::ptr::write_unaligned((old as *mut i32).add(index), *part);
            }
        }
        *old_length = needed;
        if capacity < needed { ENOMEM } else { 0 }
    }
}

extern "sysv64" fn sysctlnametomib(name: *const c_char, mib: *mut i32, size: *mut usize) -> i32 {
    if name.is_null() || mib.is_null() || size.is_null() {
        return finish(EINVAL);
    }
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let Some(entry) = find_name(&name) else {
        return finish(ENOENT);
    };
    unsafe {
        let capacity = *size;
        *size = entry.mib.len();
        if capacity < entry.mib.len() {
            return finish(ENOMEM);
        }
        for (index, part) in entry.mib.iter().enumerate() {
            std::ptr::write_unaligned(mib.add(index), *part);
        }
    }
    0
}

// ---- libproc ---------------------------------------------------------------

const PROC_ALL_PIDS: u32 = 1;
const PROC_PGRP_ONLY: u32 = 2;
const PROC_RUID_ONLY: u32 = 5;
const PROC_UID_ONLY: u32 = 4;
const PROC_PIDTBSDINFO: i32 = 3;
const PROC_PIDTASKINFO: i32 = 4;
const BSDINFO_SIZE: usize = 136;
const TASKINFO_SIZE: usize = 96;

fn own_pid() -> i32 {
    std::process::id() as i32
}

fn guest_path() -> Option<String> {
    let lookup = libc::symbol("_NSGetExecutablePath")?;
    let lookup: extern "sysv64" fn(*mut u8, *mut u32) -> i32 =
        unsafe { std::mem::transmute(lookup) };
    let mut buffer = vec![0u8; 4096];
    let mut size = buffer.len() as u32;
    if lookup(buffer.as_mut_ptr(), &mut size) != 0 {
        return None;
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

fn guest_name() -> String {
    guest_path()
        .and_then(|path| path.rsplit(['/', '\\']).next().map(str::to_string))
        .unwrap_or_default()
}

fn copy_text(text: &str, buffer: *mut u8, size: u32) -> i32 {
    if buffer.is_null() || size == 0 {
        thread_state::set_errno(EINVAL);
        return 0;
    }
    let length = text.len().min(size as usize - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), buffer, length);
        *buffer.add(length) = 0;
    }
    length as i32
}

extern "sysv64" fn proc_libversion(major: *mut i32, minor: *mut i32) -> i32 {
    unsafe {
        if !major.is_null() {
            *major = 1;
        }
        if !minor.is_null() {
            *minor = 1;
        }
    }
    0
}

extern "sysv64" fn proc_name(pid: i32, buffer: *mut u8, size: u32) -> i32 {
    if pid != own_pid() {
        thread_state::set_errno(ESRCH);
        return 0;
    }
    copy_text(&guest_name(), buffer, size)
}

extern "sysv64" fn proc_pidpath(pid: i32, buffer: *mut u8, size: u32) -> i32 {
    if pid != own_pid() {
        thread_state::set_errno(ESRCH);
        return 0;
    }
    match guest_path() {
        Some(path) => copy_text(&path, buffer, size),
        None => {
            thread_state::set_errno(ESRCH);
            0
        }
    }
}

/// Only the guest's own process exists, so every filter that can match reports it.
extern "sysv64" fn proc_listpids(kind: u32, info: u32, buffer: *mut u8, size: i32) -> i32 {
    let matches = match kind {
        PROC_ALL_PIDS => true,
        PROC_PGRP_ONLY => info as i32 == own_pid(),
        PROC_UID_ONLY | PROC_RUID_ONLY => info == 0,
        _ => false,
    };
    let wanted = if matches { 4 } else { 0 };
    if buffer.is_null() {
        return wanted;
    }
    if wanted == 0 || size < 4 {
        return 0;
    }
    unsafe { std::ptr::write_unaligned(buffer as *mut i32, own_pid()) };
    4
}

#[cfg(windows)]
fn process_times() -> (u64, u64, u64) {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    if ok == 0 {
        return (0, 0, 0);
    }
    // FILETIME counts 100 ns from 1601; convert the start to the Unix epoch.
    let start = ticks(creation).saturating_sub(116_444_736_000_000_000);
    (start, ticks(user) * 100, ticks(kernel) * 100)
}

#[cfg(not(windows))]
fn process_times() -> (u64, u64, u64) {
    (0, 0, 0)
}

fn process_memory() -> (u64, u64) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::GetCurrentProcess,
        };
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) } != 0 {
            return (
                counters.PagefileUsage as u64,
                counters.WorkingSetSize as u64,
            );
        }
    }
    (0, 0)
}

fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
    bytes[offset..offset + N].copy_from_slice(&value);
}

extern "sysv64" fn proc_pidinfo(
    pid: i32,
    flavor: i32,
    _argument: u64,
    buffer: *mut u8,
    size: i32,
) -> i32 {
    if pid != own_pid() {
        thread_state::set_errno(ESRCH);
        return 0;
    }
    let (needed, mut bytes) = match flavor {
        PROC_PIDTBSDINFO => (BSDINFO_SIZE, vec![0u8; BSDINFO_SIZE]),
        PROC_PIDTASKINFO => (TASKINFO_SIZE, vec![0u8; TASKINFO_SIZE]),
        _ => {
            thread_state::set_errno(EINVAL);
            return 0;
        }
    };
    if buffer.is_null() || (size as usize) < needed {
        thread_state::set_errno(EINVAL);
        return 0;
    }
    let (start, user, system) = process_times();
    if flavor == PROC_PIDTBSDINFO {
        // struct proc_bsdinfo: ids are root, matching the adapter's single identity.
        put(&mut bytes, 4, 2u32.to_le_bytes()); // pbi_status: SRUN
        put(&mut bytes, 12, (own_pid() as u32).to_le_bytes());
        put(&mut bytes, 16, 1u32.to_le_bytes()); // pbi_ppid
        let name = guest_name();
        let comm = name.as_bytes();
        bytes[48..48 + comm.len().min(15)].copy_from_slice(&comm[..comm.len().min(15)]);
        bytes[64..64 + comm.len().min(31)].copy_from_slice(&comm[..comm.len().min(31)]);
        put(&mut bytes, 100, (own_pid() as u32).to_le_bytes()); // pbi_pgid
        put(&mut bytes, 120, (start / 10_000_000).to_le_bytes());
        put(&mut bytes, 128, ((start % 10_000_000) / 10).to_le_bytes());
    } else {
        let (virtual_size, resident) = process_memory();
        put(&mut bytes, 0, virtual_size.to_le_bytes());
        put(&mut bytes, 8, resident.to_le_bytes());
        put(&mut bytes, 16, user.to_le_bytes());
        put(&mut bytes, 24, system.to_le_bytes());
        put(&mut bytes, 84, 1i32.to_le_bytes()); // pti_threadnum
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, needed) };
    needed as i32
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_sysctl" => sysctl as *const () as u64,
        "_sysctlbyname" => sysctlbyname as *const () as u64,
        "_sysctlnametomib" => sysctlnametomib as *const () as u64,
        "_proc_libversion" => proc_libversion as *const () as u64,
        "_proc_name" => proc_name as *const () as u64,
        "_proc_pidpath" => proc_pidpath as *const () as u64,
        "_proc_listpids" => proc_listpids as *const () as u64,
        "_proc_pidinfo" => proc_pidinfo as *const () as u64,
        _ => return None,
    })
}
