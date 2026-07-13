//! Typed access to the canonical kernel↔userspace config page.
//!
//! The kernel maps a single 4 KiB page at VADDR `0x0001_0000` in every user
//! address space and writes inputs there during setup.  Userspace programs
//! read inputs with [`read`] and publish results with [`write`].  No fixed
//! `RESULT_PAGE` or `READ_BUF` constants are needed — the config page is the
//! single shared-memory channel.

/// The canonical config-page virtual address.
pub const CONFIG_VADDR: usize = 0x0000_0000_0001_0000;

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
    unsafe { core::ptr::write_volatile((CONFIG_VADDR as *mut u8).add(offset) as *mut T, value); }
}
