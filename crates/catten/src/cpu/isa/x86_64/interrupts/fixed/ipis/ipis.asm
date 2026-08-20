.code64

.macro IPI_PROLOGUE
    test byte ptr [rsp + 8], 3
    jz 1f
    swapgs
1:
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
    mov r12, rsp
    and rsp, -16
    cld
/* IPIs are always level triggered so as not to be missed. Thus when the ISR runs we must signal
   end of interrupt to the local interrupt controller to ensure the ISR isn't called repeatedly
   ad infinitum
*/
    call signal_eoi
.endm

.macro IPI_EPILOGUE
    mov rsp, r12
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
    test byte ptr [rsp + 8], 3
    jz 1f
    swapgs
1:
.endm

.section .text
.global isr_asynchronous_ipi
isr_asynchronous_ipi:
    IPI_PROLOGUE
    call ih_asynchronous_ipi
    IPI_EPILOGUE
    iretq

.global isr_synchronous_ipi
isr_synchronous_ipi:
    IPI_PROLOGUE
    call ih_synchronous_ipi
    IPI_EPILOGUE
    iretq

.global isr_scheduler_ipi
isr_scheduler_ipi:
    IPI_PROLOGUE
    call cond_yield_lp
    IPI_EPILOGUE
    iretq
