//! Minimal Clang block runtime for XPC handler blocks.
//!
//! Layout and flag values follow libclosure's `Block_private.h`; only copying,
//! releasing and invocation of non-Objective-C blocks are supported.
use std::{
    alloc::{Layout, alloc, dealloc},
    ffi::c_void,
};

const BLOCK_DEALLOCATING: u32 = 0x0002;
const BLOCK_REFCOUNT_MASK: u32 = 0xfffe;
const BLOCK_NEEDS_FREE: u32 = 1 << 24;
const BLOCK_HAS_COPY_DISPOSE: u32 = 1 << 25;
const BLOCK_IS_GLOBAL: u32 = 1 << 28;

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
