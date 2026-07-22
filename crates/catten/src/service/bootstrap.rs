//! Bootstrap capability delivery: the config-page contract.
//!
//! Each spawned domain receives exactly one initial capability id in the
//! bootstrap slot of its config page. The capability's meaning depends on
//! the program's role:
//!
//! - the name service receives its own registry *endpoint* capability (created for it by the
//!   supervisor before it starts);
//! - every other service or client receives a *connection* capability to the name service,
//!   delegated by the supervisor.
//!
//! Names, lookup policy, and restart generations live entirely in the
//! userspace name service; the kernel only moves opaque capabilities.
#![cfg(target_arch = "aarch64")]

use charlotte_launch::{
    ARGS_OFFSET,
    CAPABILITY_VECTOR_CAPACITY,
    CapabilityKind,
    CapabilityRecord,
    LAUNCH_HEADER_OFFSET,
    LaunchHeader,
};

use crate::memory::physical::PAddr;

/// Byte offset of the per-shard CQ ring base virtual address.
///
/// Must match `catten_rt::config::SHARD_CQ_BASE_OFFSET`.
pub const SHARD_CQ_BASE_OFFSET: usize = 2064;

/// Byte offset of the per-shard CQ ring count.
///
/// Must match `catten_rt::config::SHARD_CQ_COUNT_OFFSET`.
pub const SHARD_CQ_COUNT_OFFSET: usize = 2072;

pub fn write_launch_header(config_frame: PAddr) {
    let base: *mut u8 = config_frame.into();
    unsafe {
        core::ptr::write_volatile(
            base.add(LAUNCH_HEADER_OFFSET) as *mut LaunchHeader,
            LaunchHeader::new(),
        );
    }
}

fn append_capability(config_frame: PAddr, kind: CapabilityKind, handle: u64) {
    let base: *mut u8 = config_frame.into();
    let header_ptr = unsafe { base.add(LAUNCH_HEADER_OFFSET) as *mut LaunchHeader };
    let mut header = unsafe { core::ptr::read_volatile(header_ptr) };
    assert!(header.is_compatible(), "capability written before launch header");
    let index = header.capabilities_count as usize;
    assert!(index < CAPABILITY_VECTOR_CAPACITY, "launch capability vector is full");
    let record = CapabilityRecord {
        kind: kind as u16,
        rights: 0,
        flags: 0,
        handle,
    };
    unsafe {
        let records = base.add(header.capabilities_offset as usize) as *mut CapabilityRecord;
        core::ptr::write_volatile(records.add(index), record);
        header.capabilities_count += 1;
        core::ptr::write_volatile(header_ptr, header);
    }
}

/// Write the bootstrap capability id into a domain's config page.
pub fn write_bootstrap_cap(config_frame: PAddr, cap: u64) {
    append_capability(config_frame, CapabilityKind::Bootstrap, cap);
}

/// Write a delegated MMIO-region device capability into a driver domain's
/// config page (architecture doc §10.1, Phase 8).
pub fn write_mmio_cap(config_frame: PAddr, cap: u64) {
    append_capability(config_frame, CapabilityKind::Mmio, cap);
}

/// Write a delegated interrupt device capability into a driver domain's
/// config page (architecture doc §10.1, Phase 8).
pub fn write_irq_cap(config_frame: PAddr, cap: u64) {
    append_capability(config_frame, CapabilityKind::Interrupt, cap);
}

/// Write the per-shard CQ ring layout (base virtual address and count) into a
/// domain's config page, so a user-space runtime can place each shard's
/// executor on its own completion queue (queue id `i + 1`, ring at
/// `base + i * 4096`).
pub fn write_shard_cq_layout(config_frame: PAddr, base_vaddr: usize, count: usize) {
    let base: *mut u8 = config_frame.into();
    unsafe {
        core::ptr::write_volatile(base.add(SHARD_CQ_BASE_OFFSET) as *mut u64, base_vaddr as u64);
        core::ptr::write_volatile(base.add(SHARD_CQ_COUNT_OFFSET) as *mut u64, count as u64);
    }
}

/// Write the handoff state (state-cap count, first state cap, old endpoint
/// cap) into a domain's config page so a replacement service can pick up
/// the previous instance's state and endpoint.
pub fn write_handoff_state(
    config_frame: PAddr,
    state_count: u32,
    state_cap: u64,
    endpoint_cap: u64,
) {
    if state_count > 0 {
        append_capability(config_frame, CapabilityKind::HandoffState, state_cap);
    }
    if endpoint_cap != 0 {
        append_capability(config_frame, CapabilityKind::HandoffEndpoint, endpoint_cap);
    }
}

/// Write the fixed-width launch argument vector into a domain's config page.
pub fn write_args(config_frame: PAddr, args: &[u32]) {
    let argc = u32::try_from(args.len()).expect("launch argument count exceeds ABI width");
    assert!(ARGS_OFFSET + args.len() * core::mem::size_of::<u32>() <= 2048);
    let base: *mut u8 = config_frame.into();
    let header_ptr = unsafe { base.add(LAUNCH_HEADER_OFFSET) as *mut LaunchHeader };
    let mut header = unsafe { core::ptr::read_volatile(header_ptr) };
    assert!(header.is_compatible(), "arguments written before launch header");
    header.args_count = argc;
    unsafe {
        let destination = base.add(ARGS_OFFSET) as *mut u32;
        for (index, argument) in args.iter().copied().enumerate() {
            core::ptr::write_volatile(destination.add(index), argument);
        }
        core::ptr::write_volatile(header_ptr, header);
    }
}
