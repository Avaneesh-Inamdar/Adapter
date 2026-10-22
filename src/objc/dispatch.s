.text
.p2align 4
.globl adapter_objc_msg_send
adapter_objc_msg_send:
    test rdi, rdi
    jz .Lobjc_nil
    push r10
    xor r10d, r10d
    jmp .Lobjc_lookup
.p2align 4
.globl adapter_objc_msg_send_super2
adapter_objc_msg_send_super2:
    push r10
    mov r10, [rdi + 8]
    mov rdi, [rdi]
    test rdi, rdi
    jnz .Lobjc_lookup
    pop r10
    jmp .Lobjc_nil
.Lobjc_lookup:
    // The push made RSP 16-byte aligned. Preserve arguments and SysV AL
    // (the variadic vector count); stack arguments stay in their original slots.
    sub rsp, 192
    mov [rsp], rdi
    mov [rsp + 8], rsi
    mov [rsp + 16], rdx
    mov [rsp + 24], rcx
    mov [rsp + 32], r8
    mov [rsp + 40], r9
    mov [rsp + 48], rax
    movdqu [rsp + 64], xmm0
    movdqu [rsp + 80], xmm1
    movdqu [rsp + 96], xmm2
    movdqu [rsp + 112], xmm3
    movdqu [rsp + 128], xmm4
    movdqu [rsp + 144], xmm5
    movdqu [rsp + 160], xmm6
    movdqu [rsp + 176], xmm7
    mov rdx, r10
    call adapter_objc_lookup
    mov r11, rax
    mov rdi, [rsp]
    mov rsi, [rsp + 8]
    mov rdx, [rsp + 16]
    mov rcx, [rsp + 24]
    mov r8, [rsp + 32]
    mov r9, [rsp + 40]
    mov rax, [rsp + 48]
    movdqu xmm0, [rsp + 64]
    movdqu xmm1, [rsp + 80]
    movdqu xmm2, [rsp + 96]
    movdqu xmm3, [rsp + 112]
    movdqu xmm4, [rsp + 128]
    movdqu xmm5, [rsp + 144]
    movdqu xmm6, [rsp + 160]
    movdqu xmm7, [rsp + 176]
    add rsp, 192
    pop r10
    jmp r11
.Lobjc_nil:
    xor eax, eax
    xor edx, edx
    pxor xmm0, xmm0
    pxor xmm1, xmm1
    ret

.p2align 4
.globl adapter_objc_autorelease_return
adapter_objc_autorelease_return:
    mov rsi, [rsp]
    jmp adapter_objc_offer_return
.p2align 4
.globl adapter_objc_retain_autorelease_return
adapter_objc_retain_autorelease_return:
    mov rsi, [rsp]
    jmp adapter_objc_retain_offer_return
.p2align 4
.globl adapter_objc_claim_return
adapter_objc_claim_return:
    mov rsi, [rsp]
    xor edx, edx
    jmp adapter_objc_accept_return
.p2align 4
.globl adapter_objc_unsafe_claim_return
adapter_objc_unsafe_claim_return:
    mov rsi, [rsp]
    mov edx, 1
    jmp adapter_objc_accept_return
