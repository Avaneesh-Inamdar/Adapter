//! Minimal Clang block runtime for XPC and dispatch handler blocks.
//!
//! Layout and flag values follow libclosure's `Block_private.h`: block copy and
//! release, `__block` variable (byref) promotion through `_Block_object_assign`
//! and `_Block_object_dispose`, and invocation of non-Objective-C blocks.
//! Weak and garbage-collected variants are treated like their strong form.
use std::{
    alloc::{Layout, alloc, dealloc},
    ffi::c_void,
};

const BLOCK_DEALLOCATING: u32 = 0x0002;
const BLOCK_REFCOUNT_MASK: u32 = 0xfffe;
const BLOCK_NEEDS_FREE: u32 = 1 << 24;
const BLOCK_HAS_COPY_DISPOSE: u32 = 1 << 25;
const BLOCK_IS_GLOBAL: u32 = 1 << 28;

const FIELD_IS_OBJECT: i32 = 3;
const FIELD_IS_BLOCK: i32 = 7;
const FIELD_IS_BYREF: i32 = 8;
const FIELD_IS_WEAK: i32 = 16;
const FIELD_BYREF_CALLER: i32 = 128;
const FIELD_ALL: i32 = 3 | 7 | 8 | 16 | 128;

const BYREF_REFCOUNT_MASK: i32 = 0xffff;
const BYREF_NEEDS_FREE: i32 = 1 << 24;
const BYREF_HAS_COPY_DISPOSE: i32 = 1 << 25;

/// Header of a `__block` variable; copy/dispose helpers follow it when
/// `BYREF_HAS_COPY_DISPOSE` is set, then the variable itself.
#[repr(C)]
struct Byref {
    isa: usize,
    forwarding: *mut Byref,
    flags: i32,
    size: u32,
}

#[repr(C)]
struct ByrefHelpers {
    keep: usize,
    destroy: usize,
}

#[repr(C)]
struct Descriptor {
    reserved: usize,
    size: usize,
}

#[repr(C)]
struct Block {
    isa: usize,
    flags: u32,
    reserved: u32,
    invoke: usize,
    descriptor: usize,
}

#[unsafe(no_mangle)]
static mut CONCRETE_STACK: [usize; 4] = [0; 4];
#[unsafe(no_mangle)]
static mut CONCRETE_GLOBAL: [usize; 4] = [0; 4];

fn stack_isa() -> usize {
    std::ptr::addr_of!(CONCRETE_STACK) as usize
}

fn global_isa() -> usize {
    std::ptr::addr_of!(CONCRETE_GLOBAL) as usize
}

pub fn symbol(name: &str) -> Option<u64> {
    Some(match name {
        "_Block_copy" | "__Block_copy" => block_copy as *const () as u64,
        "_Block_release" | "__Block_release" => block_release as *const () as u64,
        "_Block_object_assign" | "__Block_object_assign" => block_object_assign as *const () as u64,
        "_Block_object_dispose" | "__Block_object_dispose" => {
            block_object_dispose as *const () as u64
        }
        "_NSConcreteStackBlock" | "__NSConcreteStackBlock" => stack_isa() as u64,
        "_NSConcreteGlobalBlock" | "__NSConcreteGlobalBlock" => global_isa() as u64,
        _ => return None,
    })
}

unsafe extern "sysv64" fn block_copy(block: *const c_void) -> *mut c_void {
    if block.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let source = block as *const Block;
        let flags = (*source).flags;
        if flags & BLOCK_NEEDS_FREE != 0 {
            (*source.cast_mut()).flags = flags.wrapping_add(2);
            return block.cast_mut();
        }
        if flags & BLOCK_IS_GLOBAL != 0 || (*source).isa == global_isa() {
            return block.cast_mut();
        }
        if (*source).isa != stack_isa() {
            return block.cast_mut();
        }
        let descriptor = (*source).descriptor as *const Descriptor;
        if descriptor.is_null() {
            return std::ptr::null_mut();
        }
        let mut size = (*descriptor).size;
        if size < std::mem::size_of::<Block>() {
            size = std::mem::size_of::<Block>();
        }
        let align = std::mem::align_of::<Block>().max(16);
        let layout = Layout::from_size_align_unchecked(size, align);
        let heap = alloc(layout) as *mut Block;
        if heap.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(source as *const u8, heap as *mut u8, size);
        let flags =
            ((*heap).flags & !(BLOCK_REFCOUNT_MASK | BLOCK_DEALLOCATING)) | BLOCK_NEEDS_FREE | 2;
        (*heap).flags = flags;
        (*heap).isa = malloc_isa();
        if flags & BLOCK_HAS_COPY_DISPOSE != 0 {
            let copy = *((*heap).descriptor as *const usize).add(2);
            if copy != 0 {
                let copy: unsafe extern "sysv64" fn(*mut c_void, *const c_void) =
                    std::mem::transmute(copy);
                copy(heap.cast(), block);
            }
        }
        heap.cast()
    }
}

fn byref_layout(size: usize) -> Layout {
    unsafe { Layout::from_size_align_unchecked(size.max(std::mem::size_of::<Byref>()), 16) }
}

/// Promotes a stack `__block` variable to the heap, or retains the heap copy.
unsafe fn byref_copy(source: *mut Byref) -> *mut Byref {
    unsafe {
        let forwarding = (*source).forwarding;
        if (*forwarding).flags & BYREF_REFCOUNT_MASK == 0 {
            let size = (*source).size as usize;
            let copy = alloc(byref_layout(size)) as *mut Byref;
            if copy.is_null() {
                return source;
            }
            (*copy).isa = 0;
            // Logical refcount two: one for the caller, one for the stack original.
            (*copy).flags = (*source).flags | BYREF_NEEDS_FREE | 4;
            (*copy).forwarding = copy;
            (*source).forwarding = copy;
            (*copy).size = (*source).size;
            if (*source).flags & BYREF_HAS_COPY_DISPOSE != 0 {
                let from = source.add(1) as *const ByrefHelpers;
                let to = copy.add(1) as *mut ByrefHelpers;
                (*to).keep = (*from).keep;
                (*to).destroy = (*from).destroy;
                if (*from).keep != 0 {
                    let keep: unsafe extern "sysv64" fn(*mut Byref, *mut Byref) =
                        std::mem::transmute((*from).keep);
                    keep(copy, source);
                }
            } else {
                std::ptr::copy_nonoverlapping(
                    source.add(1) as *const u8,
                    copy.add(1) as *mut u8,
                    size.saturating_sub(std::mem::size_of::<Byref>()),
                );
            }
        } else if (*forwarding).flags & BYREF_NEEDS_FREE != 0 {
            let flags =
                &*(std::ptr::addr_of!((*forwarding).flags) as *const std::sync::atomic::AtomicI32);
            let mut old = flags.load(std::sync::atomic::Ordering::Relaxed);
            loop {
                if old & BYREF_REFCOUNT_MASK == BYREF_REFCOUNT_MASK {
                    break;
                }
                match flags.compare_exchange(
                    old,
                    old + 2,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(current) => old = current,
                }
            }
        }
        (*source).forwarding
    }
}

unsafe fn byref_release(variable: *mut Byref) {
    unsafe {
        let shared = (*variable).forwarding;
        if (*shared).flags & BYREF_NEEDS_FREE == 0 {
            return;
        }
        let flags = &*(std::ptr::addr_of!((*shared).flags) as *const std::sync::atomic::AtomicI32);
        let mut old = flags.load(std::sync::atomic::Ordering::Relaxed);
        let last = loop {
            if old & BYREF_REFCOUNT_MASK == BYREF_REFCOUNT_MASK || old & 0xfffe == 0 {
                break false;
            }
            let mut next = old - 2;
            let last = next & 0xfffe == 0;
            if last {
                next |= 1;
            }
            match flags.compare_exchange(
                old,
                next,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::Relaxed,
            ) {
                Ok(_) => break last,
                Err(current) => old = current,
            }
        };
        if !last {
            return;
        }
        if (*shared).flags & BYREF_HAS_COPY_DISPOSE != 0 {
            let helpers = shared.add(1) as *const ByrefHelpers;
            if (*helpers).destroy != 0 {
                let destroy: unsafe extern "sysv64" fn(*mut Byref) =
                    std::mem::transmute((*helpers).destroy);
                destroy(shared);
            }
        }
        dealloc(shared.cast(), byref_layout((*shared).size as usize));
    }
}

fn objc_call(name: &str, object: *const c_void) {
    if let Some(function) = crate::objc::symbol(name) {
        let function: unsafe extern "sysv64" fn(*const c_void) =
            unsafe { std::mem::transmute(function) };
        unsafe { function(object) };
    }
}

/// Called by compiler-generated copy helpers for captured blocks, objects and
/// `__block` variables.
unsafe extern "sysv64" fn block_object_assign(
    destination: *mut usize,
    object: *const c_void,
    flags: i32,
) {
    unsafe {
        match flags & FIELD_ALL {
            FIELD_IS_OBJECT => {
                objc_call("_objc_retain", object);
                *destination = object as usize;
            }
            FIELD_IS_BLOCK => *destination = block_copy(object) as usize,
            f if f == FIELD_IS_BYREF || f == FIELD_IS_BYREF | FIELD_IS_WEAK => {
                *destination = byref_copy(object as *mut Byref) as usize
            }
            f if f & FIELD_BYREF_CALLER != 0 => *destination = object as usize,
            _ => {}
        }
    }
}

unsafe extern "sysv64" fn block_object_dispose(object: *const c_void, flags: i32) {
    unsafe {
        match flags & FIELD_ALL {
            FIELD_IS_OBJECT => objc_call("_objc_release", object),
            FIELD_IS_BLOCK => block_release(object),
            f if f == FIELD_IS_BYREF || f == FIELD_IS_BYREF | FIELD_IS_WEAK => {
                byref_release(object as *mut Byref)
            }
            _ => {}
        }
    }
}

#[unsafe(no_mangle)]
static mut CONCRETE_MALLOC: [usize; 4] = [0; 4];

fn malloc_isa() -> usize {
    std::ptr::addr_of!(CONCRETE_MALLOC) as usize
}

unsafe extern "sysv64" fn block_release(block: *const c_void) {
    if block.is_null() {
        return;
    }
    unsafe {
        let header = block as *const Block;
        let flags = (*header).flags;
        if flags & BLOCK_IS_GLOBAL != 0 || flags & BLOCK_NEEDS_FREE == 0 {
            return;
        }
        let next = flags.wrapping_sub(2);
        if next & BLOCK_REFCOUNT_MASK != 0 {
            (*header.cast_mut()).flags = next;
            return;
        }
        if next & BLOCK_HAS_COPY_DISPOSE != 0 {
            let dispose = *((*header).descriptor as *const usize).add(3);
            if dispose != 0 {
                let dispose: unsafe extern "sysv64" fn(*const c_void) =
                    std::mem::transmute(dispose);
                dispose(block);
            }
        }
        let descriptor = (*header).descriptor as *const Descriptor;
        let size = (*descriptor).size.max(std::mem::size_of::<Block>());
        let align = std::mem::align_of::<Block>().max(16);
        dealloc(
            block.cast_mut().cast(),
            Layout::from_size_align_unchecked(size, align),
        );
    }
}

/// Copies a stack or heap block and returns an owned reference.
///
/// # Safety
/// `block` must be null or a valid block pointer.
pub unsafe fn copy(block: *const c_void) -> *mut c_void {
    unsafe { block_copy(block) }
}

/// Invokes a copied block with two arguments and returns its Boolean result.
///
/// # Safety
/// `block` must be a live block whose invoke function accepts these arguments.
pub unsafe fn invoke_two(block: *mut c_void, first: usize, second: *mut c_void) -> bool {
    if block.is_null() {
        return false;
    }
    unsafe {
        let invoke = (*(block as *const Block)).invoke;
        if invoke == 0 {
            return false;
        }
        let invoke: unsafe extern "sysv64" fn(*mut c_void, usize, *mut c_void) -> bool =
            std::mem::transmute(invoke);
        invoke(block, first, second)
    }
}

/// Invokes a copied block with one object argument.
///
/// # Safety
/// `block` must be a live block whose invoke function accepts one pointer.
pub unsafe fn invoke_one(block: *mut c_void, argument: *mut c_void) {
    if block.is_null() {
        return;
    }
    unsafe {
        let invoke = (*(block as *const Block)).invoke;
        if invoke != 0 {
            let invoke: unsafe extern "sysv64" fn(*mut c_void, *mut c_void) =
                std::mem::transmute(invoke);
            invoke(block, argument);
        }
    }
}

/// Allocates a zeroed heap block of `size` bytes with one reference and the
/// copy/dispose descriptor flag, for blocks whose invoke function lives in the host.
///
/// # Safety
/// `descriptor` must point at a descriptor whose size field equals `size` and whose
/// dispose slot (if non-zero) is a valid `extern "sysv64" fn(*const c_void)`.
pub unsafe fn new_heap_block(size: usize, invoke: usize, descriptor: usize) -> *mut c_void {
    unsafe {
        let layout = Layout::from_size_align_unchecked(size.max(std::mem::size_of::<Block>()), 16);
        let block = std::alloc::alloc_zeroed(layout) as *mut Block;
        if block.is_null() {
            return std::ptr::null_mut();
        }
        (*block).isa = malloc_isa();
        (*block).flags = BLOCK_NEEDS_FREE | BLOCK_HAS_COPY_DISPOSE | 2;
        (*block).invoke = invoke;
        (*block).descriptor = descriptor;
        block.cast()
    }
}

/// Returns a block's invoke function address, or 0 for a null block.
///
/// # Safety
/// `block` must be null or point at a readable block header.
pub unsafe fn invoke_address(block: *const c_void) -> usize {
    if block.is_null() {
        return 0;
    }
    unsafe { (*(block as *const Block)).invoke }
}

/// Invokes a block that takes no arguments.
///
/// # Safety
/// `block` must be a live block whose invoke function takes only the block.
pub unsafe fn invoke_void(block: *const c_void) {
    unsafe {
        let invoke = invoke_address(block);
        if invoke != 0 {
            let invoke: unsafe extern "sysv64" fn(*const c_void) = std::mem::transmute(invoke);
            invoke(block);
        }
    }
}

/// Invokes a block that takes one `size_t` argument.
///
/// # Safety
/// `block` must be a live block whose invoke function accepts one integer.
pub unsafe fn invoke_index(block: *const c_void, index: usize) {
    unsafe {
        let invoke = invoke_address(block);
        if invoke != 0 {
            let invoke: unsafe extern "sysv64" fn(*const c_void, usize) =
                std::mem::transmute(invoke);
            invoke(block, index);
        }
    }
}

/// Releases a block previously returned by `_Block_copy`.
///
/// # Safety
/// `block` must be null or a block whose copies have all been released.
pub unsafe fn release(block: *mut c_void) {
    unsafe { block_release(block) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct TestBlock {
        block: Block,
        captured: i64,
    }

    static mut INVOKED: i64 = 0;

    unsafe extern "sysv64" fn invoke(block: *mut c_void, _argument: *mut c_void) {
        unsafe {
            INVOKED = (*(block as *const TestBlock)).captured;
        }
    }

    #[test]
    fn copies_stack_blocks_and_releases_them() {
        let descriptor = Descriptor {
            reserved: 0,
            size: std::mem::size_of::<TestBlock>(),
        };
        let original = TestBlock {
            block: Block {
                isa: stack_isa(),
                flags: 0,
                reserved: 0,
                invoke: invoke as *const () as usize,
                descriptor: &descriptor as *const Descriptor as usize,
            },
            captured: 41,
        };
        let copy = unsafe { block_copy((&original as *const TestBlock).cast()) };
        assert!(!copy.is_null());
        assert_ne!(copy, (&original as *const TestBlock).cast_mut().cast());
        unsafe {
            assert_eq!((*(copy as *const TestBlock)).captured, 41);
            assert_eq!((*(copy as *const TestBlock)).block.isa, malloc_isa());
            block_release(copy);
        }
    }

    #[test]
    fn retains_a_heap_block_until_the_last_release() {
        let descriptor = Descriptor {
            reserved: 0,
            size: std::mem::size_of::<TestBlock>(),
        };
        let original = TestBlock {
            block: Block {
                isa: stack_isa(),
                flags: 0,
                reserved: 0,
                invoke: invoke as *const () as usize,
                descriptor: &descriptor as *const Descriptor as usize,
            },
            captured: 7,
        };
        let copy = unsafe { block_copy((&original as *const TestBlock).cast()) };
        let again = unsafe { block_copy(copy) };
        assert_eq!(copy, again);
        unsafe {
            block_release(copy);
            assert_eq!((*(copy as *const TestBlock)).captured, 7);
            block_release(again);
        }
    }
}
