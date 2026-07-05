.code64

.extern get_dyn_ih
.extern cond_yield_lp
.extern DYN_VECS_PER_LP
.extern DYN_VEC_START_OFFSET

.macro DYN_ISR vector:req
    .global dyn_isr_\vector
dyn_isr_\vector:
    push rax
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    mov rdi, \vector
    call get_dyn_ih
    call qword ptr [rax]
    // Execute context switch if pending
    call cond_yield_lp
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rax
    iretq
.endm

.section .text
.rept 220
    DYN_ISR \@i
.endr