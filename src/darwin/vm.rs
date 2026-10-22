//! Mach VM adapters. The guest address space is the host address space, so an
//! allocation is a host reservation that the runtime records and can name.
//!
//! Only regions this module created are releasable: a guest passing an arbitrary
//! address gets `KERN_INVALID_ADDRESS` instead of freeing host memory it never
//! received from these calls.
use std::{collections::BTreeMap, ffi::c_void, ptr, sync::Mutex};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_NOACCESS,
    PAGE_PROTECTION_FLAGS, PAGE_READONLY, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    VirtualProtect,
};

const KERN_SUCCESS: i32 = 0;
const KERN_INVALID_ADDRESS: i32 = 1;
const KERN_PROTECTION_FAILURE: i32 = 2;
const KERN_NO_SPACE: i32 = 3;
const KERN_INVALID_ARGUMENT: i32 = 4;
const KERN_FAILURE: i32 = 5;

const MACH_TASK_SELF_NAME: u32 = 0x103;
const VM_FLAGS_ANYWHERE: i32 = 1;
const VM_PROT_READ: i32 = 1;
const VM_PROT_WRITE: i32 = 2;
const VM_PROT_EXECUTE: i32 = 4;
const PAGE: u64 = 4096;
const MAX_ALLOCATION: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Region {
    size: u64,
    protection: i32,
}

static REGIONS: Mutex<BTreeMap<u64, Region>> = Mutex::new(BTreeMap::new());

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_mach_vm_allocate" | "__mach_vm_allocate" => mach_vm_allocate as *const () as u64,
        "_mach_vm_deallocate" | "__mach_vm_deallocate" => mach_vm_deallocate as *const () as u64,
        "_mach_vm_protect" | "__mach_vm_protect" => mach_vm_protect as *const () as u64,
        "_vm_allocate" | "__vm_allocate" => vm_allocate as *const () as u64,
        "_vm_deallocate" | "__vm_deallocate" => vm_deallocate as *const () as u64,
        "_vm_protect" | "__vm_protect" => vm_protect as *const () as u64,
        _ => return None,
    })
}

fn page_round(size: u64) -> Option<u64> {
    size.checked_add(PAGE - 1).map(|value| value & !(PAGE - 1))
}

fn host_protection(protection: i32) -> Option<PAGE_PROTECTION_FLAGS> {
    if protection & !(VM_PROT_READ | VM_PROT_WRITE | VM_PROT_EXECUTE) != 0 {
        return None;
    }
    let value = match protection {
        0 => PAGE_NOACCESS,
        VM_PROT_READ => PAGE_READONLY,
        p if p == VM_PROT_READ | VM_PROT_WRITE => PAGE_READWRITE,
        VM_PROT_EXECUTE => PAGE_EXECUTE,
        p if p == VM_PROT_READ | VM_PROT_EXECUTE => PAGE_EXECUTE_READ,
        // Writable-executable mappings stay rejected, as the segment loader does.
        _ => return None,
    };
    Some(value)
}

fn allocate(task: u32, address: *mut u64, size: u64, flags: i32) -> i32 {
    if task != MACH_TASK_SELF_NAME || address.is_null() {
        return KERN_INVALID_ARGUMENT;
    }
    if flags != 0 && flags != VM_FLAGS_ANYWHERE {
        return KERN_INVALID_ARGUMENT;
    }
    let rounded = match page_round(size) {
        Some(value) if value != 0 => value,
        _ => return KERN_INVALID_ARGUMENT,
    };
    if rounded > MAX_ALLOCATION {
        return KERN_NO_SPACE;
    }
    let hint = unsafe { ptr::read_unaligned(address) };
    if flags == 0 && (hint == 0 || hint % PAGE != 0) {
        return KERN_INVALID_ARGUMENT;
    }
    let requested = if flags == 0 {
        hint as usize as *const c_void
    } else {
        ptr::null()
    };
    let base = unsafe {
        VirtualAlloc(
            requested,
            rounded as usize,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if base.is_null() {
        return KERN_NO_SPACE;
    }
    if flags == 0 && base as usize as u64 != hint {
        unsafe { VirtualFree(base, 0, MEM_RELEASE) };
        return KERN_NO_SPACE;
    }
    let mut regions = match REGIONS.lock() {
        Ok(guard) => guard,
        Err(_) => {
            unsafe { VirtualFree(base, 0, MEM_RELEASE) };
            return KERN_FAILURE;
        }
    };
    regions.insert(
        base as usize as u64,
        Region {
            size: rounded,
            protection: VM_PROT_READ | VM_PROT_WRITE,
        },
    );
    unsafe { ptr::write_unaligned(address, base as usize as u64) };
    KERN_SUCCESS
}

fn deallocate(task: u32, address: u64, size: u64) -> i32 {
    if task != MACH_TASK_SELF_NAME {
        return KERN_INVALID_ARGUMENT;
    }
    if size == 0 {
        return KERN_SUCCESS;
    }
    let rounded = match page_round(size) {
        Some(value) => value,
        None => return KERN_INVALID_ARGUMENT,
    };
    let mut regions = match REGIONS.lock() {
        Ok(guard) => guard,
        Err(_) => return KERN_FAILURE,
    };
    match regions.get(&address) {
        Some(region) if region.size == rounded => {}
        Some(_) => return KERN_INVALID_ARGUMENT,
        None => return KERN_INVALID_ADDRESS,
    }
    let freed = unsafe { VirtualFree(address as usize as *mut c_void, 0, MEM_RELEASE) };
    if freed == 0 {
        return KERN_FAILURE;
    }
    regions.remove(&address);
    KERN_SUCCESS
}

fn protect(task: u32, address: u64, size: u64, protection: i32) -> i32 {
    if task != MACH_TASK_SELF_NAME {
        return KERN_INVALID_ARGUMENT;
    }
    let page = match host_protection(protection) {
        Some(value) => value,
        None => return KERN_INVALID_ARGUMENT,
    };
    let mut regions = match REGIONS.lock() {
        Ok(guard) => guard,
        Err(_) => return KERN_FAILURE,
    };
    let region = match regions.get_mut(&address) {
        Some(value) => value,
        None => return KERN_INVALID_ADDRESS,
    };
    if size == 0 || size > region.size {
        return KERN_INVALID_ARGUMENT;
    }
    let mut previous: PAGE_PROTECTION_FLAGS = 0;
    if unsafe {
        VirtualProtect(
            address as usize as *const c_void,
            size as usize,
            page,
            &mut previous,
        )
    } == 0
    {
        return KERN_PROTECTION_FAILURE;
    }
    region.protection = protection;
    KERN_SUCCESS
}

extern "sysv64" fn mach_vm_allocate(task: u32, address: *mut u64, size: u64, flags: i32) -> i32 {
    allocate(task, address, size, flags)
}

extern "sysv64" fn mach_vm_deallocate(task: u32, address: u64, size: u64) -> i32 {
    deallocate(task, address, size)
}

extern "sysv64" fn mach_vm_protect(
    task: u32,
    address: u64,
    size: u64,
    _set_maximum: i32,
    protection: i32,
) -> i32 {
    protect(task, address, size, protection)
}

extern "sysv64" fn vm_allocate(task: u32, address: *mut u64, size: u64, flags: i32) -> i32 {
    allocate(task, address, size, flags)
}

extern "sysv64" fn vm_deallocate(task: u32, address: u64, size: u64) -> i32 {
    deallocate(task, address, size)
}

extern "sysv64" fn vm_protect(
    task: u32,
    address: u64,
    size: u64,
    _set_maximum: i32,
    protection: i32,
) -> i32 {
    protect(task, address, size, protection)
}
