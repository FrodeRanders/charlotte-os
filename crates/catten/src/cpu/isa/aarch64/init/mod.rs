use super::interrupts::load_ivt;
use core::arch::asm;
use crate::cpu::isa::interface::init::InitInterface;
use crate::{early_logln, logln};

pub struct IsaInitializer;

const SCTLR_EL1_WXN: u64 = 1 << 19;

#[derive(Debug)]
pub enum Error {
    // Error type for the aarch64 architecture
}

fn clear_write_execute_never() -> (u64, u64) {
    let before: u64;
    unsafe {
        asm!("mrs {}, sctlr_el1", out(reg) before, options(nomem, nostack, preserves_flags));
    }
    let after = before & !SCTLR_EL1_WXN;
    if after != before {
        unsafe {
            asm!(
                "msr sctlr_el1, {sctlr}",
                "isb",
                sctlr = in(reg) after,
                options(nostack, preserves_flags),
            );
        }
    }
    (before, after)
}

impl InitInterface for IsaInitializer {
    type Error = Error;

    #[inline(always)]
    fn init_bsp() -> Result<(), Self::Error> {
        // Initialization code for the aarch64 architecture
        early_logln!("Performing Aarch64 ISA specific initialization...");
        let (sctlr_before, sctlr_after) = clear_write_execute_never();
        early_logln!(
            "SCTLR_EL1 WXN clear: before={:#x} after={:#x}",
            sctlr_before,
            sctlr_after
        );
        // Setup the interrupt vector table
        early_logln!("Loading the interrupt vector table on the AP");
        load_ivt();
        early_logln!("Interrupt vector table loaded on the AP");

        early_logln!("Aarch64 ISA specific initialization complete!");
        Ok(())
    }

    fn init_ap() -> Result<(), Self::Error> {
        // Initialization code for the aarch64 architecture
        logln!("Performing Aarch64 ISA specific initialization...");
        let (sctlr_before, sctlr_after) = clear_write_execute_never();
        logln!(
            "SCTLR_EL1 WXN clear: before={:#x} after={:#x}",
            sctlr_before,
            sctlr_after
        );
        // Setup the interrupt vector table
        logln!("Loading the interrupt vector table on the AP");
        load_ivt();
        logln!("Interrupt vector table loaded on the AP");

        logln!("Aarch64 ISA specific initialization complete!");
        Ok(())
    }

    fn deinit() -> Result<(), Self::Error> {
        // Deinitialization code for the aarch64 architecture
        Ok(())
    }
}
