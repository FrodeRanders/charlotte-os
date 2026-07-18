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

use alloc::collections::BTreeMap;
use core::ops::Bound::{
    Excluded,
    Unbounded,
};

use spin::{
    LazyLock,
    Mutex,
    RwLock,
};

use super::memory;
use crate::{
    cpu::isa::{
        lp::ops::{
            get_int_state,
            mask_interrupts,
            unmask_interrupts,
        },
        memory::{
            MemoryInterface,
            MemoryInterfaceImpl,
        },
    },
    logln,
    memory::{
        AddressSpaceInterface,
        KERNEL_AS,
        allocators::memory::PageSize,
        linear::{
            VAddr,
            address_map::LA_MAP,
        },
    },
};

/// Reference-counted set of kernel stack guard-page addresses.
///
/// `find_free_region` places stacks back-to-back and only checks whether pages
/// are *mapped*; guard pages are intentionally left unmapped, so one stack's
/// upper guard page and the next stack's lower guard page can be the *same*
/// address. Reference counting lets such a shared guard survive until *both*
/// stacks are freed, instead of the first free removing a guard the second
/// still relies on.
static KERNEL_GUARD_PAGES: LazyLock<RwLock<BTreeMap<VAddr, usize>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Serializes *all* kernel-stack arena management (free-region search, mapping,
/// guard-page bookkeeping, and teardown) across logical processors.
///
/// Without it, two LPs could each `find_free_region` and receive the same base
/// before either maps it — a TOCTOU that overlaps two stacks — or race on the
/// guard-page map while another LP unmaps an adjacent stack. The entire
/// alloc/free operation is therefore performed under this lock, and interrupts
/// are masked while it is held so a timer preemption on the same LP cannot
/// re-enter the allocator (e.g. via `reap_dead_threads` -> `deallocate_stack`)
/// and self-deadlock on the same lock.
static STACK_ARENA_LOCK: Mutex<()> = Mutex::new(());

/// Runs `f` with interrupts masked and [`STACK_ARENA_LOCK`] held, releasing the
/// lock *before* restoring the previous interrupt state (so a preemption taken
/// right after unmasking cannot observe the lock still held by this LP).
fn with_arena<R>(f: impl FnOnce() -> R) -> R {
    let ints_were_enabled = get_int_state();
    mask_interrupts!();
    let result = {
        let _lock = STACK_ARENA_LOCK.lock();
        f()
    };
    if ints_were_enabled {
        unmask_interrupts!();
    }
    result
}
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
    // Serialize the whole region-search-then-map sequence against other LPs.
    with_arena(|| allocate_stack_locked(n_pages))
}

fn allocate_stack_locked(n_pages: usize) -> Result<VAddr, Error> {
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
    memory::try_allocate_and_map_range(stack_buf_base, memory::PageSize::Standard, n_pages)?;
    // Record the guard pages (reference-counted; a guard may be shared with an
    // adjacent stack).
    let mut guards = KERNEL_GUARD_PAGES.write();
    *guards.entry(lower_guard).or_insert(0) += 1;
    *guards.entry(upper_guard).or_insert(0) += 1;
    Ok(stack_buf_base)
}

/// Deallocate a kernel stack previously allocated by [`allocate_stack`]. The
/// argument is the base address returned by `allocate_stack`.
pub fn deallocate_stack(stack_buf_base: VAddr) -> Result<(), Error> {
    // Serialize teardown against concurrent alloc/free on other LPs.
    with_arena(|| deallocate_stack_locked(stack_buf_base))
}

fn deallocate_stack_locked(stack_buf_base: VAddr) -> Result<(), Error> {
    let page = PageSize::Standard.num_bytes();
    let lower_guard = stack_buf_base - page;
    // The number of usable pages is the distance from the base up to the next
    // (upper) guard page.
    let upper_guard = {
        let guards = KERNEL_GUARD_PAGES.read();
        if !guards.contains_key(&lower_guard) {
            return Err(Error::InvalidStack);
        }
        guards
            .range((Excluded(&lower_guard), Unbounded))
            .next()
            .map(|(addr, _)| *addr)
            .ok_or(Error::InvalidStack)?
    };
    let n_pages = (upper_guard - stack_buf_base) as usize / page;
    memory::unmap_and_deallocate_range(stack_buf_base, PageSize::Standard, n_pages);
    // Drop a reference on each guard page; remove it only when no adjacent stack
    // still relies on it.
    let mut guards = KERNEL_GUARD_PAGES.write();
    for guard in [lower_guard, upper_guard] {
        if let Some(count) = guards.get_mut(&guard) {
            *count -= 1;
            if *count == 0 {
                guards.remove(&guard);
            }
        }
    }
    Ok(())
}
