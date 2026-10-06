use crate::{
    cpu::isa::{
        init::gdt,
        interrupts::idt::Idt,
    },
    memory::VAddr,
};

pub fn set_gates(idt: &mut Idt) {
    idt.set_gate(0, isr_divide_by_zero, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(1, isr_debug, gdt::KERNEL_CODE_SELECTOR, None, true, true);
    idt.set_gate(
        2,
        isr_non_maskable_interrupt,
        gdt::KERNEL_CODE_SELECTOR,
        Some(gdt::Tss::NMI_IST_INDEX),
        false,
        true,
    );
    idt.set_gate(3, isr_breakpoint, gdt::KERNEL_CODE_SELECTOR, None, true, true);
    idt.set_gate(4, isr_overflow, gdt::KERNEL_CODE_SELECTOR, None, true, true);
    idt.set_gate(5, isr_bound_range_exceeded, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(6, isr_invalid_opcode, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(7, isr_device_not_available, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(
        8,
        isr_double_fault,
        gdt::KERNEL_CODE_SELECTOR,
        Some(gdt::Tss::DOUBLE_FAULT_IST_INDEX),
        false,
        true,
    );
    idt.set_gate(10, isr_invalid_tss, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(11, isr_segment_not_present, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(12, isr_stack_segment_fault, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(13, isr_general_protection_fault, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(14, isr_page_fault, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(16, isr_x87_floating_point, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(17, isr_alignment_check, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(18, isr_machine_check, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(19, isr_simd_floating_point, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(20, isr_virtualization, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(21, isr_control_protection, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(28, isr_hypervisor_injection, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(29, isr_vmm_communication, gdt::KERNEL_CODE_SELECTOR, None, false, true);
    idt.set_gate(30, isr_security_exception, gdt::KERNEL_CODE_SELECTOR, None, false, true);
}

core::arch::global_asm! {
    include_str!("exceptions.asm"),
}
unsafe extern "custom" {
    fn isr_divide_by_zero();
    fn isr_debug();
    fn isr_non_maskable_interrupt();
    fn isr_breakpoint();
    fn isr_overflow();
    fn isr_bound_range_exceeded();
    fn isr_invalid_opcode();
    fn isr_device_not_available();
    fn isr_double_fault();
    fn isr_invalid_tss();
    fn isr_stack_segment_fault();
    fn isr_general_protection_fault();
    fn isr_segment_not_present();
    fn isr_page_fault();
    fn isr_x87_floating_point();
    fn isr_alignment_check();
    fn isr_machine_check();
    fn isr_simd_floating_point();
    fn isr_virtualization();
    fn isr_control_protection();
    fn isr_hypervisor_injection();
    fn isr_vmm_communication();
    fn isr_security_exception();
}

/// Only a saved ring-3 code selector permits application containment. Kernel
/// faults, NMIs and hardware aborts must never be disguised as tenant failures.
const fn user_origin(cs: u64) -> bool {
    cs & 3 == 3
}

#[unsafe(no_mangle)]
extern "C" fn ih_fault(vector: u64, error_code: u64, rip: VAddr, cs: u64, address: VAddr) {
    if user_origin(cs) {
        let asid = crate::cpu::isa::x86_64::memory::paging::CURRENT_LOGICAL_ASID
            [crate::cpu::isa::lp::ops::get_lp_id() as usize]
            .load(core::sync::atomic::Ordering::Acquire);
        assert_ne!(asid, crate::memory::KERNEL_ASID, "ring-3 fault without a live user context");
        // The complete hardware/GPR frame is on this thread's trusted stack
        // and GS is already the kernel base. Let remote retirement/shootdown
        // IPIs progress before taking any scheduler or address-space guard.
        crate::cpu::isa::lp::ops::unmask_interrupts!();
        let not_present_data = error_code & (1 | 4 | 16) == 4;
        if vector == 14 && not_present_data {
            let fault_addr = usize::from(address);
            if crate::cpu::scheduler::threads::grow_current_user_stack(asid, fault_addr).is_some()
                || crate::memory::commit_user_heap_page(asid, fault_addr)
            {
                return;
            }
        }
        crate::early_logln!(
            "FATAL USER FAULT: ASID={} vector={} error={:#x} RIP={:?} address={:?}",
            asid,
            vector,
            error_code,
            rip,
            address
        );
        crate::cpu::scheduler::abort_address_space(asid);
    }
    panic!(
        "Kernel exception vector={vector} error={error_code:#x} RIP={rip:?} CS={cs:#x} \
         address={address:?}"
    );
}

#[unsafe(no_mangle)]
extern "C" fn ih_double_fault(_error_code: u64) {
    panic!("Double fault");
}

#[unsafe(no_mangle)]
extern "C" fn ih_machine_check() {
    panic!("Machine check");
}

#[unsafe(no_mangle)]
extern "C" fn ih_non_maskable_interrupt(rip: u64, cs: u64, rflags: u64, rsp: u64, rbp: u64) {
    if crate::debug_trace::record_requested_nmi(rip, cs, rflags, rsp, rbp) {
        return;
    }
    panic!("Non-maskable interrupt");
}

pub(crate) fn test_fault_origin() {
    assert!(user_origin(gdt::USER_CODE_SELECTOR as u64));
    assert!(!user_origin(gdt::KERNEL_CODE_SELECTOR as u64));
    assert!(!user_origin(0));
    assert!(!user_origin(1));
    assert!(!user_origin(2));
}
