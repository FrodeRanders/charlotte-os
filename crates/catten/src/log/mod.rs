//! # Kernel Logging Macros
//!
//! This module provides convenient macros for logging messages to the kernel
//! log. They will be updated as the kernel develops to provide more
//! functionality and use an actual kernel log that will reside in memory and be
//! stored in a file.
//!
//! Output backends:
//! - On AArch64 a PL011 UART serial console ([`serial`]) is always available,
//!   including as an early console before the rest of the system is up, since
//!   Limine maps the platform MMIO into the HHDM from the first instruction.
//! - When the `display` feature is enabled a framebuffer terminal
//!   ([`flanterm`]) is used for the ordinary `log!`/`logln!` macros.

#[cfg(feature = "display")]
mod chars;
#[cfg(feature = "display")]
pub mod flanterm;
#[cfg(target_arch = "aarch64")]
pub mod serial;

#[inline(always)]
pub fn early_save_interrupts() -> bool {
    #[cfg(target_arch = "x86_64")]
    let interrupts_were_enabled = crate::cpu::isa::lp::ops::get_int_state();

    #[cfg(not(target_arch = "x86_64"))]
    let interrupts_were_enabled = true;

    crate::cpu::isa::lp::ops::mask_interrupts!();
    interrupts_were_enabled
}

#[inline(always)]
pub fn early_restore_interrupts(interrupts_were_enabled: bool) {
    if interrupts_were_enabled {
        crate::cpu::isa::lp::ops::unmask_interrupts!();
    }
}

/// Early, dependency-light log output. On AArch64 this writes to the PL011
/// serial console, which is usable from the very first instruction of the
/// kernel; on other architectures it is currently a no-op pending an equivalent
/// early console.
#[macro_export]
macro_rules! early_log {
    ($text:expr $(, $arg:tt)*) => {{
        #[cfg(target_arch = "aarch64")]
        {
            use core::fmt::Write;
            let _ = write!($crate::log::serial::SERIAL.lock(), $text $(, $arg)*);
        }
    }};
}

#[macro_export]
macro_rules! early_logln {
    ($text:expr $(, $arg:tt)*) => {{
        #[cfg(target_arch = "aarch64")]
        {
            use core::fmt::Write;
            let _ = writeln!($crate::log::serial::SERIAL.lock(), $text $(, $arg)*);
        }
    }};
}

#[macro_export]
macro_rules! log {
    ($text:expr $(, $arg:tt)*) => ({
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.save_int();
        #[cfg(feature = "display")]
        {
            use core::fmt::Write;
            let _ = write!($crate::log::flanterm::FT_CTX.lock(), $text $(, $arg)*);
        }
        #[cfg(all(target_arch = "aarch64", not(feature = "display")))]
        {
            use core::fmt::Write;
            let _ = write!($crate::log::serial::SERIAL.lock(), $text $(, $arg)*);
        }
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.restore_int();
    })
}
#[macro_export]
macro_rules! logln {
    ($text:expr $(, $arg:tt)*) => ({
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.save_int();
        #[cfg(feature = "display")]
        {
            use core::fmt::Write;
            let _ = writeln!($crate::log::flanterm::FT_CTX.lock(), $text $(, $arg)*);
        }
        #[cfg(all(target_arch = "aarch64", not(feature = "display")))]
        {
            use core::fmt::Write;
            let _ = writeln!($crate::log::serial::SERIAL.lock(), $text $(, $arg)*);
        }
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.restore_int();
    })
}
