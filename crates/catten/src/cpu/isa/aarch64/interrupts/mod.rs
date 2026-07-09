pub mod gic;

use core::arch::{asm, global_asm};

pub use gic::*;

use crate::cpu::isa::constants::interrupt_vectors::{ASYNC_IPI_VECTOR, LAPIC_TIMER_VECTOR};
use crate::cpu::isa::lp::ops::cond_yield_lp;

// Include the interrupt vector table assembly
global_asm!(include_str!("ivt.asm"));

#[inline(always)]
pub fn load_ivt() {
    // Load the interrupt vector table
    unsafe {
        // Load the interrupt vector table
        asm!(
            "ldr x0, =ivt", 
            "msr vbar_el1, x0",
            out("x0") _,
        );
    }
}

/// Synchronous exception dispatcher (e.g. data/instruction aborts, SVC). No
/// device interrupts arrive here; for now unexpected synchronous exceptions are
/// fatal while the exception decoding layer is developed.
#[unsafe(no_mangle)]
pub extern "C" fn sync_dispatcher() {
    let esr_el1: u64;
    let elr_el1: u64;
    let far_el1: u64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr_el1, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, elr_el1", out(reg) elr_el1, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, far_el1", out(reg) far_el1, options(nomem, nostack, preserves_flags));
    }
    panic!(
        "Unhandled synchronous exception: ESR_EL1={:#x}, ELR_EL1={:#x}, FAR_EL1={:#x}",
        esr_el1, elr_el1, far_el1
    );
}

/// IRQ dispatcher. Acknowledges the pending Group 1 interrupt, dispatches it,
/// signals end-of-interrupt, and then performs any pending context switch. This
/// is the async heart of the scheduler: the Generic Timer PPI advances the timer
/// queue (which may wake threads via their observers) and marks a context switch
/// pending, which is honoured here via `cond_yield_lp`.
#[unsafe(no_mangle)]
pub extern "C" fn irq_dispatcher() {
    let intid = gic::acknowledge_int();
    // INTIDs 1020-1023 are special/spurious and require no handling or EOI.
    if intid >= 1020 {
        return;
    }
    match intid {
        LAPIC_TIMER_VECTOR => {
            // Advance the timer queue, firing any events whose deadline passed
            // (waking their observer threads) and rearming the timer.
            if let Ok(mut timer_queue) = crate::timers::TIMER_QUEUES.try_get_mut() {
                timer_queue.process_events();
            }
        }
        ASYNC_IPI_VECTOR => {
            // Asynchronous IPI: drain this LP's IPI RPC queue.
            crate::cpu::multiprocessor::ipi::drain_local_ipi_queue();
        }
        _ => {
            // Other INTIDs (SPIs from devices) will be routed once the external
            // interrupt controller path is wired up.
        }
    }
    gic::end_of_int(intid);
    // Carry out a context switch if the timer or an IPI marked one pending.
    cond_yield_lp();
}

/// FIQ dispatcher. We route all interrupts as Group 1 IRQs, so an FIQ is
/// unexpected in the current configuration.
#[unsafe(no_mangle)]
pub extern "C" fn fiq_dispatcher() {
    panic!("Unexpected FIQ received");
}

/// SError dispatcher. An SError is an asynchronous abort and is treated as
/// fatal.
#[unsafe(no_mangle)]
pub extern "C" fn serr_dispatcher() {
    let esr_el1: u64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr_el1, options(nomem, nostack, preserves_flags));
    }
    panic!("Unhandled SError: ESR_EL1={:#x}", esr_el1);
}
