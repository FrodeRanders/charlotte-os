.pushsection .rodata
.balign 16
.global user_register_probe_start
.global user_register_probe_end
user_register_probe_start:
    push rax
    lea rax, [rsp + 8]
    and eax, 15
    mov [0x12380], rax
    pop rax
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
    sub rsp, 512
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
    pcmpeqd xmm1, xmm1
    pcmpeqd xmm2, xmm2
    pcmpeqd xmm3, xmm3
    pcmpeqd xmm4, xmm4
    pcmpeqd xmm5, xmm5
    pcmpeqd xmm6, xmm6
    pcmpeqd xmm7, xmm7
    pcmpeqd xmm8, xmm8
    pcmpeqd xmm9, xmm9
    pcmpeqd xmm10, xmm10
    pcmpeqd xmm11, xmm11
    pcmpeqd xmm12, xmm12
    pcmpeqd xmm13, xmm13
    pcmpeqd xmm14, xmm14
    pcmpeqd xmm15, xmm15
    mov dword ptr [0x12370], 0x3f80
    ldmxcsr [0x12370]
    mov word ptr [0x12378], 0x077f
    fldcw [0x12378]
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
    // Keep executing through timer preemption as well as the blocking call.
    // Syscall 79 returns monotonic counter/frequency in rax/rdi.
    mov eax, 79
    syscall
    mov rbx, rax
    mov rax, rdi
    xor edx, edx
    mov ecx, 10
    div rcx
    add rbx, rax
2:
    mov eax, 79
    syscall
    cmp rax, rbx
    jb 2b
    fxsave64 [0x12000]
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
.popsection

.pushsection .rodata
.balign 16
.global user_stack_growth_start
.global user_stack_growth_end
user_stack_growth_start:
    mov rax, rsp
    add rax, 4095
    and rax, -4096
    sub rax, 12288
    mov qword ptr [rax], 0x1234
    mov ecx, 0x12000
    mov qword ptr [rcx + 800], 0xface
1:
    cmp dword ptr [rcx + 808], 0
    jne 1b
    xor eax, eax
    mov rax, [rax]
user_stack_growth_end:
.popsection
