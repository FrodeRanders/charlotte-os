use core::{
    mem::{
        offset_of,
        transmute,
    },
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

const INIT_KERNEL_STACK_PAGES: usize = 16;
const USER_STACK_PAGES: usize = 4;

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
            rip: unsafe {
                transmute::<*const unsafe extern "C" fn() -> !, u64>(
                    user_trampoline as *const unsafe extern "C" fn() -> !,
                )
            },
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

#[derive(Debug, Clone, Copy)]
struct UserStack {
    asid: AddressSpaceId,
    base: VAddr,
}

fn deallocate_user_stack(stack: UserStack) -> bool {
    let mut ok = true;
    let mut frames = [None; USER_STACK_PAGES];
    {
        let mut as_table = ADDRESS_SPACE_TABLE.lock();
        let Ok(user_as) = as_table.get_mut(stack.asid) else {
            return false;
        };
        for (page_idx, frame) in frames.iter_mut().enumerate() {
            let vaddr = stack.base + page_idx * PAGE_SIZE;
            match user_as.unmap_page(vaddr) {
                Ok(unmapped) => *frame = Some(unmapped),
                Err(_) => ok = false,
            }
        }
    }
    crate::cpu::isa::memory::tlb::inval_range_user(stack.asid, stack.base, USER_STACK_PAGES);
    let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
    for frame in frames.into_iter().flatten() {
        if allocator.deallocate_frame(frame).is_err() {
            ok = false;
        }
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

#[derive(Debug, Clone, Default)]
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
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        if let Some(user_stack_buf) = self._user_stack_buf
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
        let base: usize = self._kernel_stack_buf.into();
        (base..base + INIT_KERNEL_STACK_PAGES * PAGE_SIZE).contains(&address)
    }

    pub fn create_user_thread_context(
        asid: AddressSpaceId,
        entry_point: extern "C" fn(),
    ) -> Result<Self, Error> {
        // Allocate and map a dedicated EL0 stack into the user address space.
        // The kernel stack allocator returns higher-half VAs that are not
        // accessible from ring 3, so — mirroring the AArch64 port — physical
        // frames are mapped into the user address space at a fixed per-thread
        // region.
        const USER_STACK_VADDR_BASE: usize = 0x0000_0000_0100_0000;
        const USER_STACK_STRIDE: usize = USER_STACK_PAGES * PAGE_SIZE + PAGE_SIZE; // + guard
        static NEXT_STACK_INDEX: AtomicUsize = AtomicUsize::new(0);
        let stack_index = NEXT_STACK_INDEX.fetch_add(1, Ordering::Relaxed);
        let stack_base = USER_STACK_VADDR_BASE + stack_index * USER_STACK_STRIDE;
        let user_stack_top_va = stack_base + USER_STACK_PAGES * PAGE_SIZE;

        // Pre-allocate all frames first, then map them.
        let mut stack_frames: [Option<crate::memory::physical::PAddr>; USER_STACK_PAGES] =
            [None; USER_STACK_PAGES];
        {
            let mut pfa = PHYSICAL_FRAME_ALLOCATOR.lock();
            for index in 0..USER_STACK_PAGES {
                match pfa.allocate_frame() {
                    Ok(allocated) => stack_frames[index] = Some(allocated),
                    Err(_) => {
                        for allocated in stack_frames.iter_mut().filter_map(Option::take) {
                            let _ = pfa.deallocate_frame(allocated);
                        }
                        return Err(Error::StackAllocError(
                            crate::memory::allocators::stack_allocator::Error::InvalidStack,
                        ));
                    }
                }
            }
        }

        let (map_result, mapped_pages) = {
            let mut as_table = ADDRESS_SPACE_TABLE.lock();
            match as_table.get_mut(asid) {
                Ok(user_as) => {
                    let mut result = Ok(());
                    let mut mapped_pages = 0;
                    for (index, frame) in stack_frames.iter_mut().enumerate() {
                        let vaddr = VAddr::from(stack_base + index * PAGE_SIZE);
                        let allocated = (*frame).expect("preallocated user-stack frame missing");
                        if user_as
                            .map_page(MemoryMapping {
                                vaddr,
                                paddr: allocated,
                                page_type: PageType::UserData,
                            })
                            .is_err()
                        {
                            result = Err(Error::StackAllocError(
                                crate::memory::allocators::stack_allocator::Error::InvalidStack,
                            ));
                            break;
                        }
                        *frame = None;
                        mapped_pages += 1;
                    }
                    (result, mapped_pages)
                }
                Err(_) => (Err(Error::AddressSpaceNotFound), 0),
            }
        };
        if let Err(error) = map_result {
            let mut mapped_frames = [None; USER_STACK_PAGES];
            if mapped_pages != 0 {
                let mut as_table = ADDRESS_SPACE_TABLE.lock();
                if let Ok(user_as) = as_table.get_mut(asid) {
                    for (index, frame) in mapped_frames.iter_mut().enumerate().take(mapped_pages) {
                        *frame =
                            user_as.unmap_page(VAddr::from(stack_base + index * PAGE_SIZE)).ok();
                    }
                }
            }
            crate::cpu::isa::memory::tlb::inval_range_user(
                asid,
                VAddr::from(stack_base),
                mapped_pages,
            );
            let mut pfa = PHYSICAL_FRAME_ALLOCATOR.lock();
            for frame in mapped_frames.into_iter().chain(stack_frames.into_iter()).flatten() {
                let _ = pfa.deallocate_frame(frame);
            }
            return Err(error);
        }

        let user_stack = UserStack {
            asid,
            base: VAddr::from(stack_base),
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
        let isf =
            UserEntryFrames::new(asid, entry_point as u64, VAddr::from(user_stack_top_va), 0x202);
        isf.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            rsp_cpl0: <VAddr as Into<u64>>::into(kernel_stack_top),
            kernel_stack_top: <VAddr as Into<u64>>::into(kernel_stack_top_va),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack_buf: Some(user_stack),
        })
    }

    pub fn create_kernel_thread_context(entry_point: extern "C" fn()) -> Result<Self, Error> {
        let kernel_stack_buf = allocate_stack(INIT_KERNEL_STACK_PAGES)?;
        let kernel_stack_top_va = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        let mut kernel_stack_top = kernel_stack_top_va;
        let ksf = KernelEntryFrame::new(KERNEL_AS.lock().get_cr3(), entry_point as u64);
        ksf.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            rsp_cpl0: <VAddr as Into<u64>>::into(kernel_stack_top),
            kernel_stack_top: <VAddr as Into<u64>>::into(kernel_stack_top_va),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack_buf: None,
        })
    }
}

#[unsafe(no_mangle)]
pub static TC_RSP_CPL0_OFFSET: usize = offset_of!(ThreadContext, rsp_cpl0);

#[unsafe(no_mangle)]
pub static TC_KERNEL_STACK_TOP_OFFSET: usize = offset_of!(ThreadContext, kernel_stack_top);
