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

use charlotte_launch::{
    CAPABILITY_VECTOR_CAPACITY,
    CapabilityKind,
    CapabilityRecord,
    LAUNCH_HEADER_OFFSET,
    LaunchHeader,
    MANIFEST_DATA_CAPACITY,
    MANIFEST_VECTOR_CAPACITY,
    ManifestRecord,
    ManifestValueKind,
    PROFILE_CAPABILITY_RIGHT_MAP_READ,
    ProfileCapabilityMetadata,
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
    append_capability_record(config_frame, kind, handle, 0, 0);
}

fn append_capability_record(
    config_frame: PAddr,
    kind: CapabilityKind,
    handle: u64,
    rights: u16,
    metadata: u32,
) {
    let base: *mut u8 = config_frame.into();
    let header_ptr = unsafe { base.add(LAUNCH_HEADER_OFFSET) as *mut LaunchHeader };
    let mut header = unsafe { core::ptr::read_volatile(header_ptr) };
    assert!(header.is_compatible(), "capability written before launch header");
    let index = header.capabilities_count as usize;
    assert!(index < CAPABILITY_VECTOR_CAPACITY, "launch capability vector is full");
    let record = CapabilityRecord {
        kind: kind as u16,
        rights,
        metadata,
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

pub fn write_dma_domain_cap(config_frame: PAddr, cap: u64) {
    append_capability(config_frame, CapabilityKind::DmaDomain, cap);
}

/// Deliver the explicitly delegated system-wide observability authority.
pub fn write_system_observer_cap(config_frame: PAddr, cap: u64) {
    append_capability(config_frame, CapabilityKind::SystemObserver, cap);
}

/// Record a read-only launch profile. Its typed metadata selects the
/// meaningful byte prefix; the final memory-object page may contain padding.
pub fn write_profile_cap(config_frame: PAddr, cap: u64, metadata: ProfileCapabilityMetadata) {
    append_capability_record(
        config_frame,
        CapabilityKind::Profile,
        cap,
        PROFILE_CAPABILITY_RIGHT_MAP_READ,
        metadata.encode(),
    );
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
    assert!(state_count <= 1, "single-state handoff helper received multiple states");
    let state_caps = if state_count > 0 {
        core::slice::from_ref(&state_cap)
    } else {
        &[]
    };
    write_handoff_states(config_frame, state_caps, endpoint_cap);
}

/// Write every state capability delegated during a live upgrade.
pub fn write_handoff_states(config_frame: PAddr, state_caps: &[u64], endpoint_cap: u64) {
    let additional = state_caps.len() + usize::from(endpoint_cap != 0);
    let base: *mut u8 = config_frame.into();
    let header =
        unsafe { core::ptr::read_volatile(base.add(LAUNCH_HEADER_OFFSET) as *const LaunchHeader) };
    assert!(header.is_compatible(), "handoff written before launch header");
    assert!(
        header.capabilities_count as usize + additional <= CAPABILITY_VECTOR_CAPACITY,
        "handoff exceeds launch capability vector"
    );
    for &state_cap in state_caps {
        append_capability(config_frame, CapabilityKind::HandoffState, state_cap);
    }
    if endpoint_cap != 0 {
        append_capability(config_frame, CapabilityKind::HandoffEndpoint, endpoint_cap);
    }
}

#[derive(Clone, Copy)]
pub enum ManifestValue<'a> {
    Unsigned(u64),
    Signed(i64),
    Bytes(&'a [u8]),
}

#[derive(Clone, Copy)]
pub struct ManifestEntry<'a> {
    pub key: u64,
    pub flags: u16,
    pub value: ManifestValue<'a>,
}

/// Write the complete typed launch manifest into a domain's config page.
pub fn write_manifest(config_frame: PAddr, entries: &[ManifestEntry<'_>]) {
    assert!(entries.len() <= MANIFEST_VECTOR_CAPACITY, "launch manifest is full");
    let base: *mut u8 = config_frame.into();
    let header_ptr = unsafe { base.add(LAUNCH_HEADER_OFFSET) as *mut LaunchHeader };
    let mut header = unsafe { core::ptr::read_volatile(header_ptr) };
    assert!(header.is_compatible(), "manifest written before launch header");
    let records = unsafe { base.add(header.manifest_offset as usize) as *mut ManifestRecord };
    let mut data_size = 0usize;
    unsafe {
        for (index, entry) in entries.iter().copied().enumerate() {
            let (kind, value_len, value) = match entry.value {
                ManifestValue::Unsigned(value) => (ManifestValueKind::Unsigned, 8, value),
                ManifestValue::Signed(value) => (ManifestValueKind::Signed, 8, value as u64),
                ManifestValue::Bytes(bytes) => {
                    let end = data_size.checked_add(bytes.len()).expect("manifest data overflow");
                    assert!(end <= MANIFEST_DATA_CAPACITY, "launch manifest data is full");
                    let offset = header.manifest_data_offset as usize + data_size;
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), base.add(offset), bytes.len());
                    data_size = end;
                    (
                        ManifestValueKind::Bytes,
                        u32::try_from(bytes.len()).expect("manifest byte value exceeds ABI width"),
                        offset as u64,
                    )
                }
            };
            core::ptr::write_volatile(
                records.add(index),
                ManifestRecord {
                    key: entry.key,
                    kind: kind as u16,
                    flags: entry.flags,
                    value_len,
                    value,
                },
            );
        }
        header.manifest_count = entries.len() as u32;
        header.manifest_data_size = data_size as u32;
        core::ptr::write_volatile(header_ptr, header);
    }
}
