//! # Universal Asynchronous Receiver/Transmitter (UART) Drivers

pub mod ns16550;

use core::fmt::Write;
use core::marker::Sized;

use crate::cpu::isa::io::IoReg8;
