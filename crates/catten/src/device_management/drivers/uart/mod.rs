//! # Universal Asynchronous Receiver/Transmitter (UART) Drivers

pub mod ns16x50;

use core::fmt::Write;
use core::marker::Sized;

pub trait Uart {}
