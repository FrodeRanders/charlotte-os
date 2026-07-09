#[macro_export]
macro_rules! await_interrupt {
    () => {
        loop {
            unsafe {
                core::arch::asm!(
                    "msr daifclr, 0b1111",
                    "wfi",
                    options(nomem, nostack, preserves_flags)
                );
            }
        }
    };
}
pub use await_interrupt;
#[macro_export]
macro_rules! mask_interrupts {
    () => {
        unsafe { core::arch::asm!("msr daifset, 0b1111", options(nomem, nostack)) }
    };
}
pub use mask_interrupts;
#[macro_export]
macro_rules! unmask_interrupts {
    () => {
        unsafe { core::arch::asm!("msr daifclr, 0b1111", options(nomem, nostack)) }
    };
}
pub use unmask_interrupts;

/// Returns `true` if IRQs are currently unmasked (enabled) on the calling
/// logical processor. The DAIF `I` bit (bit 7) is set when IRQs are masked, so
/// interrupts are enabled when it is clear.
#[inline(always)]
pub fn get_int_state() -> bool {
    let daif: u64;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags)
        );
    }
    daif & (1 << 7) == 0
}

use crate::cpu::isa::lp::LpId;
use crate::memory::VAddr;

pub fn store_lp_id(lp_id: LpId) {
    unsafe {
        core::arch::asm!(
            "msr tpidr_el1, {lp_id:w}",
            lp_id = in(reg) lp_id,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn get_lp_id() -> LpId {
    let lp_id: u32;
    unsafe {
        core::arch::asm!(
            "mrs {lp_id:w}, tpidr_el1",
            lp_id = out(reg) lp_id,
            options(nomem, nostack, preserves_flags)
        );
    }
    lp_id
}

pub fn get_lic_id() -> u32 {
    let mpidr_el1: u64;
    unsafe {
        core::arch::asm!(
            "mrs {mpidr_el1}, mpidr_el1",
            mpidr_el1 = out(reg) mpidr_el1,
            options(nomem, nostack, preserves_flags)
        );
    }
    // The Affinity Level 0 field (bits [7:0]) contains the CPU ID within the cluster
    (mpidr_el1 & 0xff) as u32
}

pub fn set_lp_local_base(vaddr: VAddr) {
    unsafe {
        core::arch::asm!(
            "msr tpidr_el0, {vaddr:x}",
            vaddr = in(reg) <VAddr as Into<u64>>::into(vaddr),
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn get_lp_local_base() -> crate::memory::VAddr {
    let addr: u64;
    unsafe {
        core::arch::asm!(
            "mrs {addr}, tpidr_el0",
            addr = out(reg) addr,
            options(nomem, nostack, preserves_flags)
        );
    }
    crate::memory::VAddr::from(addr)
}
