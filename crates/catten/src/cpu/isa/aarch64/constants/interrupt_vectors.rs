//! # AArch64 Interrupt Vector Constants
//!
//! On AArch64 with a Generic Interrupt Controller (GIC), interrupts are
//! identified by INTIDs rather than an IDT vector number as on x86-64:
//! - INTIDs 0-15 are Software Generated Interrupts (SGIs), used for IPIs.
//! - INTIDs 16-31 are Private Peripheral Interrupts (PPIs), which include the
//!   per-core Generic Timer interrupts.
//! - INTIDs 32+ are Shared Peripheral Interrupts (SPIs).
//!
//! See the ARM Generic Interrupt Controller Architecture Specification for
//! details.

/// EL1 physical timer PPI (INTID 30) as presented by the GIC on the QEMU
/// `virt` machine and typical ARM platforms.
pub const LAPIC_TIMER_VECTOR: u32 = 30;

/// SGI used for asynchronous inter-processor interrupts.
pub const ASYNC_IPI_VECTOR: u32 = 0;

/// SGI used for synchronous inter-processor interrupts.
pub const SYNC_IPI_VECTOR: u32 = 1;
