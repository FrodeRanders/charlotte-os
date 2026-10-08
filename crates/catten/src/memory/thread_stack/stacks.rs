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
}

impl Stacks {
    pub(crate) fn kernel() -> Result<Self, Error> {
        let charge =
            budget::Reservation::reserve(KERNEL_STACK_PAGES, true).ok_or(Error::InvalidStack)?;
        let mut stacks = Self::default();
        stacks.kernel_charge = Some(charge);
        stacks.allocate_kernel_with(stack_allocator::allocate_stack)?;
        Ok(stacks)
    }

    pub(crate) fn user(handle: AddressSpaceHandle, pages: usize) -> Result<Self, Error> {
        if !(1..=charlotte_launch::MAX_USER_STACK_PAGES).contains(&pages) {
            return Err(Error::InvalidStack);
        }
        let preparation = PreparingStackPage::reserve(handle).map_err(|_| Error::InvalidStack)?;
        let low = preparation.base() + (pages - 1) * PAGE;
        let slot = preparation.map(low).map_err(|_| Error::InvalidStack)?;
        let mut stacks = Self::default();
        stacks.user = Some(UserStack {
            slot,
            budget_pages: pages,
            committed_pages: 1,
        });
        stacks.allocate_kernel_with(stack_allocator::allocate_stack)?;
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

    pub(crate) fn grow_user_stack(&mut self, address: usize) -> Option<usize> {
        let stack = self.user.as_mut()?;
        let page = address & !(PAGE - 1);
        let low = stack.committed_low();
        if page >= low || page < stack.base_addr() || stack.slot.uncertain {
            return None;
        }
        let (free, total) = {
            let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            (allocator.free_frames() as u64, allocator.usable_bytes() / PAGE as u64)
        };
        let floor = (total / charlotte_lifecycle::STACK_GROWTH_RESERVE_DIVISOR).max(1);
        if free < floor.saturating_add(((low - page) / PAGE) as u64) {
            return None;
        }
        let mut mapped_low = low;
        while mapped_low > page {
            let vaddr = mapped_low - PAGE;
            let Some(preparation) = PreparingGrowthPage::allocate(stack) else {
                break;
            };
            if !preparation.map(vaddr) {
                break;
            }
            mapped_low = vaddr;
        }
        (mapped_low < low).then_some(mapped_low)
    }
}

impl Stacks {
    fn release_with(&mut self, release_kernel: impl FnOnce(VAddr, usize) -> Result<(), Error>) {
        // Do not return the slot/root or node charge until *both* physical
        // ranges completed. A partial/uncertain kernel allocation also fences
        // refund even when no usable kernel base was returned.
        let user = self.user.take().map(retire_user);
        let kernel_ok = if let Some(base) = self.kernel.take() {
            self.kernel_uncertain = true;
            let released = release_kernel(base, KERNEL_STACK_PAGES).is_ok();
            self.kernel_uncertain = !released;
            released
        } else {
            !self.kernel_uncertain
        };
        if kernel_ok {
            if let Some(Some(slot)) = user {
                slot.released();
            }
            if let Some(charge) = self.kernel_charge.take() {
                charge.refund();
            }
        } else {
            crate::early_logln!("[stack admission] kernel cleanup uncertain; admission retained");
        }
        // Failed user cleanup drops only a published slot: its original root,
        // bit and entire reservation remain retained, even if kernel release
        // succeeded. Default node-reservation Drop never refunds.
    }
}

impl Drop for Stacks {
    fn drop(&mut self) {
        self.release_with(stack_allocator::deallocate_stack);
    }
}

fn retire_user(stack: UserStack) -> Option<StackSlot> {
    retire_user_with(
        stack,
        |base, pages, handle| {
            crate::cpu::isa::memory::tlb::try_inval_range_user(handle.id(), base, pages).is_ok()
        },
        |frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame),
    )
}

fn retire_user_with(
    stack: UserStack,
    invalidate: impl FnOnce(VAddr, usize, AddressSpaceHandle) -> bool,
    mut release: impl FnMut(crate::memory::PAddr) -> Result<(), crate::memory::physical::Error>,
) -> Option<StackSlot> {
    RETIREMENT_PROGRESS[STARTED].fetch_add(1, Ordering::Relaxed);
    let handle = stack.slot.identity();
    let low = stack.committed_low();
    let mut frames = [None; charlotte_launch::MAX_USER_STACK_PAGES];
    let mut ok = true;
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        if table.generation(handle.id()).ok() != Some(handle.generation()) {
            RETIREMENT_PROGRESS[IDENTITY_REJECTED].fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let Ok(space) = table.get_mut(handle.id()) else {
            RETIREMENT_PROGRESS[IDENTITY_REJECTED].fetch_add(1, Ordering::Relaxed);
            return None;
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
    if !invalidate(VAddr::from(low), stack.committed_pages, handle) {
        RETIREMENT_PROGRESS[INVALIDATION_REJECTED].fetch_add(1, Ordering::Relaxed);
        return None;
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
        Some(stack.slot)
    } else {
        None
    }
}

struct PreparingGrowthPage<'a> {
    stack: &'a mut UserStack,
    frame: Option<PreparingUserFrame>,
    mapping_started: bool,
}
impl<'a> PreparingGrowthPage<'a> {
    fn allocate(stack: &'a mut UserStack) -> Option<Self> {
        let mut preparation = Self {
            stack,
            frame: None,
            mapping_started: false,
        };
        preparation.frame = Some(PreparingUserFrame::allocate_with_policy(|free, total| {
            free > (total / charlotte_lifecycle::STACK_GROWTH_RESERVE_DIVISOR).max(1)
        })?);
        preparation.frame.as_ref().unwrap().zero();
        Some(preparation)
    }

    fn map(mut self, vaddr: usize) -> bool {
        let handle = self.stack.slot.identity();
        assert!(self.stack.slot.contains_page(vaddr));
        self.mapping_started = true;
        let mapped = {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            table.generation(handle.id()).ok() == Some(handle.generation())
                && table.get_mut(handle.id()).is_ok_and(|space| {
                    space
                        .map_page(MemoryMapping {
                            vaddr: VAddr::from(vaddr),
                            paddr: self.frame.as_ref().unwrap().frame(),
                            page_type: PageType::UserData,
                        })
                        .is_ok()
                })
        };
        if mapped {
            self.stack.committed_pages += 1;
            self.frame.take().unwrap().quarantine();
        }
        self.mapping_started = false;
        mapped
    }
}
impl Drop for PreparingGrowthPage<'_> {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            if self.mapping_started {
                self.stack.slot.uncertain = true;
                frame.quarantine();
            } else {
                self.stack.slot.uncertain = true;
                if frame
                    .release_with(|frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame))
                    .is_ok()
                {
                    self.stack.slot.uncertain = false;
                }
            }
        }
    }
}
