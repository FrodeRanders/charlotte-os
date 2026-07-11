//! # Kernel Stack Allocator
//!
//! This module provides a an allocator for kernel thread stacks.
//!
//! Please note that on all supported architectures, the stack grows towards lower addresses so this
//! is the highest address of the stack. Also be aware that stacks allocated using this allocator
//! are mapped into the kernel stack arena in the higher half which means it is only suitable for
//! allocating stacks for kernel threads. Stacks are surrounded on both sides by guard pages to
//! allow for safe stack overflow detection and when enabled for the owning thread, transparent
//! reallocation such that from that thread's perspective it is as if the stack overflow never
//! happened.

use alloc::collections::BTreeSet;
use core::ops::Bound::{Excluded, Unbounded};

use spin::{LazyLock, RwLock};

use super::memory;
use crate::cpu::isa::memory::{MemoryInterface, MemoryInterfaceImpl};
use crate::logln;
use crate::memory::allocators::memory::PageSize;
use crate::memory::linear::VAddr;
use crate::memory::linear::address_map::LA_MAP;
use crate::memory::{AddressSpaceInterface, KERNEL_AS};

static KERNEL_GUARD_PAGE_SET: LazyLock<RwLock<BTreeSet<VAddr>>> =
    LazyLock::new(|| RwLock::new(BTreeSet::new()));
#[derive(Debug)]
pub enum Error {
    IsaMemoryIfce(<MemoryInterfaceImpl as MemoryInterface>::Error),
    AllocatorsMemory(memory::Error),
    InvalidStack,
}

impl From<<MemoryInterfaceImpl as MemoryInterface>::Error> for Error {
    fn from(err: <MemoryInterfaceImpl as MemoryInterface>::Error) -> Self {
        Error::IsaMemoryIfce(err)
    }
}

impl From<memory::Error> for Error {
    fn from(err: memory::Error) -> Self {
        Error::AllocatorsMemory(err)
    }
}

/// Allocate a kernel stack with `n_pages` being the number of usable pages.
///
/// The address returned by this function is the base (lowest) address of the
/// usable stack region, page-aligned. The usable region is surrounded by one
/// guard page below and one above, which are recorded in
/// [`KERNEL_GUARD_PAGE_SET`] so the stack can later be validated and freed.
pub fn allocate_stack(n_pages: usize) -> Result<VAddr, Error> {
    const NUM_GUARD_PAGES: usize = 2;
    let page = PageSize::Standard.num_bytes();
    // find a suitable range in the kernel stack arena
    let stack_region_base = KERNEL_AS.lock().find_free_region(
        n_pages + NUM_GUARD_PAGES,
        (*LA_MAP.get_region(crate::memory::linear::address_map::RegionType::KernelStackArena))
            .clone()
            .into(),
    )?;
    // One guard page below the usable region, then `n_pages` usable, then one
    // guard page above.
    let lower_guard = stack_region_base;
    let stack_buf_base = stack_region_base + page * (NUM_GUARD_PAGES / 2);
    let upper_guard = stack_buf_base + page * n_pages;
    logln!("Mapping a thread stack at {stack_buf_base:?}.");
    memory::try_allocate_and_map_range(stack_buf_base, memory::PageSize::Standard, n_pages)?;
    logln!("Thread stack mapped.");
    // Record the guard pages so the stack can be validated and freed later.
    let mut guard_set = KERNEL_GUARD_PAGE_SET.write();
    guard_set.insert(lower_guard);
    guard_set.insert(upper_guard);
    Ok(stack_buf_base)
}

/// Deallocate a kernel stack previously allocated by [`allocate_stack`]. The
/// argument is the base address returned by `allocate_stack`.
pub fn deallocate_stack(stack_buf_base: VAddr) -> Result<(), Error> {
    let page = PageSize::Standard.num_bytes();
    let lower_guard = stack_buf_base - page;
    // The number of usable pages is the distance from the base up to the next
    // (upper) guard page.
    let upper_guard = {
        let guard_set = KERNEL_GUARD_PAGE_SET.read();
        if !guard_set.contains(&lower_guard) {
            return Err(Error::InvalidStack);
        }
        guard_set
            .range((Excluded(&lower_guard), Unbounded))
            .next()
            .copied()
            .ok_or(Error::InvalidStack)?
    };
    let n_pages = (upper_guard - stack_buf_base) as usize / page;
    memory::unmap_and_deallocate_range(stack_buf_base, PageSize::Standard, n_pages);
    let mut guard_set = KERNEL_GUARD_PAGE_SET.write();
    guard_set.remove(&lower_guard);
    guard_set.remove(&upper_guard);
    Ok(())
}
