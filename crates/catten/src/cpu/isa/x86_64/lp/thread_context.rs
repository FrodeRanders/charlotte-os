use core::{
    mem::offset_of,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

const INIT_KERNEL_STACK_PAGES: usize = 16;

use crate::{
    cpu::isa::{
        init::gdt::{
            USER_CODE_SELECTOR,
            USER_DATA_SELECTOR,
        },
        interface::memory::{
            AddressSpaceInterface,
            MemoryMapping,
            address::VirtualAddress,
        },
        lp::ops::{
            kernel_thread_trampoline,
            user_trampoline,
        },
        memory::paging::PAGE_SIZE,
    },
    klib::collections::id_table,
    memory::{
        ADDRESS_SPACE_TABLE,
        AddressSpaceId,
        KERNEL_AS,
        PHYSICAL_FRAME_ALLOCATOR,
        VAddr,
        allocators::stack_allocator::{
            allocate_stack,
            deallocate_stack,
        },
        linear::PageType,
    },
};

/// The initial kernel-stack frame consumed by `switch_ctx`'s restore path when a
/// freshly created user thread is first scheduled, followed by the `iretq`
/// frame that `user_trampoline` pops to enter ring 3.
///
/// The field order matches `switch_ctx`'s restore order (see
/// [`crate::cpu::isa::x86_64::lp::ops::switch_ctx`]): the callee-saved
/// registers r15/r14/r13/r12/rbp/rbx are popped after CR3 and RFLAGS, and
/// `ret` then pops `rip` (here the [`user_trampoline`] address). The
/// trampoline executes `iretq`, which consumes the trailing five words
/// (RIP, CS, RFLAGS, RSP, SS) and drops to ring 3.
#[repr(C, align(16))]
struct UserEntryFrames {
    // switch_ctx yield/restore frame
    cr3: u64,
    rflags_cpl0: u64,
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbp: u64,
    rbx: u64,
    rip: u64,
    // iretq return frame
    user_rip: u64,
    cs: u64,
    user_rflags: u64,
    user_rsp: u64,
    ss: u64,
}

impl UserEntryFrames {
    fn new(asp: AddressSpaceId, entry_point: u64, iretq_rsp: VAddr, flags: u64) -> Self {
        let trampoline: unsafe extern "C" fn() -> ! = user_trampoline;
        UserEntryFrames {
            cr3: ADDRESS_SPACE_TABLE
                .lock()
                .get(asp)
                .expect("Address space not found when creating thread context.")
                .get_cr3(),
            rflags_cpl0: 0x2,
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            rip: trampoline as usize as u64,
            user_rip: entry_point,
            cs: USER_CODE_SELECTOR as u64,
            user_rflags: flags,
            user_rsp: <VAddr as Into<u64>>::into(iretq_rsp),
            ss: USER_DATA_SELECTOR as u64,
        }
    }

    fn push_to_stack(self, rsp: &mut VAddr) {
        let new_rsp = *rsp - core::mem::size_of::<UserEntryFrames>();
        unsafe {
            let isf_ptr = new_rsp.into_mut::<UserEntryFrames>();
            isf_ptr.write(self);
        }
        *rsp = new_rsp;
    }
}

#[repr(C, align(16))]
struct KernelEntryFrame {
    cr3: u64,
    rflags: u64,
    callee_saved_regs: [u64; 6],
    rip: u64,
}

impl KernelEntryFrame {
    fn new(cr3: u64, entry_point: u64) -> Self {
        let mut callee_saved_regs = [0; 6];
        callee_saved_regs[3] = entry_point;
        KernelEntryFrame {
            cr3,
            rflags: 0x2,
            callee_saved_regs,
            rip: kernel_thread_trampoline as *const () as u64,
        }
    }

    fn push_to_stack(self, rsp: &mut VAddr) {
        let new_rsp = *rsp - core::mem::size_of::<KernelEntryFrame>();
        unsafe {
            let kef_ptr = new_rsp.into_mut::<KernelEntryFrame>();
            kef_ptr.write(self);
        }
        *rsp = new_rsp;
    }
}

#[derive(Debug)]
struct UserStack {
    slot: crate::memory::thread_stack::StackSlot,
    asid: AddressSpaceId,
    /// Bottom of the thread's virtual stack region; also the growth floor.
    base: VAddr,
    /// Maximum committed pages (the signed or adaptive stack policy).
    budget_pages: usize,
    /// Pages mapped so far, counted from the top.
    committed_pages: usize,
}

impl UserStack {
    fn base_addr(&self) -> usize {
        self.base.into()
    }

    fn top(&self) -> usize {
        self.base_addr() + self.budget_pages * PAGE_SIZE
    }

    fn committed_low(&self) -> usize {
        self.top() - self.committed_pages * PAGE_SIZE
    }
}

fn deallocate_user_stack(stack: UserStack) -> bool {
    let mut ok = true;
    let mut frames = [None; charlotte_launch::MAX_USER_STACK_PAGES];
    let low = stack.committed_low();
    let handle = stack.slot.identity();
    {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        if table.generation(handle.id()).ok() != Some(handle.generation()) {
            return false;
        }
        let Ok(space) = table.get_mut(handle.id()) else {
            return false;
        };
        for (index, frame) in frames.iter_mut().enumerate().take(stack.committed_pages) {
            match space.unmap_page(VAddr::from(low + index * PAGE_SIZE)) {
                Ok(unmapped) => *frame = Some(unmapped),
                Err(_) => ok = false,
            }
        }
    }
    // The slot owns the original root through invalidation, outside its guard.
    crate::cpu::isa::memory::tlb::inval_range_user(
        handle.id(),
        VAddr::from(low),
        stack.committed_pages,
    );
    {
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for frame in frames.into_iter().flatten() {
            if allocator.deallocate_frame(frame).is_err() {
                ok = false;
            }
        }
    }
    if ok {
        stack.slot.released();
    }
    ok
}

#[derive(Debug)]
pub enum Error {
    AddressSpaceNotFound,
    StackAllocError(crate::memory::allocators::stack_allocator::Error),
    IdTableError(id_table::Error),
}

impl From<crate::memory::allocators::stack_allocator::Error> for Error {
    fn from(err: crate::memory::allocators::stack_allocator::Error) -> Self {
        Error::StackAllocError(err)
    }
}

impl From<id_table::Error> for Error {
    fn from(err: id_table::Error) -> Self {
        Error::IdTableError(err)
    }
}

#[derive(Debug, Default)]
pub struct ThreadContext {
    /// The saved kernel stack pointer at which this thread's `switch_ctx` frame
    /// resides. `cond_yield_lp` reads and writes this field through a raw
    /// pointer during a context switch.
    pub rsp_cpl0: u64,
    /// The top of this thread's dedicated kernel stack (the address loaded into
    /// `TSS.RSP0` so a ring-3 interrupt or syscall entry lands on the correct
    /// per-thread stack).
    pub kernel_stack_top: u64,
    _kernel_stack_buf: VAddr,
    _user_stack_buf: Option<UserStack>,
    /// Lowest ring-3 stack pointer observed for this thread. Sampling happens
    /// from the context-switch path, so it must remain a plain relaxed atomic.
    user_stack_low_water: AtomicUsize,
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        if let Some(user_stack_buf) = self._user_stack_buf.take()
            && !deallocate_user_stack(user_stack_buf)
        {
            crate::early_logln!("WARNING: failed to free user stack on thread teardown");
        }
        if deallocate_stack(self._kernel_stack_buf, INIT_KERNEL_STACK_PAGES).is_err() {
            crate::early_logln!("WARNING: failed to free kernel stack on thread teardown");
        }
    }
}

impl ThreadContext {
    /// x86_64 has no cross-LP ownership handshake yet. Runtime migration stays
    /// disabled there; current migration happens before contexts execute.
    pub(crate) fn is_on_cpu(&self) -> bool {
        false
    }

    /// Whether `address` lies in this context's mapped kernel-stack pages.
    pub(crate) fn kernel_stack_contains(&self, address: usize) -> bool {
        let (base, end) = self.kernel_stack_bounds();
        (base..end).contains(&address)
    }

    /// Bounds of the mapped kernel-stack pages, excluding both guard pages.
    pub(crate) fn kernel_stack_bounds(&self) -> (usize, usize) {
        let base: usize = self._kernel_stack_buf.into();
        (base, base + INIT_KERNEL_STACK_PAGES * PAGE_SIZE)
    }

    /// Fold one observed ring-3 stack pointer into the thread's high-water
    /// estimate. Kernel threads have no user stack and ignore the sample. The
    /// bounds check also rejects a stale pointer from a previous occupant of
    /// this LP, whose stack lies in a different VA stride.
    pub(crate) fn sample_user_stack_pointer(&self, sp: usize) {
        let Some(stack) = self._user_stack_buf.as_ref() else {
            return;
        };
        if (stack.base_addr()..stack.top()).contains(&sp) {
            self.user_stack_low_water.fetch_min(sp, Ordering::Relaxed);
        }
    }

    /// Pages currently committed to this thread's user stack.
    ///
    /// The gap between this and the budget reported by
    /// [`Self::user_stack_usage`] is the growth headroom the demand-grown
    /// stack protocol can still charge.
    pub(crate) fn user_stack_committed_pages(&self) -> usize {
        self._user_stack_buf.as_ref().map_or(0, |stack| stack.committed_pages)
    }

    /// Reserved (budget) and touched pages of this thread's user stack.
    pub(crate) fn user_stack_usage(&self) -> (usize, usize) {
        let Some(stack) = self._user_stack_buf.as_ref() else {
            return (0, 0);
        };
        let low_water =
            self.user_stack_low_water.load(Ordering::Relaxed).clamp(stack.base_addr(), stack.top());
        let used = (stack.top() - low_water).div_ceil(PAGE_SIZE).min(stack.committed_pages);
        (stack.budget_pages, used)
    }

    /// Extend this thread's user stack downward to cover `fault_addr`.
    ///
    /// Returns the new committed low address when at least the faulting page
    /// was mapped, or `None` outside the growable guard region and when free
    /// frames are below the pressure reserve. Mapping is one page at a time so
    /// a partial failure leaves an exact committed count; whatever was mapped
    /// is still released by `deallocate_user_stack` when the fatal path
    /// retires the domain.
    pub(crate) fn grow_user_stack(&mut self, fault_addr: usize) -> Option<usize> {
        let stack = self._user_stack_buf.as_mut()?;
        let page = fault_addr & !(PAGE_SIZE - 1);
        let low = stack.committed_low();
        let base = stack.base_addr();
        if page >= low || page < base {
            return None;
        }
        let (free_frames, total_frames) = {
            let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            (allocator.free_frames() as u64, allocator.usable_bytes() / PAGE_SIZE as u64)
        };
        let reserve_frames =
            (total_frames / charlotte_lifecycle::STACK_GROWTH_RESERVE_DIVISOR).max(1);
        let required_frames = ((low - page) / PAGE_SIZE) as u64;
        if free_frames < reserve_frames.saturating_add(required_frames) {
            return None;
        }
        let asid = stack.asid;
        let mut mapped_low = low;
        let mut grew = false;
        while mapped_low > page {
            let vaddr = mapped_low - PAGE_SIZE;
            let frame = match PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame() {
                Ok(frame) => frame,
                Err(_) => break,
            };
            let page_ptr: *mut u8 = frame.into();
            unsafe {
                core::ptr::write_bytes(page_ptr, 0, PAGE_SIZE);
            }
            let mapped = {
                let mut as_table = ADDRESS_SPACE_TABLE.lock();
                as_table.get_mut(asid).is_ok_and(|user_as| {
                    user_as
                        .map_page(MemoryMapping {
                            vaddr: VAddr::from(vaddr),
                            paddr: frame,
                            page_type: PageType::UserData,
                        })
                        .is_ok()
                })
            };
            if !mapped {
                let _ = PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame);
                break;
            }
            stack.committed_pages += 1;
            mapped_low = vaddr;
            grew = true;
        }
        grew.then_some(mapped_low)
    }

    pub fn create_user_thread_context(
        identity: crate::memory::AddressSpaceHandle,
        entry_point: extern "C" fn(),
        user_stack_pages: usize,
    ) -> Result<Self, Error> {
        assert!(
            (1..=charlotte_launch::MAX_USER_STACK_PAGES).contains(&user_stack_pages),
            "invalid userspace stack limit"
        );
        let asid = identity.id();
        let preparation = crate::memory::thread_stack::PreparingStackPage::reserve(identity)
            .map_err(|_| {
                Error::StackAllocError(
                    crate::memory::allocators::stack_allocator::Error::InvalidStack,
                )
            })?;
        let stack_base = preparation.base();
        let user_stack_top_va = stack_base + user_stack_pages * PAGE_SIZE;
        const _: () = assert!(charlotte_launch::INITIAL_USER_STACK_PAGES == 1);
        let initial_pages = 1;
        let mapped_low = user_stack_top_va - PAGE_SIZE;
        let slot = preparation.map(mapped_low).map_err(|_| {
            Error::StackAllocError(crate::memory::allocators::stack_allocator::Error::InvalidStack)
        })?;
        let user_stack = UserStack {
            slot,
            asid,
            base: VAddr::from(stack_base),
            budget_pages: user_stack_pages,
            committed_pages: initial_pages,
        };

        let kernel_stack_buf = match allocate_stack(INIT_KERNEL_STACK_PAGES) {
            Ok(stack) => stack,
            Err(error) => {
                let _ = deallocate_user_stack(user_stack);
                return Err(error.into());
            }
        };
        let kernel_stack_top_va = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        let mut kernel_stack_top = kernel_stack_top_va;
        let isf = UserEntryFrames::new(
            asid,
            entry_point as usize as u64,
            VAddr::from(user_stack_top_va),
            0x202,
        );
        isf.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            rsp_cpl0: <VAddr as Into<u64>>::into(kernel_stack_top),
            kernel_stack_top: <VAddr as Into<u64>>::into(kernel_stack_top_va),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack_buf: Some(user_stack),
            user_stack_low_water: AtomicUsize::new(user_stack_top_va),
        })
    }

    pub fn create_kernel_thread_context(entry_point: extern "C" fn()) -> Result<Self, Error> {
        let kernel_stack_buf = allocate_stack(INIT_KERNEL_STACK_PAGES)?;
        let kernel_stack_top_va = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        let mut kernel_stack_top = kernel_stack_top_va;
        let ksf = KernelEntryFrame::new(KERNEL_AS.lock().get_cr3(), entry_point as usize as u64);
        ksf.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            rsp_cpl0: <VAddr as Into<u64>>::into(kernel_stack_top),
            kernel_stack_top: <VAddr as Into<u64>>::into(kernel_stack_top_va),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack_buf: None,
            user_stack_low_water: AtomicUsize::new(0),
        })
    }
}

#[unsafe(no_mangle)]
pub static TC_RSP_CPL0_OFFSET: usize = offset_of!(ThreadContext, rsp_cpl0);

#[unsafe(no_mangle)]
pub static TC_KERNEL_STACK_TOP_OFFSET: usize = offset_of!(ThreadContext, kernel_stack_top);
