//! # Low-level operations for x86_64 Logical Processors

pub fn init_lp_state() {
    unsafe {
        core::arch::asm! {
            "mov rax, cr4",
            "or rax, 1<<16",
            "mov cr4, rax",
            "mov rax, 0",
            "wrfsbase rax",
            "wrgsbase rax",
            out("rax") _
        }
    }
}

#[rustfmt::skip]
#[macro_export]
macro_rules! await_interrupt {
    () => {
        loop {
            unsafe {
                core::arch::asm!(
                    "sti",
                    "hlt", 
                    options(nomem, nostack, preserves_flags)
                );
            }
        }
    };
}
#[rustfmt::skip]
pub use await_interrupt;

#[rustfmt::skip]
#[macro_export]
macro_rules! mask_interrupts {
    () => {
        unsafe {
            core::arch::asm!("cli", options(nomem, nostack));
        }
    };
}
#[rustfmt::skip]
pub use mask_interrupts;

#[rustfmt::skip]
#[macro_export]
macro_rules! unmask_interrupts {
    () => {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    };
}
#[rustfmt::skip]
pub use unmask_interrupts;

pub fn get_lic_id() -> u32 {
    let apic_id: u32;
    use crate::cpu::isa::constants::*;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            inlateout("ecx") msrs::x2apic::ID_REG => _,
            lateout("eax") apic_id,
            lateout("edx") _,
            options(nostack, preserves_flags)
        );
    }
    apic_id
}

use core::arch::asm;

use super::LpId;
use crate::cpu::isa::constants::*;

pub fn store_lp_id(id: LpId) {
    let id_upper = ((id as u64) >> 32) as u32;
    let id_lower = ((id as u64) & (1 << 32) - 1) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("eax") id_lower,
            in("edx") id_upper,
            in("ecx") msrs::TSC_AUX,
            options(nostack, preserves_flags)
        );
    }
}

pub fn get_lp_id() -> LpId {
    let mut id: u32;
    unsafe {
        core::arch::asm!(
            "rdtscp",
            out("edx") _,
            out("eax") _,
            out("ecx") id,
        );
    }
    id as crate::cpu::isa::lp::LpId
}

use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interface::timers::LpTimerIfce;
use crate::cpu::isa::interrupts::x2apic::{LAPICS, X2Apic};
use crate::cpu::isa::lp::thread_context::{TC_CR3_OFFSET, TC_RSP_CPL0_OFFSET};
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::MASTER_THREAD_TABLE;
use crate::memory::VAddr;

#[inline]
pub extern "C" fn get_lp_local_base() -> VAddr {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "rdgsbase {}",
            out(reg) ret,
            options(nomem, nostack, preserves_flags)
        );
    }
    VAddr::from(ret)
}

#[inline]
pub extern "C" fn set_lp_local_base(base: VAddr) {
    unsafe {
        core::arch::asm!(
            "wrgsbase {}",
            in(reg) <VAddr as Into<u64>>::into(base),
            options(nomem, nostack, preserves_flags)
        )
    }
}

#[inline]
pub extern "C" fn get_thread_context_ptr() -> VAddr {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "rdfsbase {}",
            out(reg) ret,
            options(nomem, nostack, preserves_flags)
        );
    }
    VAddr::from(ret)
}

#[inline]
pub extern "C" fn set_thread_context_ptr(ctx_ptr: VAddr) {
    unsafe {
        core::arch::asm!(
            "wrfsbase {}",
            in(reg) <VAddr as Into<u64>>::into(ctx_ptr),
            options(nomem, nostack, preserves_flags)
        )
    };
}

#[repr(u8)]
pub enum YieldStatus {
    Success = 0,
    InvalidTidFromSched = 1,
}

#[unsafe(no_mangle)]
pub extern "C" fn yield_lp() -> YieldStatus {
    if let Ok(next_tid) = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().next() {
        let mut gs_base: usize;
        unsafe {
            core::arch::asm!(
                "lea {temp}, gs:[0]",
                temp = out(reg) gs_base,
            )
        }
        if VAddr::from(gs_base) != unsafe { VAddr::from_raw_unchecked(0) } {
            unsafe {
                core::arch::asm!(
                    "push rbx",
                    "push rbp",
                    "push r12",
                    "push r13",
                    "push r14",
                    "push r15",
                    "mov rbx, cr3",
                    "mov gs:[{cr3_offset}], rbx",
                    "mov gs:[{rsp_offset}], rsp",
                    cr3_offset = const TC_CR3_OFFSET,
                    rsp_offset = const TC_RSP_CPL0_OFFSET,
                );
            }
        }
        if let Ok(thread) = MASTER_THREAD_TABLE.write().get_mut(next_tid) {
            let ctx_ptr = &raw mut thread.context;
            unsafe {
                core::arch::asm!(
                    "wrgsbase {context}",
                    "mov {clobber}, gs:[{cr3_offset}]",
                    "mov cr3, {clobber}",
                    "mov rsp, gs:[{rsp_offset}]",
                    "pop r15",
                    "pop r14",
                    "pop r13",
                    "pop r12",
                    "pop rbp",
                    "pop rbx",
                    context = in(reg) ctx_ptr,
                    clobber = out(reg) _,
                    cr3_offset = const TC_CR3_OFFSET,
                    rsp_offset = const TC_RSP_CPL0_OFFSET,
                );
            }
            X2Apic::signal_eoi();
            if let Ok(mut lapic) = LAPICS.try_get_mut() {
                lapic.timer.reset().expect("Failed to reset LAPIC timer")
            }
            YieldStatus::Success
        } else {
            YieldStatus::InvalidTidFromSched
        }
    } else {
        await_interrupt!()
    }
}
