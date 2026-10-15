//! A single owner covers both halves of a thread's admitted stack footprint.
// Bounded diagnostic observations only. No counter authorizes cleanup, refund
// or retry. Snapshots are not transactional and include boot fault fixtures.
use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use super::*;
use crate::memory::{
    AddressSpaceInterface,
    PHYSICAL_FRAME_ALLOCATOR,
    PreparingUserFrame,
    VAddr,
    allocators::stack_allocator::{
        self,
        AllocationFailure,
        Error,
    },
    linear::{
        MemoryMapping,
        PageType,
    },
};
static RETIREMENT_PROGRESS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
const STARTED: usize = 0;
const USER_RELEASED: usize = 1;
const IDENTITY_REJECTED: usize = 2;
const DETACH_REJECTED: usize = 3;
const INVALIDATION_REJECTED: usize = 4;
const PHYSICAL_REJECTED: usize = 5;

pub(super) fn retirement_progress() -> [u64; 6] {
    core::array::from_fn(|index| RETIREMENT_PROGRESS[index].load(Ordering::Relaxed))
}

const PAGE: usize = crate::cpu::isa::memory::paging::PAGE_SIZE;
const _: () = assert!(charlotte_launch::INITIAL_USER_STACK_PAGES == 1 && PAGE == 4096);
pub(super) mod tests;

#[derive(Debug)]
pub(crate) struct UserStack {
    slot: StackSlot,
    budget_pages: usize,
    committed_pages: usize,
}
impl UserStack {
    pub(crate) fn base_addr(&self) -> usize {
        self.slot.base()
    }

    pub(crate) fn top(&self) -> usize {
        self.base_addr() + self.budget_pages * PAGE
    }

    fn committed_low(&self) -> usize {
        self.top() - self.committed_pages * PAGE
    }
}

#[derive(Debug, Default)]
pub(crate) struct Stacks {
    kernel: Option<VAddr>,
    user: Option<UserStack>,
    // Kernel-only threads own node admission here. A user thread's StackSlot
    // owns the complete maximum user+kernel reservation and exact root lease.
    kernel_charge: Option<budget::Reservation>,
    kernel_uncertain: bool,
    // Any started retirement is terminal on rejection/interruption.
    release_started: bool,
}

impl Stacks {
    pub(crate) fn kernel() -> Result<Self, Error> {
        let charge =
            budget::Reservation::reserve(KERNEL_STACK_PAGES, true).ok_or(Error::InvalidStack)?;
        let mut stacks = Self {
            kernel_charge: Some(charge),
            ..Self::default()
        };
        if let Err(error) = stacks.allocate_kernel_with(stack_allocator::allocate_stack) {
            let _ = stacks.release();
            return Err(error);
        }
        Ok(stacks)
    }

    pub(crate) fn user(handle: AddressSpaceHandle, pages: usize) -> Result<Self, Error> {
        if !(1..=charlotte_launch::MAX_USER_STACK_PAGES).contains(&pages) {
            return Err(Error::InvalidStack);
        }
        let preparation = PreparingStackPage::reserve(handle).map_err(|_| Error::InvalidStack)?;
        let low = preparation.base() + (pages - 1) * PAGE;
        let slot = preparation.map(low).map_err(|_| Error::InvalidStack)?;
        let mut stacks = Self {
            user: Some(UserStack {
                slot,
                budget_pages: pages,
                committed_pages: 1,
            }),
            ..Self::default()
        };
        if let Err(error) = stacks.allocate_kernel_with(stack_allocator::allocate_stack) {
            let _ = stacks.release();
            return Err(error);
        }
        Ok(stacks)
    }

    fn allocate_kernel_with(
        &mut self,
        allocate: impl FnOnce(usize) -> Result<VAddr, AllocationFailure>,
    ) -> Result<(), Error> {
        self.kernel_uncertain = true;
        match allocate(KERNEL_STACK_PAGES) {
            Ok(base) => {
                self.kernel = Some(base);
                self.kernel_uncertain = false;
                Ok(())
            }
            Err(failure) => {
                self.kernel_uncertain = failure.retained;
                Err(failure.error)
            }
        }
    }

    pub(crate) fn kernel_base(&self) -> VAddr {
        self.kernel.unwrap_or_default()
    }

    pub(crate) fn user_stack(&self) -> Option<&UserStack> {
        self.user.as_ref()
    }

    pub(crate) fn committed_pages(&self) -> usize {
        self.user.as_ref().map_or(0, |stack| stack.committed_pages)
    }

    pub(crate) fn usage(&self, low_water: usize) -> (usize, usize) {
        self.user.as_ref().map_or((0, 0), |stack| {
            let low = low_water.clamp(stack.base_addr(), stack.top());
            (stack.budget_pages, (stack.top() - low).div_ceil(PAGE).min(stack.committed_pages))
        })
    }

    pub(crate) fn begin_growth(&mut self, address: usize) -> Option<Growth> {
        if self.release_started {
            return None;
        }
        let stack = self.user.as_mut()?;
        let page = address & !(PAGE - 1);
        if page >= stack.committed_low() || page < stack.base_addr() || stack.slot.uncertain {
            return None;
        }
        Some(Growth::new(stack, page))
    }

    /// The containing execution owner prevents retirement/migration through
    /// this exact handback. Failure returns the entire preparation unchanged.
    #[allow(clippy::result_large_err)]
    pub(crate) fn finish_growth(&mut self, growth: Growth) -> Result<Option<usize>, Growth> {
        let Some(stack) = self.user.as_mut() else {
            return Err(growth);
        };
        if self.release_started
            || !growth.completed
            || growth.mapping_started
            || growth.frame.is_some()
            || !stack.slot.uncertain
            || stack.slot.identity() != growth.handle
            || stack.base_addr() != growth.base
            || stack.budget_pages != growth.budget_pages
            || stack.committed_pages != growth.initial_pages
        {
            return Err(growth);
        }
        let low = growth.result();
        stack.committed_pages = growth.committed_pages;
        stack.slot.uncertain = growth.uncertain;
        Ok(low)
    }

    // Serialized fixtures own this entire never-scheduled stack directly.
    // Production uses the exact executing-thread owner around these phases.
    pub(in crate::memory::thread_stack) fn grow_user_stack(
        &mut self,
        address: usize,
    ) -> Option<usize> {
        let mut growth = self.begin_growth(address)?;
        growth.run();
        self.finish_growth(growth).unwrap_or_else(|_| panic!("stack growth handback rejected"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetirementError {
    RunQueueLinked,
    AlreadyStarted,
    User,
    Kernel(KernelFailure),
    Admission,
}

/// Diagnostic classification only; none of these values authorizes retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KernelFailure {
    InvalidStack,
    Detach,
    Physical,
    Unconfirmed,
}
impl KernelFailure {
    fn from_error(error: &Error) -> Self {
        match error {
            Error::InvalidStack => Self::InvalidStack,
            Error::AllocatorsMemory(crate::memory::allocators::memory::Error::RetirementFailed) => {
                Self::Unconfirmed
            }
            Error::AllocatorsMemory(crate::memory::allocators::memory::Error::PfaError(_)) => {
                Self::Physical
            }
            Error::IsaMemoryIfce(_) | Error::AllocatorsMemory(_) => Self::Detach,
        }
    }
}

impl Stacks {
    pub(crate) fn reject_release_for_test(&mut self) -> Result<(), RetirementError> {
        self.release_with(retire_user, |_, _| Err(Error::InvalidStack))
    }

    pub(crate) fn retirement_started(&self) -> bool {
        self.release_started
    }

    /// The caller proves this context is off-CPU and releases enclosing guards.
    /// Failure retains this pair's admission; it never authorizes a second attempt.
    pub(crate) fn release(&mut self) -> Result<(), RetirementError> {
        self.release_with(retire_user, stack_allocator::deallocate_stack)
    }

    fn release_with(
        &mut self,
        release_user: impl FnOnce(&mut UserStack) -> bool,
        release_kernel: impl FnOnce(VAddr, usize) -> Result<(), Error>,
    ) -> Result<(), RetirementError> {
        if self.release_started {
            return Err(RetirementError::AlreadyStarted);
        }
        // Arm before callbacks/detachment: interruption cannot retry freed pages.
        self.release_started = true;
        let user_ok = self.user.as_mut().is_none_or(release_user);
        let mut kernel_failure = KernelFailure::Unconfirmed;
        let kernel_ok = if let Some(base) = self.kernel {
            self.kernel_uncertain = true;
            match release_kernel(base, KERNEL_STACK_PAGES) {
                Ok(()) => {
                    self.kernel = None;
                    self.kernel_uncertain = false;
                    true
                }
                Err(error) => {
                    kernel_failure = KernelFailure::from_error(&error);
                    false
                }
            }
        } else {
            !self.kernel_uncertain
        };
        if !user_ok {
            return Err(RetirementError::User);
        }
        if !kernel_ok {
            return Err(RetirementError::Kernel(kernel_failure));
        }
        if let Some(user) = self.user.as_mut() {
            user.slot.released().map_err(|_| RetirementError::Admission)?;
        }
        if let Some(charge) = self.kernel_charge.take() {
            charge.refund();
        }
        Ok(())
    }
}

// No physical destructor. Implicit slot/charge destruction retains their
// original admission, and scalar mapped ranges are never re-adopted by address.

/// Bounded re-confirmation attempts for user-translation invalidation before
/// any backing is physically released. A transient rendezvous miss (busy or
/// timeout under load) must not permanently retain the stack pair: no frame
/// has been released yet, and each attempt is a fresh rendezvous that cannot
/// count stale acknowledgements.
const USER_INVALIDATION_ATTEMPTS: usize = 3;

fn retire_user(stack: &mut UserStack) -> bool {
    retire_user_with(
        stack,
        |base, pages, handle| {
            crate::cpu::isa::memory::tlb::try_inval_range_user(handle.id(), base, pages).is_ok()
        },
        |frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame),
    )
}

fn retire_user_with(
    stack: &mut UserStack,
    mut invalidate: impl FnMut(VAddr, usize, AddressSpaceHandle) -> bool,
    mut release: impl FnMut(crate::memory::PAddr) -> Result<(), crate::memory::physical::Error>,
) -> bool {
    RETIREMENT_PROGRESS[STARTED].fetch_add(1, Ordering::Relaxed);
    let handle = stack.slot.identity();
    let low = stack.committed_low();
    let mut frames = [None; charlotte_launch::MAX_USER_STACK_PAGES];
    let mut ok = true;
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        if table.generation(handle.id()).ok() != Some(handle.generation()) {
            RETIREMENT_PROGRESS[IDENTITY_REJECTED].fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let Ok(space) = table.get_mut(handle.id()) else {
            RETIREMENT_PROGRESS[IDENTITY_REJECTED].fetch_add(1, Ordering::Relaxed);
            return false;
        };
        for (index, frame) in frames.iter_mut().enumerate().take(stack.committed_pages) {
            match space.unmap_page(VAddr::from(low + index * PAGE)) {
                Ok(address) => *frame = Some(address),
                Err(_) => ok = false,
            }
        }
    }
    if !ok {
        RETIREMENT_PROGRESS[DETACH_REJECTED].fetch_add(1, Ordering::Relaxed);
    }
    let mut invalidated = false;
    for _ in 0..USER_INVALIDATION_ATTEMPTS {
        if invalidate(VAddr::from(low), stack.committed_pages, handle) {
            invalidated = true;
            break;
        }
    }
    if !invalidated {
        RETIREMENT_PROGRESS[INVALIDATION_REJECTED].fetch_add(1, Ordering::Relaxed);
        return false;
    }
    // Each release adapter takes only the physical allocator, never a table
    // guard or root lookup. Consumption precedes invoking the callback.
    for frame in frames.into_iter().flatten() {
        if release(frame).is_err() {
            RETIREMENT_PROGRESS[PHYSICAL_REJECTED].fetch_add(1, Ordering::Relaxed);
            ok = false;
        }
    }
    if ok {
        RETIREMENT_PROGRESS[USER_RELEASED].fetch_add(1, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// One in-flight growth episode. The original stack remains in its containing
/// owner with its slot fenced; production additionally retains that exact
/// thread's execution fence. This is not standalone cleanup/retry authority.
/// Drop retains provisional backing and leaves both containing fences armed.
pub(crate) struct Growth {
    handle: AddressSpaceHandle,
    base: usize,
    budget_pages: usize,
    initial_pages: usize,
    committed_pages: usize,
    target: usize,
    frame: Option<PreparingUserFrame>,
    started: bool,
    completed: bool,
    mapping_started: bool,
    uncertain: bool,
}

impl Growth {
    fn new(stack: &mut UserStack, target: usize) -> Self {
        assert!(!stack.slot.uncertain);
        stack.slot.uncertain = true;
        Self {
            handle: stack.slot.identity(),
            base: stack.base_addr(),
            budget_pages: stack.budget_pages,
            initial_pages: stack.committed_pages,
            committed_pages: stack.committed_pages,
            target,
            frame: None,
            started: false,
            completed: false,
            mapping_started: false,
            uncertain: false,
        }
    }

    pub(crate) fn handle(&self) -> AddressSpaceHandle {
        self.handle
    }

    fn low(&self) -> usize {
        self.base + (self.budget_pages - self.committed_pages) * PAGE
    }

    fn result(&self) -> Option<usize> {
        (self.committed_pages > self.initial_pages).then(|| self.low())
    }

    pub(crate) fn run(&mut self) {
        let handle = self.handle;
        self.run_with(
            || {
                crate::cpu::scheduler::threads::stack_growth_test_phase(handle, 1);
                PreparingUserFrame::allocate_with_policy(|free, total| {
                    free > (total / charlotte_lifecycle::STACK_GROWTH_RESERVE_DIVISOR).max(1)
                })
            },
            |handle, vaddr, frame| {
                crate::cpu::scheduler::threads::stack_growth_test_phase(handle, 2);
                let mut table = ADDRESS_SPACE_TABLE.lock();
                table.generation(handle.id()).ok() == Some(handle.generation())
                    && table.get_mut(handle.id()).is_ok_and(|space| {
                        space
                            .map_page(MemoryMapping {
                                vaddr: VAddr::from(vaddr),
                                paddr: frame,
                                page_type: PageType::UserData,
                            })
                            .is_ok()
                    })
            },
            |frame| {
                crate::cpu::scheduler::threads::stack_growth_test_phase(handle, 4);
                PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
            },
        );
    }

    fn run_with(
        &mut self,
        mut allocate: impl FnMut() -> Option<PreparingUserFrame>,
        mut map: impl FnMut(AddressSpaceHandle, usize, crate::memory::PAddr) -> bool,
        mut release: impl FnMut(crate::memory::PAddr) -> Result<(), crate::memory::physical::Error>,
    ) {
        assert!(!self.started && !self.completed && self.frame.is_none());
        self.started = true;
        let (free, total) = {
            let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            (allocator.free_frames() as u64, allocator.usable_bytes() / PAGE as u64)
        };
        let floor = (total / charlotte_lifecycle::STACK_GROWTH_RESERVE_DIVISOR).max(1);
        if free >= floor.saturating_add(((self.low() - self.target) / PAGE) as u64) {
            while self.low() > self.target {
                self.frame = allocate();
                let Some(frame) = self.frame.as_ref() else {
                    break;
                };
                frame.zero();
                let vaddr = self.low() - PAGE;
                // On interruption, no handback or physical rollback is allowed.
                self.mapping_started = true;
                if map(self.handle, vaddr, frame.frame()) {
                    self.committed_pages += 1;
                    self.frame.take().unwrap().quarantine();
                    self.mapping_started = false;
                } else {
                    // Walkers reject before leaf publication. Physical rollback
                    // follows their local table guard and the scheduler guard.
                    self.mapping_started = false;
                    let _ = self.rollback_with(&mut release);
                    break;
                }
            }
        }
        self.completed = true;
    }

    fn rollback_with(
        &mut self,
        release: impl FnOnce(crate::memory::PAddr) -> Result<(), crate::memory::physical::Error>,
    ) -> Result<(), crate::memory::physical::Error> {
        assert!(!self.mapping_started && !self.uncertain);
        self.uncertain = true;
        if let Some(frame) = self.frame.take() {
            frame.release_with(release)?;
        }
        self.uncertain = false;
        Ok(())
    }
}

impl Drop for Growth {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            frame.quarantine();
        }
        // No root lookup, allocator, pool refund, callbacks or logger. The
        // original stack slot stays fenced until a consuming proven handback.
    }
}
