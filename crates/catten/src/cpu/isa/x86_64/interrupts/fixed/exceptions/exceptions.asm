.code64

.section .text
// Ordinary faults use ih_fault; hardware aborts and NMI stay separate.
.extern ih_double_fault
.extern ih_non_maskable_interrupt
.extern ih_machine_check

.macro EX_SAVE_REGISTERS
	// save the caller saved registers
	push r12
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

.macro EX_ALIGN_CALL_STACK
	mov r12, rsp
	and rsp, -16
	cld
.endm

.macro EX_PROLOGUE_NO_ERROR_CODE
	test byte ptr [rsp + 8], 3 // saved CS RPL
	jz 1f
	swapgs
1:
	EX_SAVE_REGISTERS
	EX_ALIGN_CALL_STACK
.endm

.macro EX_RESTORE_REGISTERS
	mov rsp, r12
	// restore the caller saved registers
	pop r11
	pop r10
	pop r9
	pop r8
	pop rcx
	pop rdx
	pop rsi
	pop rdi
	pop rax
	pop r12
.endm

.macro EX_EPILOGUE_NO_ERROR_CODE
	EX_RESTORE_REGISTERS
	test byte ptr [rsp + 8], 3
	jz 1f
	swapgs
1:
.endm

.macro EX_PROLOGUE_WITH_ERROR_CODE
	test byte ptr [rsp + 8 * 2], 3 // saved CS follows error code and RIP
	jz 1f
	swapgs
1:
	EX_SAVE_REGISTERS
	mov rdi, [rsp + 8 * 10] // load the error code
.endm

.macro EX_PROLOGUE_WITH_ERROR_CODE_AND_FAULT_ADDR
	EX_PROLOGUE_WITH_ERROR_CODE
	mov rsi, [rsp + 8 * 11] // saved RIP
	EX_ALIGN_CALL_STACK
.endm

.macro EX_EPILOGUE_WITH_ERROR_CODE
	EX_RESTORE_REGISTERS
	add rsp, 8 // Clean up the error code from the stack
	test byte ptr [rsp + 8], 3
	jz 1f
	swapgs
1:
.endm

// The hardware CS, not a fault-code guess or reusable TID, identifies origin.
// Ten saved words precede the hardware frame. All ordinary fault vectors pass
// the same trusted frame to Rust; hardware aborts and watchdog NMIs stay separate.
.extern ih_fault
.macro FAULT_NO_ERROR name, vector
.global \name
\name:
    EX_PROLOGUE_NO_ERROR_CODE
    mov edi, \vector
    xor esi, esi
    mov rdx, [r12 + 8 * 10] // RIP
    mov rcx, [r12 + 8 * 11] // CS
    xor r8d, r8d
    call ih_fault
    cli // keep restoring GS atomic with respect to maskable interrupts
    EX_EPILOGUE_NO_ERROR_CODE
    iretq
.endm

.macro FAULT_ERROR name, vector
.global \name
\name:
    EX_PROLOGUE_WITH_ERROR_CODE
    EX_ALIGN_CALL_STACK
    mov edi, \vector
    mov rsi, [r12 + 8 * 10] // error code
    mov rdx, [r12 + 8 * 11] // RIP
    mov rcx, [r12 + 8 * 12] // CS
    .if \vector == 14
        mov r8, cr2
    .else
        xor r8d, r8d
    .endif
    call ih_fault
    cli
    EX_EPILOGUE_WITH_ERROR_CODE
    iretq
.endm

FAULT_NO_ERROR isr_divide_by_zero, 0
FAULT_NO_ERROR isr_debug, 1
FAULT_NO_ERROR isr_breakpoint, 3
FAULT_NO_ERROR isr_overflow, 4
FAULT_NO_ERROR isr_bound_range_exceeded, 5
FAULT_NO_ERROR isr_invalid_opcode, 6
FAULT_NO_ERROR isr_device_not_available, 7
FAULT_ERROR isr_invalid_tss, 10
FAULT_ERROR isr_segment_not_present, 11
FAULT_ERROR isr_stack_segment_fault, 12
FAULT_ERROR isr_general_protection_fault, 13
FAULT_ERROR isr_page_fault, 14
FAULT_NO_ERROR isr_x87_floating_point, 16
FAULT_ERROR isr_alignment_check, 17
FAULT_NO_ERROR isr_simd_floating_point, 19
FAULT_NO_ERROR isr_virtualization, 20
FAULT_ERROR isr_control_protection, 21
FAULT_NO_ERROR isr_hypervisor_injection, 28
FAULT_ERROR isr_vmm_communication, 29
FAULT_ERROR isr_security_exception, 30

.global isr_double_fault
isr_double_fault:
    pop rdi
    and rsp, -16
    cld
    call ih_double_fault
    hlt

.global isr_machine_check
isr_machine_check:
    and rsp, -16
    cld
    call ih_machine_check
    hlt

.global isr_non_maskable_interrupt
isr_non_maskable_interrupt:
    EX_PROLOGUE_NO_ERROR_CODE
    mov rdi, [r12 + 8 * 10] // RIP
    mov rsi, [r12 + 8 * 11] // CS
    mov rdx, [r12 + 8 * 12] // RFLAGS
    mov rcx, [r12 + 8 * 13] // interrupted RSP (IST)
    mov r8, rbp
    call ih_non_maskable_interrupt
    cli
    EX_EPILOGUE_NO_ERROR_CODE
    iretq
