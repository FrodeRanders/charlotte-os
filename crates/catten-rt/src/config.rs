//! Typed access to the canonical kernel↔userspace launch and status pages.
//!
//! The kernel maps read-only launch metadata at `CONFIG_VADDR` and a separate
//! mutable diagnostic/status page at `STATUS_VADDR` in every runtime domain.

use charlotte_launch::{
    CAPABILITY_VECTOR_CAPACITY,
    CapabilityRecord,
    LAUNCH_HEADER_OFFSET,
    LaunchHeader,
    MANIFEST_VECTOR_CAPACITY,
    ManifestRecord,
};
pub use charlotte_launch::{
    CONFIG_PAGE_SIZE,
    CONFIG_VADDR,
    CapabilityKind,
    INPUT_CAPACITY,
    INPUT_VADDR,
    ManifestValueKind,
    STATUS_PAGE_SIZE,
    STATUS_VADDR,
};

/// Check the fixed-width launch header before crt0 interprets any other field.
pub fn launch_header_is_compatible() -> bool {
    launch_header().is_compatible()
}

pub(crate) fn manifest_record(index: usize) -> Option<ManifestRecord> {
    let header = launch_header();
    if !header.is_compatible() || index >= header.manifest_count as usize {
        return None;
    }
    let count = core::cmp::min(header.manifest_count as usize, MANIFEST_VECTOR_CAPACITY);
    if index >= count {
        return None;
    }
    let records = (CONFIG_VADDR + header.manifest_offset as usize) as *const ManifestRecord;
    Some(unsafe { core::ptr::read_volatile(records.add(index)) })
}

pub(crate) fn manifest_bytes(record: ManifestRecord) -> Option<&'static [u8]> {
    if ManifestValueKind::from_raw(record.kind) != Some(ManifestValueKind::Bytes) {
        return None;
    }
    let header = launch_header();
    if !header.is_compatible() {
        return None;
    }
    let offset = usize::try_from(record.value).ok()?;
    let len = record.value_len as usize;
    let data_start = header.manifest_data_offset as usize;
    let data_end = data_start.checked_add(header.manifest_data_size as usize)?;
    let value_end = offset.checked_add(len)?;
    if offset < data_start || value_end > data_end {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts((CONFIG_VADDR + offset) as *const u8, len) })
}

pub(crate) fn launch_layout() -> LaunchHeader {
    launch_header()
}

fn launch_header() -> LaunchHeader {
    unsafe {
        core::ptr::read_volatile((CONFIG_VADDR + LAUNCH_HEADER_OFFSET) as *const LaunchHeader)
    }
}

fn capability(kind: CapabilityKind) -> Option<u64> {
    let header = launch_header();
    if !header.is_compatible() {
        return None;
    }
    let count = core::cmp::min(header.capabilities_count as usize, CAPABILITY_VECTOR_CAPACITY);
    let records = (CONFIG_VADDR + header.capabilities_offset as usize) as *const CapabilityRecord;
    for index in 0..count {
        let record = unsafe { core::ptr::read_volatile(records.add(index)) };
        if CapabilityKind::from_raw(record.kind) == Some(kind) {
            return Some(record.handle);
        }
    }
    None
}

pub(crate) fn capability_record(index: usize) -> Option<CapabilityRecord> {
    let header = launch_header();
    if !header.is_compatible() || index >= header.capabilities_count as usize {
        return None;
    }
    let records = (CONFIG_VADDR + header.capabilities_offset as usize) as *const CapabilityRecord;
    Some(unsafe { core::ptr::read_volatile(records.add(index)) })
}

fn capability_count(kind: CapabilityKind) -> u32 {
    let header = launch_header();
    if !header.is_compatible() {
        return 0;
    }
    let count = core::cmp::min(header.capabilities_count as usize, CAPABILITY_VECTOR_CAPACITY);
    let records = (CONFIG_VADDR + header.capabilities_offset as usize) as *const CapabilityRecord;
    let mut matches = 0;
    for index in 0..count {
        let record = unsafe { core::ptr::read_volatile(records.add(index)) };
        if CapabilityKind::from_raw(record.kind) == Some(kind) {
            matches += 1;
        }
    }
    matches
}

/// Read the delegated MMIO-region capability, or `None` if none was granted.
pub fn mmio_cap() -> Option<u64> {
    capability(CapabilityKind::Mmio)
}

/// Read the delegated interrupt capability, or `None` if none was granted.
pub fn irq_cap() -> Option<u64> {
    capability(CapabilityKind::Interrupt)
}

/// The per-shard CQ ring base virtual address slot.
pub const SHARD_CQ_BASE_OFFSET: usize = 2064;

/// The per-shard CQ ring count slot.
pub const SHARD_CQ_COUNT_OFFSET: usize = 2072;

/// Base virtual address of the per-shard completion-queue ring array (queue
/// id `i + 1`, ring at `base + i * 4096`), or `None` if the loader mapped no
/// per-shard rings. A shard executor waits on its own ring so a wake targeted
/// at one shard never releases another.
pub fn shard_cq_base() -> Option<usize> {
    let base = unsafe { read_launch::<u64>(SHARD_CQ_BASE_OFFSET) } as usize;
    if base == 0 {
        None
    } else {
        Some(base)
    }
}

/// Number of per-shard completion-queue rings the loader mapped.
pub fn shard_cq_count() -> usize {
    unsafe { read_launch::<u64>(SHARD_CQ_COUNT_OFFSET) as usize }
}

/// How many handoff memory-object state caps the supervisor delivered.
pub fn handoff_count() -> u32 {
    capability_count(CapabilityKind::HandoffState)
}

/// The first handoff state memory-object cap, or 0 if none.
pub fn handoff_state_cap() -> u64 {
    capability(CapabilityKind::HandoffState).unwrap_or(0)
}

/// The old endpoint capability (for re-registration), or 0 if none.
pub fn handoff_endpoint_cap() -> u64 {
    capability(CapabilityKind::HandoffEndpoint).unwrap_or(0)
}

/// Output/status words begin at the start of the dedicated status page.
pub const OUTPUT_OFFSET: usize = 0;

/// Read the bootstrap capability id delivered by the supervisor, or `None`
/// when no capability was delivered.
pub fn bootstrap_cap() -> Option<u64> {
    capability(CapabilityKind::Bootstrap)
}

unsafe fn read_launch<T: Copy>(offset: usize) -> T {
    assert!(offset.is_multiple_of(core::mem::align_of::<T>()));
    assert!(offset.saturating_add(core::mem::size_of::<T>()) <= CONFIG_PAGE_SIZE as usize);
    unsafe { core::ptr::read_volatile((CONFIG_VADDR as *const u8).add(offset) as *const T) }
}

/// Read a value of type `T` from `offset` bytes into the mutable status page.
///
/// `offset` should be a multiple of `align_of::<T>()`.
///
/// # Safety
/// The caller must ensure that a value of type `T` was written at `offset` by
/// the kernel (or by a prior [`write`] in this program). Reading a location
/// that has never been written is sound (the kernel zeros the page), but its
/// value is unspecified.
pub unsafe fn read<T: Copy>(offset: usize) -> T {
    assert!(offset.is_multiple_of(core::mem::align_of::<T>()));
    assert!(offset.saturating_add(core::mem::size_of::<T>()) <= STATUS_PAGE_SIZE as usize);
    unsafe { core::ptr::read_volatile((STATUS_VADDR as *const u8).add(offset) as *const T) }
}

/// Write `value` of type `T` to `offset` bytes into the mutable status page.
///
/// `offset` should be a multiple of `align_of::<T>()`.
pub fn write<T: Copy>(offset: usize, value: T) {
    assert!(offset.is_multiple_of(core::mem::align_of::<T>()));
    assert!(offset.saturating_add(core::mem::size_of::<T>()) <= STATUS_PAGE_SIZE as usize);
    unsafe {
        core::ptr::write_volatile((STATUS_VADDR as *mut u8).add(offset) as *mut T, value);
    }
}

/// Pointer to the canonical output/status area at the start of the status page.
pub fn output_ptr<T>() -> *mut T {
    (STATUS_VADDR + OUTPUT_OFFSET) as *mut T
}
