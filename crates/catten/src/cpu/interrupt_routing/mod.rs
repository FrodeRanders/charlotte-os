//! # The Interrupt Routing Manager
//!
//! The interrupt routing manager is responsible for the following:
//! - Allocating and deallocating interrupt vectors to devices
//! - Routing interrupts from devices to the appropriate interrupt vector
//! - Managing interrupt redirection tables for both platform level interrupt controllers and
//!   IOMMUs.
//! - Providing a unified interface for devices to register and unregister interrupt handlers.
//! - Ensuring that the interrupt service load is roughly balanced across all logical processors in
//!   the system.

use hashbrown::HashMap;

pub struct InterruptDescriptor {
    pub gsi: u32,
    pub flags: u16,
}

pub type IsaIrqOverrideTable = HashMap<u8, InterruptDescriptor>;
