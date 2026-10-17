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

use spin::Mutex;

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
    memory::{
        AddressSpaceInterface,
        KERNEL_AS,
        allocators::memory::{
            PageSize,
            RetiredKernelRange,
        },
        linear::{
            VAddr,
            address_map::LA_MAP,
        },
    },
};

/// Exact authority for one kernel stack range. Only successful allocation can
/// construct it. It is neither Copy nor Clone, and no scalar adoption API exists.
/// The containing Stacks owner retains its root and original admission through
/// explicit release. Dropping this token never unmaps or frees backing.
#[derive(Debug)]
#[must_use]
pub(in crate::memory) struct KernelStack {
    base: VAddr,
    pages: usize,
    release_started: bool,
}

impl KernelStack {
    pub(in crate::memory) fn base(&self) -> VAddr {
        self.base
    }

    pub(in crate::memory) fn reject_release_for_test(&mut self) -> Result<(), Error> {
        self.release_with(|_, _| Err(Error::InvalidStack))
    }

    fn release_with(
        &mut self,
        release: impl FnOnce(VAddr, usize) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.release_started {
            return Err(Error::InvalidStack);
        }
        // Arm before detachment or callbacks. Failure/interruption is terminal;
        // the containing pair must retain its original admission and root.
        self.release_started = true;
        release(self.base, self.pages)
    }
}

/// Serializes *all* kernel-stack arena management (free-region search, mapping,
/// and teardown) across logical processors.
///
/// Without it, two LPs could each `find_free_region` and receive the same base
/// before either maps it, or reuse a range before its detachment is complete.
/// The entire alloc/free operation is therefore performed under this lock,
/// and interrupts are masked while it is held so a timer preemption cannot
/// re-enter the allocator (e.g. via `reap_dead_threads` -> `deallocate_stack`)
/// and self-deadlock on the same lock.
static STACK_ARENA_LOCK: Mutex<()> = Mutex::new(());

/// Runs `f` with interrupts masked and [`STACK_ARENA_LOCK`] held, releasing the
/// lock *before* restoring the previous interrupt state (so a preemption taken
/// right after unmasking cannot observe the lock still held by this LP).
#[cfg(target_arch = "aarch64")]
fn current_sp() -> u64 {
    let sp: u64;
    unsafe {
        core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

#[cfg(target_arch = "x86_64")]
fn current_sp() -> u64 {
    let sp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

fn with_arena<R>(operation: u64, subject: u64, f: impl FnOnce() -> R) -> R {
    let ints_were_enabled = get_int_state();
    mask_interrupts!();
    crate::debug_trace::trace(
        crate::debug_trace::TAG_STACK_ARENA_WAIT,
        operation,
        current_sp(),
        subject,
    );
    let result = {
        let _lock = STACK_ARENA_LOCK.lock();
        crate::debug_trace::trace(
            crate::debug_trace::TAG_STACK_ARENA_ACQUIRED,
            operation,
            current_sp(),
            subject,
        );
        f()
    };
    crate::debug_trace::trace(
        crate::debug_trace::TAG_STACK_ARENA_RELEASED,
        operation,
        current_sp(),
        subject,
    );
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
/// usable stack region, page-aligned. The returned owner records its exact
/// range; one unmapped guard page remains below and above. Guards carry no
/// mutable metadata and adjacent stacks may share the same unmapped page.
/// Used only by the owning admitted stack transaction, which retains its
/// reservation on incomplete physical rollback or interrupted preparation.
pub(crate) struct AllocationFailure {
    pub(crate) error: Error,
    pub(crate) retained: bool,
}

pub(in crate::memory) fn allocate_stack(n_pages: usize) -> Result<KernelStack, AllocationFailure> {
    if n_pages == 0 || n_pages > memory::KERNEL_RANGE_FRAME_CAPACITY {
        return Err(AllocationFailure {
            error: Error::InvalidStack,
            retained: false,
        });
    }
    let mut retirement = RetiredKernelRange::new();
    // Serialize the whole region-search-then-map sequence against other LPs.
    let result = with_arena(1, n_pages as u64, || allocate_stack_locked(n_pages, &mut retirement));
    // Failed preparation must not free its prefix under the arena/table locks.
    if let Err(error) = retirement.release() {
        return Err(AllocationFailure {
            error: error.into(),
            retained: true,
        });
    }
    if let Ok(stack) = &result
        && crate::cpu::isa::memory::tlb::try_inval_range_kernel(stack.base, stack.pages).is_err()
    {
        return Err(AllocationFailure {
            error: Error::AllocatorsMemory(memory::Error::RetirementFailed),
            retained: true,
        });
    }
    result.map_err(|error| AllocationFailure {
        error,
        retained: false,
    })
}

fn allocate_stack_locked(
    n_pages: usize,
    retirement: &mut RetiredKernelRange,
) -> Result<KernelStack, Error> {
    const NUM_GUARD_PAGES: usize = 2;
    let page = PageSize::Standard.num_bytes();
    // find a suitable range in the kernel stack arena
    let stack_region_base = KERNEL_AS.lock().find_free_region(
        n_pages + NUM_GUARD_PAGES,
        (*LA_MAP.get_region(crate::memory::linear::address_map::RegionType::KernelStackArena))
            .into(),
    )?;
    // One guard page below the usable region, then `n_pages` usable, then one
    // guard page above.
    let stack_buf_base = stack_region_base + page * (NUM_GUARD_PAGES / 2);
    memory::try_allocate_and_map_range(
        stack_buf_base,
        memory::PageSize::Standard,
        n_pages,
        retirement,
    )?;
    Ok(KernelStack {
        base: stack_buf_base,
        pages: n_pages,
        release_started: false,
    })
}

/// Release only the original range owner, once, with the containing stack pair
/// and its original root/admission retained. Guards are never mapped or detached.
pub(in crate::memory) fn deallocate_stack(stack: &mut KernelStack) -> Result<(), Error> {
    stack.release_with(|base, pages| {
        let mut retirement = RetiredKernelRange::new();
        let result = with_arena(2, base.into(), || {
            memory::retire_kernel_range(base, PageSize::Standard, pages, &mut retirement)
        });
        // Backing stays marked in-use through post-guard invalidation/release.
        // Either failure retains the receipt and the containing pair's charge.
        retirement.release()?;
        result.map_err(Error::from)
    })
}
