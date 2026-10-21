#[cfg(windows)]
pub mod windows;

/// True when the address lies inside the calling thread's stack. Used only for
/// bounded Objective-C name reads, whose bytes may come from a guest frame.
pub fn stack_contains(address: u64) -> bool {
    #[cfg(windows)]
    {
        windows::stack_contains(address)
    }
    #[cfg(not(windows))]
    {
        let _ = address;
        false
    }
}
