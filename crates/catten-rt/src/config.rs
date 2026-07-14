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

/// Preserved for consumers that still need to inspect the kernel-assigned ASID.
pub const ASID_OFFSET: usize = 16;

/// Number of 32-bit launch argument words at [`ARGS_OFFSET`].
pub const ARGC_OFFSET: usize = 24;

/// Launch argument words start here.
pub const ARGS_OFFSET: usize = 32;

/// Output/status words are intentionally kept at the beginning of the page so
/// existing kernel verifiers can poll `config[0]` as a sentinel.
pub const OUTPUT_OFFSET: usize = 0;

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

/// Kernel-assigned address-space id for the current user image.
pub fn asid() -> usize {
    unsafe { read::<usize>(ASID_OFFSET) }
}

/// Pointer to the canonical output/status area at the start of the config page.
pub fn output_ptr<T>() -> *mut T {
    (CONFIG_VADDR + OUTPUT_OFFSET) as *mut T
}
