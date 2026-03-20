.section .text

.extern signal_eoi
.extern reset_lp_timer
.extern set_next_thread

.macro m_push_caller_saved
    push rax
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9
    push r10
    push r11
.endm

.macro m_pop_caller_saved
    pop r11
    pop r10
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rax
.endm


.global isr_lapic_timer
isr_lapic_timer:
    m_push_caller_saved
    call yield_lp
    m_pop_caller_saved
    iretq