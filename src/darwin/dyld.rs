//! dyld image introspection and `dlsym` over the adapters' own namespace.
//! Guest image headers and slides are real mapped addresses, not host modules.
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::OnceLock,
};

struct Image {
    name: CString,
    header: u64,
    slide: i64,
}

static IMAGES: OnceLock<Vec<Image>> = OnceLock::new();

pub fn begin(images: Vec<(String, u64, i64)>) {
    let images = images
        .into_iter()
        .filter_map(|(name, header, slide)| {
            CString::new(name).ok().map(|name| Image {
                name,
                header,
                slide,
            })
        })
        .collect();
    let _ = IMAGES.set(images);
}

fn image(index: u32) -> Option<&'static Image> {
    IMAGES.get()?.get(index as usize)
}

extern "sysv64" fn image_count() -> u32 {
    IMAGES.get().map_or(0, |v| v.len() as u32)
}

extern "sysv64" fn image_header(index: u32) -> *const c_void {
    image(index).map_or(std::ptr::null(), |i| i.header as *const c_void)
}

extern "sysv64" fn image_name(index: u32) -> *const c_char {
    image(index).map_or(std::ptr::null(), |i| i.name.as_ptr())
}

extern "sysv64" fn image_slide(index: u32) -> isize {
    image(index).map_or(0, |i| i.slide as isize)
}

unsafe extern "sysv64" fn dlsym(_handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    if symbol.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(name) = unsafe { CStr::from_ptr(symbol) }.to_str() else {
        return std::ptr::null_mut();
    };
    crate::darwin::symbol(name).map_or(std::ptr::null_mut(), |v| v as *mut c_void)
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_dyld_image_count" | "__dyld_image_count" => image_count as *const () as u64,
        "_dyld_get_image_header" | "__dyld_get_image_header" => image_header as *const () as u64,
        "_dyld_get_image_name" | "__dyld_get_image_name" => image_name as *const () as u64,
        "_dyld_get_image_vmaddr_slide" | "__dyld_get_image_vmaddr_slide" => {
            image_slide as *const () as u64
        }
        "_dlsym" => dlsym as *const () as u64,
        _ => return None,
    })
}
