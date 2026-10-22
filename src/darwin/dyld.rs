//! dyld image introspection over the guest images the runtime has mapped.
//! Guest headers and slides are real mapped addresses, not host modules.
use std::{
    ffi::{CString, c_char, c_void},
    sync::Mutex,
};

struct Image {
    name: CString,
    header: u64,
    slide: i64,
    unwind: UnwindImage,
}

#[derive(Clone, Default)]
pub struct UnwindImage {
    pub ranges: Vec<(u64, u64)>,
    pub dwarf: (u64, u64),
    pub compact: (u64, u64),
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct UnwindSections {
    header: u64,
    dwarf: u64,
    dwarf_size: u64,
    compact: u64,
    compact_size: u64,
}

static IMAGES: Mutex<Vec<Image>> = Mutex::new(Vec::new());

fn convert(images: Vec<(String, u64, i64, UnwindImage)>) -> Vec<Image> {
    images
        .into_iter()
        .filter_map(|(name, header, slide, unwind)| {
            CString::new(name).ok().map(|name| Image {
                name,
                header,
                slide,
                unwind,
            })
        })
        .collect()
}

pub fn begin(images: Vec<(String, u64, i64, UnwindImage)>) {
    if let Ok(mut registry) = IMAGES.lock() {
        *registry = convert(images);
    }
}

/// Register images loaded after process start. `CString` buffers stay put when
/// the vector grows, so pointers returned by the introspection adapters remain
/// valid for the life of the process.
pub fn append(images: Vec<(String, u64, i64, UnwindImage)>) {
    if let Ok(mut registry) = IMAGES.lock() {
        registry.extend(convert(images));
    }
}

extern "sysv64" fn image_count() -> u32 {
    IMAGES.lock().map_or(0, |v| v.len() as u32)
}

extern "sysv64" fn image_header(index: u32) -> *const c_void {
    IMAGES
        .lock()
        .ok()
        .and_then(|v| v.get(index as usize).map(|i| i.header as *const c_void))
        .unwrap_or(std::ptr::null())
}

extern "sysv64" fn image_name(index: u32) -> *const c_char {
    IMAGES
        .lock()
        .ok()
        .and_then(|v| v.get(index as usize).map(|i| i.name.as_ptr()))
        .unwrap_or(std::ptr::null())
}

extern "sysv64" fn image_slide(index: u32) -> isize {
    IMAGES
        .lock()
        .ok()
        .and_then(|v| v.get(index as usize).map(|i| i.slide as isize))
        .unwrap_or(0)
}

fn containing(images: &[Image], address: u64) -> Option<&Image> {
    images.iter().find(|image| {
        image
            .unwind
            .ranges
            .iter()
            .any(|&(start, end)| start <= address && address < end)
    })
}

unsafe extern "sysv64" fn find_unwind_sections(address: u64, output: *mut UnwindSections) -> bool {
    if output.is_null() {
        return false;
    }
    let Ok(images) = IMAGES.lock() else {
        return false;
    };
    let Some(image) = containing(&images, address) else {
        return false;
    };
    unsafe {
        output.write_unaligned(UnwindSections {
            header: image.header,
            dwarf: image.unwind.dwarf.0,
            dwarf_size: image.unwind.dwarf.1,
            compact: image.unwind.compact.0,
            compact_size: image.unwind.compact.1,
        });
    }
    true
}

extern "sysv64" fn header_containing(address: u64) -> u64 {
    IMAGES
        .lock()
        .ok()
        .and_then(|images| containing(&images, address).map(|i| i.header))
        .unwrap_or(0)
}
extern "sysv64" fn path_containing(address: u64) -> *const c_char {
    IMAGES
        .lock()
        .ok()
        .and_then(|images| containing(&images, address).map(|i| i.name.as_ptr()))
        .unwrap_or(std::ptr::null())
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "__dyld_find_unwind_sections" => find_unwind_sections as *const () as u64,
        "_dyld_image_header_containing_address" => header_containing as *const () as u64,
        "_dyld_image_path_containing_address" => path_containing as *const () as u64,
        "_dyld_image_count" | "__dyld_image_count" => image_count as *const () as u64,
        "_dyld_get_image_header" | "__dyld_get_image_header" => image_header as *const () as u64,
        "_dyld_get_image_name" | "__dyld_get_image_name" => image_name as *const () as u64,
        "_dyld_get_image_vmaddr_slide" | "__dyld_get_image_vmaddr_slide" => {
            image_slide as *const () as u64
        }
        _ => return None,
    })
}
