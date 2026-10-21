//! Darwin x86-64 stack probing. Code compiled with large frames calls
//! `___chkstk_darwin` with the requested size in RAX; every page between the
//! return address and the new frame must be touched so the host can grow the
//! stack. General registers are preserved; condition flags are not, matching
//! the Darwin helper.
#[cfg(windows)]
core::arch::global_asm!(
    ".globl db_chkstk_darwin",
    ".p2align 4",
    "db_chkstk_darwin:",
    "push rcx",
    "push rax",
    "cmp rax, 0x1000",
    "lea rcx, [rsp + 0x10]",
    "jb 2f",
    "1:",
    "sub rcx, 0x1000",
    "or qword ptr [rcx], 0",
    "sub rax, 0x1000",
    "cmp rax, 0x1000",
    "ja 1b",
    "2:",
    "sub rcx, rax",
    "or qword ptr [rcx], 0",
    "pop rax",
    "pop rcx",
    "ret",
);

#[cfg(windows)]
unsafe extern "C" {
    fn db_chkstk_darwin();
}

pub fn symbol(name: &str) -> Option<u64> {
    #[cfg(windows)]
    if name == "___chkstk_darwin" {
        return Some(db_chkstk_darwin as *const () as u64);
    }
    let _ = name;
    None
}
