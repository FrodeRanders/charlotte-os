//! Typed access to the canonical kernel↔userspace config page.
//!
//! The kernel maps a single 4 KiB page at VADDR `0x0001_0000` in every user
//! address space and writes launch metadata there during setup.

/// The canonical config-page virtual address.
pub const CONFIG_VADDR: usize = 0x0000_0000_0001_0000;

/// Canonical launch input buffer virtual address.
pub const INPUT_VADDR: usize = 0x0000_0000_0001_2000;

/// Bytes available in the canonical launch input buffer.
pub const INPUT_CAPACITY: usize = 4096;

/// Number of 32-bit launch argument words at [`ARGS_OFFSET`].
pub const ARGC_OFFSET: usize = 24;

/// Launch argument words start here.
pub const ARGS_OFFSET: usize = 32;

/// The bootstrap capability slot.
///
/// The supervisor writes one initial capability id here before the domain
/// starts (architecture doc §16.7). Its type (endpoint vs. connection) is
/// determined by the program's role, not encoded in the slot. A value of 0
/// means "no bootstrap capability was delivered".
pub const BOOTSTRAP_CAP_OFFSET: usize = 16;

/// The delegated MMIO-region device capability slot (architecture doc §10.1,
/// Phase 8). A driver domain receives exactly the register windows its
/// manager grants here; 0 means "no MMIO region was delivered". Placed well
/// past the launch-argument region so it never collides with `argv`.
pub const MMIO_CAP_OFFSET: usize = 2048;

/// The delegated interrupt device capability slot (architecture doc §10.1).
/// 0 means "no interrupt was delivered".
pub const IRQ_CAP_OFFSET: usize = 2056;

/// Read the delegated MMIO-region capability, or `None` if none was granted.
pub fn mmio_cap() -> Option<u64> {
    let cap = unsafe { read::<u64>(MMIO_CAP_OFFSET) };
    if cap == 0 {
        None
    } else {
        Some(cap)
    }
}

/// Read the delegated interrupt capability, or `None` if none was granted.
pub fn irq_cap() -> Option<u64> {
    let cap = unsafe { read::<u64>(IRQ_CAP_OFFSET) };
    if cap == 0 {
        None
    } else {
        Some(cap)
    }
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
    let base = unsafe { read::<u64>(SHARD_CQ_BASE_OFFSET) } as usize;
    if base == 0 {
        None
    } else {
        Some(base)
    }
}

/// Number of per-shard completion-queue rings the loader mapped.
pub fn shard_cq_count() -> usize {
    unsafe { read::<u64>(SHARD_CQ_COUNT_OFFSET) as usize }
}

/// Byte offset of the handoff state count (u32) — how many memory-object
/// caps the supervisor delivered from the previous instance.
pub const HANDOFF_COUNT_OFFSET: usize = 2080;

/// Byte offset of the first handoff state capability id (u64).
pub const HANDOFF_STATE_OFFSET: usize = 2088;

/// Byte offset of the old endpoint capability id (u64) — the previous
/// instance's endpoint, delivered so the new instance can re-register it.
pub const HANDOFF_ENDPOINT_OFFSET: usize = 2096;

/// How many handoff memory-object state caps the supervisor delivered.
pub fn handoff_count() -> u32 {
    unsafe { read::<u32>(HANDOFF_COUNT_OFFSET) }
}

/// The first handoff state memory-object cap, or 0 if none.
pub fn handoff_state_cap() -> u64 {
    unsafe { read::<u64>(HANDOFF_STATE_OFFSET) }
}

/// The old endpoint capability (for re-registration), or 0 if none.
pub fn handoff_endpoint_cap() -> u64 {
    unsafe { read::<u64>(HANDOFF_ENDPOINT_OFFSET) }
}

/// Output/status words are intentionally kept at the beginning of the page so
/// existing kernel verifiers can poll `config[0]` as a sentinel.
pub const OUTPUT_OFFSET: usize = 0;

/// Read the bootstrap capability id delivered by the supervisor, or `None`
/// when no capability was delivered.
pub fn bootstrap_cap() -> Option<u64> {
    let cap = unsafe { read::<u64>(BOOTSTRAP_CAP_OFFSET) };
    if cap == 0 {
        None
    } else {
        Some(cap)
    }
}

/// Read a value of type `T` from `offset` bytes into the config page.
///
/// `offset` should be a multiple of `align_of::<T>()`.
///
/// # Safety
/// The caller must ensure that a value of type `T` was written at `offset` by
/// the kernel (or by a prior [`write`] in this program).  Reading a location
/// that has never been written is sound (the kernel zeros the page), but its
/// value is unspecified.
pub unsafe fn read<T: Copy>(offset: usize) -> T {
    unsafe { core::ptr::read_volatile((CONFIG_VADDR as *const u8).add(offset) as *const T) }
}

/// Write `value` of type `T` to `offset` bytes into the config page.
///
/// `offset` should be a multiple of `align_of::<T>()`.
pub fn write<T: Copy>(offset: usize, value: T) {
    unsafe {
        core::ptr::write_volatile((CONFIG_VADDR as *mut u8).add(offset) as *mut T, value);
    }
}

/// Pointer to the canonical output/status area at the start of the config page.
pub fn output_ptr<T>() -> *mut T {
    (CONFIG_VADDR + OUTPUT_OFFSET) as *mut T
}
