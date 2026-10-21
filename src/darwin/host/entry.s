.intel_syntax noprefix
.text
# Save all incoming SysV argument registers before making a Windows ABI call.
# Entry RSP is 8 mod 16; 280 bytes gives aligned RSP and 32-byte shadow space.
.macro SAVE_ARGS gp
    sub rsp, 280
    mov [rsp+64], rdi
    mov [rsp+72], rsi
    mov [rsp+80], rdx
    mov [rsp+88], rcx
    mov [rsp+96], r8
    mov [rsp+104], r9
    movaps [rsp+112], xmm0
    movaps [rsp+128], xmm1
    movaps [rsp+144], xmm2
    movaps [rsp+160], xmm3
    movaps [rsp+176], xmm4
    movaps [rsp+192], xmm5
    movaps [rsp+208], xmm6
    movaps [rsp+224], xmm7
    mov dword ptr [rsp+32], \gp
    mov dword ptr [rsp+36], 48
    lea rax, [rsp+288]
    mov [rsp+40], rax
    lea rax, [rsp+64]
    mov [rsp+48], rax
.endm
.globl db_guest_printf
db_guest_printf:
    SAVE_ARGS 8
    mov rcx, [rsp+64]
    lea rdx, [rsp+32]
    call db_printf_entry
    add rsp, 280
    ret
.globl db_guest_fprintf
db_guest_fprintf:
    SAVE_ARGS 16
    mov rcx, [rsp+64]
    mov rdx, [rsp+72]
    lea r8, [rsp+32]
    call db_fprintf_entry
    add rsp, 280
    ret
.globl db_guest_snprintf
db_guest_snprintf:
    SAVE_ARGS 24
    mov rcx, [rsp+64]
    mov rdx, [rsp+72]
    mov r8, [rsp+80]
    lea r9, [rsp+32]
    call db_snprintf_entry
    add rsp, 280
    ret
.globl db_chkstk
db_chkstk:
    push rcx
    push r11
    lea rcx, [rsp+24]
    mov r11, rax
1:
    cmp r11, 4096
    jb 2f
    sub rcx, 4096
    test byte ptr [rcx], 0
    sub r11, 4096
    jmp 1b
2:
    sub rcx, r11
    test byte ptr [rcx], 0
    pop r11
    pop rcx
    ret
.globl db_nexttoward
db_nexttoward:
    sub rsp, 56
    movsd [rsp+32], xmm0
    fld qword ptr [rsp+32]
    fld tbyte ptr [rsp+64]
    fucomip st(0), st(1)
    fstp st(0)
    seta r8b
    setb al
    movzx r8d, r8b
    movzx eax, al
    sub r8d, eax
    fld tbyte ptr [rsp+64]
    fstp qword ptr [rsp+40]
    movsd xmm1, [rsp+40]
    call db_nexttoward_entry
    add rsp, 56
    ret
