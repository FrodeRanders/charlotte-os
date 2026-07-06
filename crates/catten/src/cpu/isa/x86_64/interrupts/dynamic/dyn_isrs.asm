.code64

.extern get_dyn_ih
.extern cond_yield_lp
.extern DYN_VECS_PER_LP
.extern DYN_VEC_START_OFFSET

.macro m_dyn_isr vector:req
.global dyn_isr_\vector
dyn_isr_\vector:
    pushfq
    push rax
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    lea rdi, [DYN_IH_MATRIX]
    mov rsi, \vector
    call get_dyn_ih
; if the function pointer returned by get_dyn_ih is null, skip the call
    test rax, rax
    jz skip_ih_call_\vector
; make the call to the interrupt handler if the function pointer is non-null
    call qword ptr [rax]
skip_ih_call_\vector:
    ; Execute context switch if pending
    call cond_yield_lp
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rax
    popfq
    iretq
.endm

.section .text
.altmacro
.set vector_num, 0
.rept 220
    m_dyn_isr %vector_num
    .set vector_num, vector_num+1
.endr
.noaltmacro