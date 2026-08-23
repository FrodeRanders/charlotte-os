use core::arch::asm;

use super::interrupts::load_ivt;
use crate::{
    cpu::isa::{
        interface::init::InitInterface,
        lp::ops::get_lp_id,
    },
    early_logln,
    logln,
};

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
        let (sctlr_before, sctlr_after) = clear_write_execute_never();
        early_logln!(
            "BSP: SCTLR_EL1 WXN clear before={:#x} after={:#x}",
            sctlr_before,
            sctlr_after
        );
        load_ivt();
        early_logln!("BSP: Aarch64 ISA initialization complete.");
        Ok(())
    }

    fn init_ap() -> Result<(), Self::Error> {
        clear_write_execute_never();
        load_ivt();
        logln!("LP {}: Aarch64 ISA initialization complete.", get_lp_id());
        Ok(())
    }

    fn deinit() -> Result<(), Self::Error> {
        // Deinitialization code for the aarch64 architecture
        Ok(())
    }
}
