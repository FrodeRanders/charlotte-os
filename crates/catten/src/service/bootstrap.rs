//! Bootstrap capability delivery: the config-page contract.
//!
//! Each spawned domain receives exactly one initial capability id in the
//! bootstrap slot of its config page. The capability's meaning depends on
//! the program's role:
//!
//! - the name service receives its own registry *endpoint* capability
//!   (created for it by the supervisor before it starts);
//! - every other service or client receives a *connection* capability to
//!   the name service, delegated by the supervisor.
//!
//! Names, lookup policy, and restart generations live entirely in the
//! userspace name service; the kernel only moves opaque capabilities.
#![cfg(target_arch = "aarch64")]

use crate::memory::physical::PAddr;

/// Byte offset of the bootstrap capability slot in the config page.
///
/// Must match `catten_rt::config::BOOTSTRAP_CAP_OFFSET`.
pub const BOOTSTRAP_CAP_OFFSET: usize = 16;

/// Byte offset of the launch argument count (`catten-rt` contract).
pub const ARGC_OFFSET: usize = 24;

/// Byte offset of the delegated MMIO-region device capability slot.
///
/// Must match `catten_rt::config::MMIO_CAP_OFFSET`.
pub const MMIO_CAP_OFFSET: usize = 2048;

/// Byte offset of the delegated interrupt device capability slot.
///
/// Must match `catten_rt::config::IRQ_CAP_OFFSET`.
pub const IRQ_CAP_OFFSET: usize = 2056;

/// Write the bootstrap capability id into a domain's config page.
pub fn write_bootstrap_cap(config_frame: PAddr, cap: u64) {
    let base: *mut u8 = config_frame.into();
    unsafe {
        core::ptr::write_volatile(base.add(BOOTSTRAP_CAP_OFFSET) as *mut u64, cap);
    }
}

/// Write a delegated MMIO-region device capability into a driver domain's
/// config page (architecture doc §10.1, Phase 8).
pub fn write_mmio_cap(config_frame: PAddr, cap: u64) {
    let base: *mut u8 = config_frame.into();
    unsafe {
        core::ptr::write_volatile(base.add(MMIO_CAP_OFFSET) as *mut u64, cap);
    }
}

/// Write a delegated interrupt device capability into a driver domain's
/// config page (architecture doc §10.1, Phase 8).
pub fn write_irq_cap(config_frame: PAddr, cap: u64) {
    let base: *mut u8 = config_frame.into();
    unsafe {
        core::ptr::write_volatile(base.add(IRQ_CAP_OFFSET) as *mut u64, cap);
    }
}

/// Write the launch argument count into a domain's config page.
pub fn write_argc(config_frame: PAddr, argc: usize) {
    let base: *mut u8 = config_frame.into();
    unsafe {
        core::ptr::write_volatile(base.add(ARGC_OFFSET) as *mut usize, argc);
    }
}
