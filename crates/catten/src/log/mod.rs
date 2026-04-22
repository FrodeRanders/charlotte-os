//! # Kernel Logging Macros
//!
//! This module provides convenient macros for logging messages to the kernel
//! log. They will be updated as the kernel develops to provide more
//! functionality and use an actual kernel log that will reside in memory and be
//! stored in a file. For now they print to the framebuffer and on x86_64 systems the COM1 serial
//! port with the legacy_com_ports feature enabled.

#[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))]
use spin::lazy::Lazy;
#[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))]
use spin::mutex::Mutex;

#[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))]
use crate::cpu::isa::io;
#[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))]
use crate::drivers::uart::Uart;
#[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))]
use crate::drivers::uart::ns16550::{Ns16550, legacy_ports};

#[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))]
pub static LOG_PORT: spin::Lazy<spin::Mutex<Ns16550>> =
    Lazy::new(|| Mutex::new(Ns16550::try_new(io::IoReg8::IoPort(legacy_ports::COM1)).unwrap()));

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

#[macro_export]
macro_rules! early_log {
    ($text:expr $(, $arg:tt)*) => ({
        let interrupts_were_enabled = $crate::log::early_save_interrupts();
        #[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))] {
            use core::fmt::Write;
            use $crate::log::LOG_PORT;
            let _ = write!(LOG_PORT.lock(), $text $(, $arg)*);
        }
        use $crate::print;
        print!($text $(, $arg)*);
        $crate::log::early_restore_interrupts(interrupts_were_enabled);
    })
}

#[macro_export]
macro_rules! early_logln {
    ($text:expr $(, $arg:tt)*) => ({
        let interrupts_were_enabled = $crate::log::early_save_interrupts();
        #[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))] {
            use core::fmt::Write;
            use $crate::log::LOG_PORT;
            let _ = writeln!(LOG_PORT.lock(), $text $(, $arg)*);
        }
        use $crate::println;
        println!($text $(, $arg)*);
        $crate::log::early_restore_interrupts(interrupts_were_enabled);
    })
}

#[macro_export]
macro_rules! log {
    ($text:expr $(, $arg:tt)*) => ({
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.save_int();
        #[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))] {
            use core::fmt::Write;
            use $crate::log::LOG_PORT;
            let _ = write!(LOG_PORT.lock(), $text $(, $arg)*);
        }
        use $crate::print;
        print!($text $(, $arg)*);
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.restore_int();
    })
}
#[macro_export]
macro_rules! logln {
    ($text:expr $(, $arg:tt)*) => ({
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.save_int();
        #[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))] {
            use core::fmt::Write;
            use $crate::log::LOG_PORT;
            let _ = writeln!(LOG_PORT.lock(), $text $(, $arg)*);
        }
        use $crate::println;
        println!($text $(, $arg)*);
        $crate::cpu::multiprocessor::interrupt_tracking::INT_STATE.restore_int();
    })
}
