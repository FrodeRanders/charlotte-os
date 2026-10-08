use core::{
    mem::offset_of,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use crate::{
    cpu::isa::{
        init::gdt::{
            USER_CODE_SELECTOR,
            USER_DATA_SELECTOR,
        },
        interface::memory::address::VirtualAddress,
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
        VAddr,
        thread_stack::KERNEL_STACK_PAGES as INIT_KERNEL_STACK_PAGES,
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

/// Eagerly owned x87/MMX/SSE state. AVX is unavailable while OSXSAVE is
/// disabled; the software-float ABI does not grant access to unsaved hardware
/// state. A fresh image contains zero register payload and default controls.
#[repr(C, align(16))]
#[derive(Debug)]
pub(crate) struct FxState([u8; 512]);

impl Default for FxState {
    fn default() -> Self {
        let mut bytes = [0; 512];
        bytes[..2].copy_from_slice(&0x037fu16.to_le_bytes());
        bytes[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        Self(bytes)
    }
}

#[repr(C, align(16))]
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
    pub(crate) fs_base: u64,
    /// While kernel GS is active, this lives in IA32_KERNEL_GS_BASE.
    pub(crate) user_gs_base: u64,
    pub(crate) fp_state: FxState,
    _stacks: crate::memory::thread_stack::Stacks,
    /// Lowest ring-3 stack pointer observed for this thread. Sampling happens
    /// from the context-switch path, so it must remain a plain relaxed atomic.
    user_stack_low_water: AtomicUsize,
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
    pub(crate) fn release_stacks(
        &mut self,
    ) -> Result<(), crate::memory::thread_stack::RetirementError> {
        self._stacks.release()
    }

    pub(crate) fn reject_stack_release_for_test(
        &mut self,
    ) -> Result<(), crate::memory::thread_stack::RetirementError> {
        self._stacks.reject_release_for_test()
    }

    pub(crate) fn stack_retirement_started(&self) -> bool {
        self._stacks.retirement_started()
    }

    pub(crate) fn kernel_stack_bounds(&self) -> (usize, usize) {
        let base: usize = self._stacks.kernel_base().into();
        (base, base + INIT_KERNEL_STACK_PAGES * PAGE_SIZE)
    }

    /// Fold one observed ring-3 stack pointer into the thread's high-water
    /// estimate. Kernel threads have no user stack and ignore the sample. The
    /// bounds check also rejects a stale pointer from a previous occupant of
    /// this LP, whose stack lies in a different VA stride.
    pub(crate) fn sample_user_stack_pointer(&self, sp: usize) {
        let Some(stack) = self._stacks.user_stack() else {
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
        self._stacks.committed_pages()
    }

    /// Reserved (budget) and touched pages of this thread's user stack.
    pub(crate) fn user_stack_usage(&self) -> (usize, usize) {
        self._stacks.usage(self.user_stack_low_water.load(Ordering::Relaxed))
    }

    /// Extend this thread's user stack downward to cover `fault_addr`.
    ///
    /// Returns the new committed low address when at least the faulting page
    /// was mapped, or `None` outside the growable guard region and when free
    /// frames are below the pressure reserve. Mapping is one page at a time so
    /// a partial failure leaves an exact committed count; whatever was mapped
    /// is still released by the owning stack transaction when the fatal path
    /// retires the domain.
    pub(crate) fn grow_user_stack(&mut self, fault_addr: usize) -> Option<usize> {
        self._stacks.grow_user_stack(fault_addr)
    }

    pub fn create_user_thread_context(
        identity: crate::memory::AddressSpaceHandle,
        entry_point: extern "C" fn(),
        user_stack_pages: usize,
    ) -> Result<Self, Error> {
        let stacks = crate::memory::thread_stack::Stacks::user(identity, user_stack_pages)?;
        let kernel_stack_buf = stacks.kernel_base();
        let user_stack_top_va = stacks.user_stack().unwrap().top();
        let asid = identity.id();
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
            fs_base: 0,
            user_gs_base: 0,
            fp_state: FxState::default(),
            _stacks: stacks,
            user_stack_low_water: AtomicUsize::new(user_stack_top_va),
        })
    }

    pub fn create_kernel_thread_context(entry_point: extern "C" fn()) -> Result<Self, Error> {
        let stacks = crate::memory::thread_stack::Stacks::kernel()?;
        let kernel_stack_buf = stacks.kernel_base();
        let kernel_stack_top_va = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        let mut kernel_stack_top = kernel_stack_top_va;
        let ksf = KernelEntryFrame::new(KERNEL_AS.lock().get_cr3(), entry_point as usize as u64);
        ksf.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            rsp_cpl0: <VAddr as Into<u64>>::into(kernel_stack_top),
            kernel_stack_top: <VAddr as Into<u64>>::into(kernel_stack_top_va),
            fs_base: 0,
            user_gs_base: 0,
            fp_state: FxState::default(),
            _stacks: stacks,
            user_stack_low_water: AtomicUsize::new(0),
        })
    }
}

#[unsafe(no_mangle)]
pub static TC_RSP_CPL0_OFFSET: usize = offset_of!(ThreadContext, rsp_cpl0);

#[unsafe(no_mangle)]
pub static TC_KERNEL_STACK_TOP_OFFSET: usize = offset_of!(ThreadContext, kernel_stack_top);

// switch_ctx receives a pointer to rsp_cpl0. The auxiliary state is part of
// the same pinned context allocation and remains valid across the switch.
const _: () = assert!(offset_of!(ThreadContext, rsp_cpl0) == 0);
const _: () = assert!(offset_of!(ThreadContext, fs_base) == 16);
const _: () = assert!(offset_of!(ThreadContext, user_gs_base) == 24);
const _: () = assert!(offset_of!(ThreadContext, fp_state) == 32);
