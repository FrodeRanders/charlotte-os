pub mod gic;

use core::arch::{asm, global_asm};

pub use gic::*;

use crate::cpu::isa::constants::interrupt_vectors::{ASYNC_IPI_VECTOR, LAPIC_TIMER_VECTOR};
use crate::cpu::isa::lp::ops::{cond_yield_lp, get_lp_id};
use crate::early_logln;
use crate::syscall::{self, ec_from_esr, EC_SVC_AARCH64, MAX_SYSCALL, TrapFrame};

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

/// Synchronous exception dispatcher (data/instruction aborts, SVC, etc.). On
/// the SVC path (exception class 0x15) it extracts the syscall number from
/// ESR_EL1.ISS, reads the volatile register context saved by the IVT entry's
/// `push_volatile_regs` into a [`TrapFrame`], advances ELR_EL1 past the SVC
/// instruction, and hands off to [`syscall::syscall_dispatch`]. For every other
/// exception class the existing fatal-panic behavior is preserved.
#[unsafe(no_mangle)]
pub extern "C" fn sync_dispatcher(frame_base: *mut u64) {
    let esr_el1: u64;
    let elr_el1: u64;
    let far_el1: u64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr_el1, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, elr_el1", out(reg) elr_el1, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, far_el1", out(reg) far_el1, options(nomem, nostack, preserves_flags));
    }

    let ec = ec_from_esr(esr_el1);

    if ec == EC_SVC_AARCH64 {
        let svc_imm = (esr_el1 & 0xFFFF) as u16;
        if svc_imm > MAX_SYSCALL {
            panic!("Unknown syscall number: {svc_imm}");
        }

        let spsr: u64;
        let sp_el0: u64;
        unsafe {
            asm!("mrs {}, spsr_el1", out(reg) spsr, options(nomem, nostack, preserves_flags));
        }
        unsafe {
            asm!("mrs {}, sp_el0", out(reg) sp_el0, options(nomem, nostack, preserves_flags));
        }

        let mut frame = TrapFrame {
            regs: [0u64; 19],
            // For an SVC exception, ELR_EL1 already holds the address of the
            // instruction *after* the SVC (the preferred return address). Do
            // NOT advance it — unlike data/instruction aborts, which point at
            // the faulting instruction.
            elr_el1,
            spsr_el1: spsr,
            sp_el0,
            lp_id: get_lp_id(),
        };

        // Read the saved volatile registers from the kernel stack. `frame_base`
        // is the stack pointer captured by the vector entry immediately after
        // `push_volatile_regs`, so it points at the saved x0. The mapping below
        // is derived directly from the push ordering in ivt.asm:
        //
        //  offset 0: x0,x1 (<- frame_base)
        //  offset 16: x2,x3
        //  …
        //  offset 144: x18,pad
        //  offset 160: x30,pad
        let base = frame_base as *const u64;
        unsafe {
            frame.regs[0] = base.add(0).read_volatile(); // x0
            frame.regs[1] = base.add(1).read_volatile(); // x1
            frame.regs[2] = base.add(2).read_volatile(); // x2
            frame.regs[3] = base.add(3).read_volatile(); // x3
            frame.regs[4] = base.add(4).read_volatile(); // x4
            frame.regs[5] = base.add(5).read_volatile(); // x5
            frame.regs[6] = base.add(6).read_volatile(); // x6
            frame.regs[7] = base.add(7).read_volatile(); // x7
            frame.regs[8] = base.add(8).read_volatile(); // x8
            frame.regs[9] = base.add(9).read_volatile(); // x9
            frame.regs[10] = base.add(10).read_volatile(); // x10
            frame.regs[11] = base.add(11).read_volatile(); // x11
            frame.regs[12] = base.add(12).read_volatile(); // x12
            frame.regs[13] = base.add(13).read_volatile(); // x13
            frame.regs[14] = base.add(14).read_volatile(); // x14
            frame.regs[15] = base.add(15).read_volatile(); // x15
            frame.regs[16] = base.add(16).read_volatile(); // x16
            frame.regs[17] = base.add(17).read_volatile(); // x17
            frame.regs[18] = base.add(18).read_volatile(); // x18
        }

        syscall::syscall_dispatch(&mut frame, svc_imm);

        // Write back x0 (return value) to the stack slot so `pop_volatile_regs`
        // restores it into the user's x0 before `eret`.
        unsafe {
            (base as *mut u64).write_volatile(frame.regs[0]);
            asm!("msr elr_el1, {}", in(reg) frame.elr_el1, options(nomem, nostack, preserves_flags));
        }

        return;
    }

    // Not SVC — log and potentially recover for known exception classes.
    // EC values from Arm ARM D17.2.37:
    //   0x24 = Data Abort (lower EL)
    //   0x25 = Data Abort (same EL)
    match ec {
        0x24 | 0x25 => {
            early_logln!("DATA ABORT: ESR={:x} ELR={:x} FAR={:x}", esr_el1, elr_el1, far_el1);
        }
        0x20 | 0x21 => {
            early_logln!("INST ABORT: ESR={:x} ELR={:x} FAR={:x}", esr_el1, elr_el1, far_el1);
        }
        _ => {
            early_logln!("UNHANDLED SYNC: EC={:x} ESR={:x} ELR={:x} FAR={:x}", ec, esr_el1, elr_el1, far_el1);
        }
    }
    early_logln!("  EC: {} (see Arm ARM D17.2.37 for exception classes)", ec);
    panic!(
        "Unhandled synchronous exception: EC={ec:#x}, ESR_EL1={esr_el1:#x}, ELR_EL1={elr_el1:#x}, FAR_EL1={far_el1:#x}",
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

/// FIQ dispatcher. Log and return — in emulated environments (QEMU) FIQs can
/// fire spuriously.
#[unsafe(no_mangle)]
pub extern "C" fn fiq_dispatcher() {
    early_logln!("Unexpected FIQ received (logged, not fatal)");
}

/// SError dispatcher. An SError is an asynchronous abort and is normally
/// fatal, but in emulated environments (QEMU) can fire spuriously. Log and
/// return.
#[unsafe(no_mangle)]
pub extern "C" fn serr_dispatcher() {
    let esr_el1: u64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr_el1, options(nomem, nostack, preserves_flags));
    }
    early_logln!("SError received: ESR={:#018x}", esr_el1);
}
