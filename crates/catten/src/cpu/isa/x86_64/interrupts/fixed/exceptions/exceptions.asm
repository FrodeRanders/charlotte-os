.code64

.text
//Handlers
.extern ih_divide_by_zero
.extern ih_double_fault
.extern ih_general_protection_fault
.extern ih_page_fault
.extern ih_segment_not_present
.extern ih_debug
.extern ih_non_maskable_interrupt
.extern ih_breakpoint
.extern ih_overflow
.extern ih_bound_range_exceeded
.extern ih_invalid_opcode
.extern ih_device_not_available
.extern ih_invalid_tss
.extern ih_stack_segment_fault
.extern ih_x87_floating_point
.extern ih_alignment_check
.extern ih_machine_check
.extern ih_simd_floating_point
.extern ih_virtualization
.extern ih_control_protection
.extern ih_hypervisor_injection
.extern ih_vmm_communication
.extern ih_security_exception

.macro EX_PROLOGUE_NO_ERROR_CODE
	// save the caller saved registers
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

.macro EX_EPILOGUE_NO_ERROR_CODE
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
.endm

.macro EX_PROLOGUE_WITH_ERROR_CODE
	EX_PROLOGUE_NO_ERROR_CODE
	mov rdi, [rsp + 8 * 9] // load the error code
.endm

.macro EX_PROLOGUE_WITH_ERROR_CODE_AND_FAULT_ADDR
	EX_PROLOGUE_WITH_ERROR_CODE
	mov rsi, [rsp + 8 * 10]
.endm

.macro EX_EPILOGUE_WITH_ERROR_CODE
	EX_EPILOGUE_NO_ERROR_CODE
	add rsp, 8 // Clean up the error code from the stack
.endm

//The actual ISRs
.global isr_divide_by_zero
isr_divide_by_zero:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_divide_by_zero
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_double_fault
isr_double_fault:
	//Registers are not saved since this exception is an abort
	pop rdi //pop the error code (should always be 0)
	call ih_double_fault
	hlt //halt the core since double faults are an abort

.global isr_general_protection_fault
isr_general_protection_fault:
	EX_PROLOGUE_WITH_ERROR_CODE_AND_FAULT_ADDR
	mov rdx, [rsp]
	call ih_general_protection_fault
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_page_fault
isr_page_fault:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_page_fault
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_segment_not_present
isr_segment_not_present:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_segment_not_present
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_debug
isr_debug:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_debug
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_non_maskable_interrupt
isr_non_maskable_interrupt:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_non_maskable_interrupt
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_breakpoint
isr_breakpoint:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_breakpoint
	EX_EPILOGUE_NO_ERROR_CODE
	iretq


.global isr_overflow
isr_overflow:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_overflow
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_bound_range_exceeded
isr_bound_range_exceeded:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_bound_range_exceeded
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_invalid_opcode
isr_invalid_opcode:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_invalid_opcode
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_device_not_available
isr_device_not_available:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_device_not_available
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_invalid_tss
isr_invalid_tss:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_invalid_tss
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_stack_segment_fault
isr_stack_segment_fault:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_stack_segment_fault
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_x87_floating_point
isr_x87_floating_point:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_x87_floating_point
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_alignment_check
isr_alignment_check:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_alignment_check
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_machine_check
isr_machine_check:
	// Registers are not saved since this exception is an abort
	// Unlike Double Fault, Machine Check does not push an error code
	call ih_machine_check
	hlt // Halt the core since machine checks indicate severe hardware issues

.global isr_simd_floating_point
isr_simd_floating_point:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_simd_floating_point
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_virtualization
isr_virtualization:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_virtualization
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_control_protection
isr_control_protection:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_control_protection
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_hypervisor_injection
isr_hypervisor_injection:
	EX_PROLOGUE_NO_ERROR_CODE
	call ih_hypervisor_injection
	EX_EPILOGUE_NO_ERROR_CODE
	iretq

.global isr_vmm_communication
isr_vmm_communication:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_vmm_communication
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq

.global isr_security_exception
isr_security_exception:
	EX_PROLOGUE_WITH_ERROR_CODE
	call ih_security_exception
	EX_EPILOGUE_WITH_ERROR_CODE
	iretq
