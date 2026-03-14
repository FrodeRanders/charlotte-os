.section .text

/* Load the address space (CR3) and stack pointer (RSP) from the current thread context */
.macro LOAD_AS_SP_FROM_CTX
    mov rbx, qword ptr [TC_CR3_OFFSET]
    add rbx, rax
    mov rbx, qword ptr [rbx]
    mov cr3, rbx // Load the next thread's address space register value from its context
    mov rbx, qword ptr [TC_RSP_CPL0_OFFSET]
    add rbx, rax
    mov rsp, qword ptr [rbx] # Load the next thread's stack pointer from its context
    wrgsbase rax
.endm

.macro STORE_AS_SP_TO_CTX
    rdgsbase rax
    mov rbx, qword ptr [TC_RSP_CPL0_OFFSET]
    add rbx, rax
    mov qword ptr [rbx], rsp // save the stack pointer to the thread context
    mov rbx, qword ptr [TC_CR3_OFFSET]
    add rbx, rax
    mov rcx, cr3
    mov qword ptr [rbx], rcx // save the stack pointer to the thread context
.endm

.global isr_context_switch
isr_context_switch:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    STORE_AS_SP_TO_CTX
    call set_next_thread  # Call the local scheduler to get the next thread and set FSBASE to its context base
    LOAD_AS_SP_FROM_CTX
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    iretq

.global isr_yield
isr_yield:
    call set_next_thread  # Call the local scheduler to get the next thread and return the context base in rax
    LOAD_AS_SP_FROM_CTX
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    iretq