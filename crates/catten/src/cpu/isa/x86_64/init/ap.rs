use alloc::{
    boxed::Box,
    vec::Vec,
};

use spin::lazylock::LazyLock;

use super::{
    INTERRUPT_STACK_SIZE,
    gdt,
};
use crate::{
    cpu::{
        isa::{
            interrupts::{
                idt::{
                    Idt,
                    asm_load_idt,
                },
                x2apic::X2Apic,
            },
            lp::ops::init_lp_state,
        },
        multiprocessor::get_lp_count,
    },
    logln,
};

static AP_INTERRUPT_STACKS: LazyLock<Vec<[u8; INTERRUPT_STACK_SIZE]>> = LazyLock::new(|| {
    let num_aps = get_lp_count() - 1; // Exclude BSP
    let mut ret = Vec::<[u8; INTERRUPT_STACK_SIZE]>::with_capacity(num_aps as usize);
    for _ in 0..num_aps {
        ret.push(*(Box::new([0u8; INTERRUPT_STACK_SIZE])));
    }
    ret
});

static AP_DF_STACKS: LazyLock<Vec<[u8; INTERRUPT_STACK_SIZE]>> = LazyLock::new(|| {
    let num_aps = get_lp_count() - 1; // Exclude BSP
    let mut ret = Vec::<[u8; INTERRUPT_STACK_SIZE]>::with_capacity(num_aps as usize);
    for _ in 0..num_aps {
        ret.push(*(Box::new([0u8; INTERRUPT_STACK_SIZE])));
    }
    ret
});

static AP_NMI_STACKS: LazyLock<Vec<[u8; INTERRUPT_STACK_SIZE]>> = LazyLock::new(|| {
    let num_aps = get_lp_count() - 1;
    let mut stacks = Vec::with_capacity(num_aps as usize);
    for _ in 0..num_aps {
        stacks.push(*(Box::new([0u8; INTERRUPT_STACK_SIZE])));
    }
    stacks
});

pub static AP_TSS: LazyLock<Vec<super::gdt::Tss>> = LazyLock::new(|| {
    let mut tsses = Vec::new();
    for i in 0..(get_lp_count() - 1) {
        tsses.push(super::gdt::Tss::new(
            unsafe { (&raw const AP_INTERRUPT_STACKS[i as usize]).byte_add(INTERRUPT_STACK_SIZE) }
                as u64,
            unsafe { (&raw const AP_DF_STACKS[i as usize]).byte_add(INTERRUPT_STACK_SIZE) } as u64,
            unsafe { (&raw const AP_NMI_STACKS[i as usize]).byte_add(INTERRUPT_STACK_SIZE) } as u64,
        ));
    }
    tsses
});

static AP_GDTS: LazyLock<Vec<super::gdt::Gdt>> = LazyLock::new(|| {
    let mut gdts = Vec::new();
    for tss in AP_TSS.iter() {
        gdts.push(super::gdt::Gdt::new(tss));
    }
    gdts
});

pub static AP_IDTS: LazyLock<Vec<crate::cpu::isa::interrupts::idt::Idt>> = LazyLock::new(|| {
    let mut idts = Vec::new();
    for _ in 0..(get_lp_count() - 1) {
        let mut idt = crate::cpu::isa::interrupts::idt::Idt::new();
        crate::cpu::isa::interrupts::fixed::register_fixed_isr_gates(&mut idt);
        crate::cpu::isa::interrupts::dynamic::register_dynamic_isr_gates(&mut idt);
        idts.push(idt);
    }
    idts
});

pub static AP_IDTRS: LazyLock<Vec<crate::cpu::isa::interrupts::idt::Idtr>> = LazyLock::new(|| {
    let mut idtrs = Vec::new();
    for idt in AP_IDTS.iter() {
        idtrs.push(crate::cpu::isa::interrupts::idt::Idtr::new(
            (size_of::<Idt>() - 1) as u16,
            idt as *const Idt as u64,
        ));
    }
    idtrs
});

pub fn init_ap() {
    let lp_id = crate::cpu::isa::lp::ops::get_lp_id();
    let ap_index = (lp_id - 1) as usize; // APs start from LP1
    AP_GDTS[ap_index].load();
    unsafe {
        gdt::reload_segment_regs();
    }
    unsafe { asm_load_idt(&raw const AP_IDTRS[ap_index]) };
    init_lp_state();
    X2Apic::record_id();
    logln!("LP {}: x86-64 AP initialization complete.", lp_id);
}
