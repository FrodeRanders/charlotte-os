.section .rodata
.balign 16
.global user_register_probe_start
.global user_register_probe_end
user_register_probe_start:
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    sub rsp, 520
    fxsave64 [rsp]
    mov edi, 0x12000
    mov rsi, rsp
    mov ecx, 80
    cld
    rep movsq
    rdfsbase rax
    mov [0x12358], rax
    rdgsbase rax
    mov [0x12360], rax
    mov qword ptr [0x12320], 0xdead
    mov eax, 8
    syscall
user_register_probe_end:

.global user_state_probe_start
.global user_state_probe_end
user_state_probe_start:
    pcmpeqd xmm0, xmm0
    fld1
    mov eax, 0x123000
    wrfsbase rax
    mov eax, 0x456000
    wrgsbase rax
    mov qword ptr [0x12318], 0xbeef
    mov eax, 1
    mov edi, 3
    xor esi, esi
    mov edx, 50
    syscall
    mov rdi, rax
    mov eax, 4
    syscall
    movdqu [0x12340], xmm0
    fstp qword ptr [0x12350]
    rdfsbase rax
    mov [0x12358], rax
    rdgsbase rax
    mov [0x12360], rax
    mov qword ptr [0x12320], 0xdead
    mov eax, 8
    syscall
user_state_probe_end:
