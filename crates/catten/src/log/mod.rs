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

#[macro_export]
macro_rules! log {
    ($text:expr $(, $arg:tt)*) => ({
        #[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))] {
            use core::fmt::Write;
            use crate::log::LOG_PORT;
            let _ = write!(LOG_PORT.lock(), $text $(, $arg)*);
        }
        use crate::print;
        print!($text $(, $arg)*);
    })
}
#[macro_export]
macro_rules! logln {
    ($text:expr $(, $arg:tt)*) => ({
        #[cfg(all(target_arch = "x86_64", feature = "legacy_com_ports"))] {
            use core::fmt::Write;
            use crate::log::LOG_PORT;
            let _ = writeln!(LOG_PORT.lock(), $text $(, $arg)*);
        }
        use crate::println;
        println!($text $(, $arg)*);
    })
}
