//! Windows VM ownership and protections. No Mach-O parsing occurs here.
use crate::{Result, inspect::Image};
use std::{ffi::c_void, ptr};
use windows_sys::Win32::System::{
    Diagnostics::Debug::FlushInstructionCache, Memory::*, Threading::GetCurrentProcess,
};

pub struct Mapping {
    base: *mut u8,
    pub low: u64,
    len: usize,
    ranges: Vec<(u64, u64)>,
}
impl Mapping {
    pub fn new(image: &Image) -> Result<Self> {
        let segments: Vec<_> = image
            .segments
            .iter()
            .filter(|s| s.name != "__PAGEZERO" && s.vmsize != 0)
            .collect();
        let low = segments
            .iter()
            .map(|s| s.vmaddr)
            .min()
            .ok_or("no loadable segments")?;
        let high = segments
            .iter()
            .map(|s| {
                s.vmaddr
                    .checked_add(s.vmsize)
                    .ok_or_else(|| "segment address overflow".to_string())
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap();
        let len = high.checked_sub(low).ok_or("invalid VM span")?;
        if len == 0 || len > 256 * 1024 * 1024 {
            return Err("VM span exceeds 256 MiB limit".into());
        }
        for (i, s) in segments.iter().enumerate() {
            crate::inspect::range(&image.data, s.fileoff, s.filesize)?;
            if s.filesize > s.vmsize {
                return Err("segment file size exceeds VM size".into());
            }
            if s.vmaddr % 4096 != 0 {
                return Err("execution requires page-aligned segment starts".into());
            }
            if s.initprot & !7 != 0 || s.initprot & !s.maxprot != 0 || s.initprot & 6 == 6 {
                return Err("invalid or writable-executable segment protection".into());
            }
            for t in segments.iter().take(i) {
                if s.vmaddr < t.vmaddr + ((t.vmsize + 4095) & !4095)
                    && t.vmaddr < s.vmaddr + ((s.vmsize + 4095) & !4095)
                {
                    return Err("overlapping VM segment pages".into());
                }
            }
        }
        // Reserve inaccessible gaps; commit only actual mapped segments as RW
        // during relocation. Execution permissions are installed afterwards.
        let base = unsafe { VirtualAlloc(ptr::null(), len as usize, MEM_RESERVE, PAGE_NOACCESS) }
            as *mut u8;
        if base.is_null() {
            return Err(format!(
                "VirtualAlloc reserve: {}",
                std::io::Error::last_os_error()
            ));
        }
        let m = Self {
            base,
            low,
            len: len as usize,
            ranges: segments.iter().map(|s| (s.vmaddr, s.vmsize)).collect(),
        };
        for s in segments {
            let dest = unsafe { base.add((s.vmaddr - low) as usize) };
            if unsafe {
                VirtualAlloc(
                    dest as *const c_void,
                    s.vmsize as usize,
                    MEM_COMMIT,
                    PAGE_READWRITE,
                )
            }
            .is_null()
            {
                return Err(format!(
                    "VirtualAlloc commit: {}",
                    std::io::Error::last_os_error()
                ));
            }
            unsafe {
                ptr::copy_nonoverlapping(
                    image.data.as_ptr().add(s.fileoff as usize),
                    dest,
                    s.filesize as usize,
                );
            }
        }
        Ok(m)
    }
    pub fn pointer(&self, address: u64, size: u64) -> Result<*mut u8> {
        let end = address.checked_add(size).ok_or("VM address overflow")?;
        if !self
            .ranges
            .iter()
            .any(|&(a, n)| address >= a && end <= a + n)
        {
            return Err(format!(
                "address {address:#x}+{size:#x} is outside mapped segments"
            ));
        }
        Ok(unsafe { self.base.add((address - self.low) as usize) })
    }
    pub fn relocated(&self, address: u64) -> Result<u64> {
        Ok(self.pointer(address, 1)? as u64)
    }
    pub fn protect(&self, image: &Image) -> Result<()> {
        for s in &image.segments {
            if s.name == "__PAGEZERO" || s.vmsize == 0 {
                continue;
            }
            let final_prot = if s.flags & 0x10 != 0 {
                s.initprot & !2
            } else {
                s.initprot
            };
            let prot = match final_prot {
                0 => PAGE_NOACCESS,
                1 => PAGE_READONLY,
                2 | 3 => PAGE_READWRITE,
                4 => PAGE_EXECUTE,
                5 => PAGE_EXECUTE_READ,
                _ => return Err("unsupported protection".into()),
            };
            let mut old = 0;
            if unsafe {
                VirtualProtect(
                    self.pointer(s.vmaddr, s.vmsize)? as *const c_void,
                    s.vmsize as usize,
                    prot,
                    &mut old,
                )
            } == 0
            {
                return Err(format!(
                    "VirtualProtect: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        if unsafe {
            FlushInstructionCache(GetCurrentProcess(), self.base as *const c_void, self.len)
        } == 0
        {
            return Err(format!(
                "FlushInstructionCache: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            VirtualFree(self.base as *mut c_void, 0, MEM_RELEASE);
        }
    }
}
