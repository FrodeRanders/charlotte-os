//! # AArch64 Thread Context
//!
//! A thread's context is the minimal machine state required to suspend it and
//! later resume it as if nothing had happened. On AArch64 (as on x86-64) the
//! kernel performs cooperative, callee-saved-register context switches in
//! [`switch_ctx`](crate::cpu::isa::lp::ops::switch_ctx): the outgoing thread
//! pushes the callee-saved registers onto its own kernel stack, and the
//! incoming thread pops the same frame. Because of this, "creating" a thread
//! means synthesising an initial stack frame that looks exactly like one
//! `switch_ctx` would have produced, so that the very first switch into the
//! thread lands on a trampoline with the right registers loaded.
//!
//! This design is what makes threads cheap enough to spawn freely, which is a
//! cornerstone of Catten's async-first model: blocking is expressed by parking
//! a thread on an observable event, and completion is delivered by waking it,
//! rather than by heavyweight thread-pool machinery.

use core::sync::atomic::{
    AtomicU8,
    AtomicUsize,
    Ordering,
};

use crate::{
    cpu::isa::{
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
    memory::{
        ADDRESS_SPACE_TABLE,
        AddressSpaceId,
        PHYSICAL_FRAME_ALLOCATOR,
        VAddr,
        allocators::stack_allocator::{
            Error,
            allocate_stack,
            deallocate_stack,
        },
        linear::PageType,
    },
};

const INIT_KERNEL_STACK_PAGES: usize = 16;

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

/// The initial kernel-stack frame consumed by `switch_ctx`'s restore path when
/// a freshly created thread is first scheduled.
///
/// The field order matches the pop order in `switch_ctx` from the current stack
/// pointer upwards: the callee-saved register pairs x19/x20 through x29/x30,
/// followed by q8/q9 through q14/q15 and FPCR/FPSR. `switch_ctx` reloads x30
/// and eventually executes `ret`, so placing a trampoline address in `x30`
/// makes execution begin there after the SIMD/FP slots have also been
/// consumed. TTBR0 is *not* part of the frame: `switch_ctx` reloads it from
/// the incoming address space's software record (see
/// [`incoming_ttbr0`](crate::cpu::isa::aarch64::lp::ops::incoming_ttbr0)),
/// never from a `mrs ttbr0_el1` readback, because hypervisors such as HVF do
/// not preserve the hardware ASID bits on read.
#[repr(C)]
struct InitialFrame {
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x23: u64,
    x24: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    x28: u64,
    x29: u64,
    x30: u64,
    // AAPCS64 requires the low 64 bits of v8-v15 to survive a call. The
    // context switch preserves the complete 128-bit registers for a simpler,
    // stronger thread-context contract. These zeroes are consumed only on a
    // thread's first dispatch; later frames contain the saved live values.
    q8: [u64; 2],
    q9: [u64; 2],
    q10: [u64; 2],
    q11: [u64; 2],
    q12: [u64; 2],
    q13: [u64; 2],
    q14: [u64; 2],
    q15: [u64; 2],
    fpcr: u64,
    fpsr: u64,
}

impl InitialFrame {
    fn push_to_stack(self, sp: &mut VAddr) {
        let new_sp = *sp - core::mem::size_of::<InitialFrame>();
        unsafe {
            new_sp.into_mut::<InitialFrame>().write(self);
        }
        *sp = new_sp;
    }
}

#[derive(Debug, Default)]
pub struct ThreadContext {
    /// The saved kernel stack pointer at which this thread's `switch_ctx` frame
    /// resides. `cond_yield_lp` reads and writes this field through a raw
    /// pointer during a context switch.
    pub saved_sp: u64,
    /// Ownership flag for the SMP context-switch handshake. Nonzero while the
    /// thread is owned by *some* logical processor — i.e. from the moment it is
    /// selected to run until `switch_ctx` has finished saving its context on the
    /// way out. `switch_ctx` release-clears it after the outgoing save and
    /// acquire-waits for it to be zero before restoring an incoming thread, so a
    /// thread woken onto another LP can never be resumed with a stale `saved_sp`
    /// before the LP that last ran it has finished saving (the wake-before-save
    /// race). `switch_ctx` accesses this with byte-sized acquire/release and
    /// exclusive operations.
    pub on_cpu: AtomicU8,
    _kernel_stack_buf: VAddr,
    _user_stack: Option<UserStack>,
    /// Lowest user stack pointer observed for this thread. Sampling happens
    /// from the context-switch path, so it must remain a plain relaxed atomic.
    user_stack_low_water: AtomicUsize,
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        if let Some(user_stack) = self._user_stack.take()
            && !deallocate_user_stack(user_stack)
        {
            crate::early_logln!("WARNING: failed to free user stack on thread teardown");
        }
        if deallocate_stack(self._kernel_stack_buf, INIT_KERNEL_STACK_PAGES).is_err() {
            crate::early_logln!("WARNING: failed to free kernel stack on thread teardown");
        }
    }
}

impl ThreadContext {
    /// Whether an LP still owns this context. Assembly accesses `on_cpu` with
    /// byte-sized acquire/release operations; use matching atomic semantics.
    pub(crate) fn is_on_cpu(&self) -> bool {
        self.on_cpu.load(Ordering::Acquire) != 0
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

    /// Fold one observed user stack pointer into the thread's high-water
    /// estimate. Kernel threads have no user stack and ignore the sample. The
    /// bounds check also rejects a stale pointer from a previous occupant of
    /// this LP, whose stack lies in a different VA stride.
    pub(crate) fn sample_user_stack_pointer(&self, sp: usize) {
        let Some(stack) = self._user_stack.as_ref() else {
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
        self._user_stack.as_ref().map_or(0, |stack| stack.committed_pages)
    }

    /// Reserved (budget) and touched pages of this thread's user stack.
    pub(crate) fn user_stack_usage(&self) -> (usize, usize) {
        let Some(stack) = self._user_stack.as_ref() else {
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
        let stack = self._user_stack.as_mut()?;
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

    /// Create the context for a kernel thread that begins executing at
    /// `entry_point` at EL1 on its own kernel stack.
    pub fn create_kernel_thread_context(entry_point: extern "C" fn()) -> Result<Self, Error> {
        let kernel_stack_buf = allocate_stack(INIT_KERNEL_STACK_PAGES)?;
        let mut kernel_stack_top = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        // The current (kernel) address space's TTBR0 is what a kernel thread
        // runs with; higher-half kernel mappings live in TTBR1 and are shared.
        // TTBR0 itself is not stored here — `switch_ctx` reloads it from the
        // incoming address space's software record.
        let frame = InitialFrame {
            // kernel_thread_trampoline calls the entry point held in x19.
            x19: entry_point as usize as u64,
            x20: 0,
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            x29: 0,
            x30: kernel_thread_trampoline as *const () as usize as u64,
            q8: [0; 2],
            q9: [0; 2],
            q10: [0; 2],
            q11: [0; 2],
            q12: [0; 2],
            q13: [0; 2],
            q14: [0; 2],
            q15: [0; 2],
            fpcr: 0,
            fpsr: 0,
        };
        frame.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            saved_sp: <VAddr as Into<u64>>::into(kernel_stack_top),
            on_cpu: AtomicU8::new(0),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack: None,
            user_stack_low_water: AtomicUsize::new(0),
        })
    }

    /// Create the context for a user thread that begins executing at
    /// `entry_point` at EL0 in the address space identified by `asid`, using a
    /// dedicated kernel stack for the in-kernel trampoline and a separate user
    /// stack for EL0 execution.
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
            .map_err(|_| Error::InvalidStack)?;
        let stack_base = preparation.base();
        let user_stack_top_va = stack_base + user_stack_pages * PAGE_SIZE;
        const _: () = assert!(charlotte_launch::INITIAL_USER_STACK_PAGES == 1);
        let initial_pages = 1;
        let mapped_low = user_stack_top_va - PAGE_SIZE;
        let slot = preparation.map(mapped_low).map_err(|_| Error::InvalidStack)?;
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
                return Err(error);
            }
        };
        let mut kernel_stack_top = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        // Run the user thread in its own address space's lower half (TTBR0).
        // TTBR0 is not stored in the frame: `switch_ctx` reloads it from the
        // incoming address space's software record at switch time.
        let frame = InitialFrame {
            // user_trampoline loads x19 into ELR_EL1 and x20 into SP_EL0.
            x19: entry_point as usize as u64,
            x20: user_stack_top_va as u64,
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            x29: 0,
            x30: user_trampoline as *const () as usize as u64,
            q8: [0; 2],
            q9: [0; 2],
            q10: [0; 2],
            q11: [0; 2],
            q12: [0; 2],
            q13: [0; 2],
            q14: [0; 2],
            q15: [0; 2],
            fpcr: 0,
            fpsr: 0,
        };
        frame.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            saved_sp: <VAddr as Into<u64>>::into(kernel_stack_top),
            on_cpu: AtomicU8::new(0),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack: Some(user_stack),
            user_stack_low_water: AtomicUsize::new(user_stack_top_va),
        })
    }
}
