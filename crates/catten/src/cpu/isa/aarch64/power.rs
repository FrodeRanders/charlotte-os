//! AArch64 firmware power transitions.

use crate::environment::acpi::sdt::discovery::{
    PsciConduit,
    fadt_psci_conduit,
};

/// PSCI v0.2 `SYSTEM_OFF` function identifier.
const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;

/// Ask platform firmware to remove power from the whole system.
///
/// Callers must first drain protected domains and independently establish
/// device quiescence. A conforming PSCI implementation does not return from
/// `SYSTEM_OFF`; returning is treated as a fatal firmware failure because the
/// node is no longer allowed to resume ordinary scheduling after shutdown.
pub fn power_off() -> ! {
    let conduit = fadt_psci_conduit()
        .expect("AArch64 system poweroff requires a PSCI conduit advertised by ACPI");
    crate::logln!("[shutdown] POWER-OFF REQUESTED via PSCI {:?}", conduit);
    crate::mask_interrupts!();
    let status: i64;
    unsafe {
        match conduit {
            PsciConduit::Smc => core::arch::asm!(
                "smc #0",
                inlateout("x0") PSCI_SYSTEM_OFF => status,
                in("x1") 0u64,
                in("x2") 0u64,
                in("x3") 0u64,
                clobber_abi("C"),
                options(nostack)
            ),
            PsciConduit::Hvc => core::arch::asm!(
                "hvc #0",
                inlateout("x0") PSCI_SYSTEM_OFF => status,
                in("x1") 0u64,
                in("x2") 0u64,
                in("x3") 0u64,
                clobber_abi("C"),
                options(nostack)
            ),
        }
    }
    panic!("PSCI SYSTEM_OFF returned status {status}");
}
