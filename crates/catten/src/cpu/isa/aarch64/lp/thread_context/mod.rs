//! # AArch64 Thread Context
//!
//! A thread's context is the minimal machine state required to suspend it and
//! later resume it as if nothing had happened. On AArch64 (as on x86-64) the
//! kernel performs cooperative, callee-saved-register context switches in
//! [`switch_ctx`](crate::cpu::isa::lp::ops::switch_ctx): the outgoing thread
//! pushes the callee-saved registers plus its user translation table base onto
//! its own kernel stack, and the incoming thread pops the same frame. Because
//! of this, "creating" a thread means synthesising an initial stack frame that
//! looks exactly like one `switch_ctx` would have produced, so that the very
//! first switch into the thread lands on a trampoline with the right registers
//! loaded.
//!
//! This design is what makes threads cheap enough to spawn freely, which is a
//! cornerstone of Catten's async-first model: blocking is expressed by parking
//! a thread on an observable event, and completion is delivered by waking it,
//! rather than by heavyweight thread-pool machinery.

use core::sync::atomic::AtomicUsize;

use crate::{
    cpu::isa::{
        interface::memory::{
            address::VirtualAddress,
            AddressSpaceInterface,
            MemoryMapping,
        },
        lp::ops::{
            kernel_thread_trampoline,
            user_trampoline,
        },
        memory::paging::{
            AddressSpace,
            PAGE_SIZE,
        },
    },
    memory::{
        allocators::stack_allocator::{
            allocate_stack,
            deallocate_stack,
            Error,
        },
        linear::PageType,
        AddressSpaceId,
        VAddr,
        ADDRESS_SPACE_TABLE,
        PHYSICAL_FRAME_ALLOCATOR,
    },
};

const INIT_KERNEL_STACK_PAGES: usize = 16;
const USER_STACK_PAGES: usize = 4;

#[derive(Debug, Clone, Copy)]
struct UserStack {
    asid: AddressSpaceId,
    base: VAddr,
}

fn deallocate_user_stack(stack: UserStack) -> bool {
    let mut ok = true;
    let mut as_table = ADDRESS_SPACE_TABLE.lock();
    let Ok(user_as) = as_table.get_mut(stack.asid) else {
        return false;
    };

    for page_idx in 0..USER_STACK_PAGES {
        let vaddr = stack.base + page_idx * PAGE_SIZE;
        match user_as.unmap_page(vaddr) {
            Ok(frame) => {
                if PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame).is_err() {
                    ok = false;
                }
            }
            Err(_) => ok = false,
        }
    }
    ok
}

/// The initial kernel-stack frame consumed by `switch_ctx`'s restore path when
/// a freshly created thread is first scheduled.
///
/// The field order matches the pop order in `switch_ctx` from the current stack
/// pointer upwards: first the saved `TTBR0_EL1` (stored as a 16-byte pair with
/// a zero pad to preserve stack alignment), then the callee-saved register
/// pairs x19/x20 through x29/x30. `switch_ctx` reloads x30 last and executes
/// `ret`, so placing a trampoline address in `x30` makes execution begin there.
#[repr(C)]
struct InitialFrame {
    ttbr0_el1: u64,
    _pad: u64,
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

#[derive(Debug, Clone, Default)]
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
    pub on_cpu: u8,
    _kernel_stack_buf: VAddr,
    _user_stack: Option<UserStack>,
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        if let Some(user_stack) = self._user_stack {
            if !deallocate_user_stack(user_stack) {
                crate::early_logln!("WARNING: failed to free user stack on thread teardown");
            }
        }
        if deallocate_stack(self._kernel_stack_buf, INIT_KERNEL_STACK_PAGES).is_err() {
            crate::early_logln!("WARNING: failed to free kernel stack on thread teardown");
        }
    }
}

impl ThreadContext {
    /// Create the context for a kernel thread that begins executing at
    /// `entry_point` at EL1 on its own kernel stack.
    pub fn create_kernel_thread_context(entry_point: extern "C" fn()) -> Result<Self, Error> {
        let kernel_stack_buf = allocate_stack(INIT_KERNEL_STACK_PAGES)?;
        let mut kernel_stack_top = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        // The current (kernel) address space's TTBR0 is what a kernel thread
        // runs with; higher-half kernel mappings live in TTBR1 and are shared.
        let ttbr0_el1 = AddressSpace::get_current().get_ttbr0();
        let frame = InitialFrame {
            ttbr0_el1,
            _pad: 0,
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
        };
        frame.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            saved_sp: <VAddr as Into<u64>>::into(kernel_stack_top),
            on_cpu: 0,
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack: None,
        })
    }

    /// Create the context for a user thread that begins executing at
    /// `entry_point` at EL0 in the address space identified by `asid`, using a
    /// dedicated kernel stack for the in-kernel trampoline and a separate user
    /// stack for EL0 execution.
    pub fn create_user_thread_context(
        asid: AddressSpaceId,
        entry_point: extern "C" fn(),
    ) -> Result<Self, Error> {
        // Allocate user stack pages from physical frames and map them into the
        // user address space.  The kernel stack allocator returns higher-half
        // VAs that have no TTBR0 mapping; EL0 can only use TTBR0.  Because
        // this prototype has no virtual-memory manager we place each user
        // thread's stack at a fixed VA region, offset by a per-thread index.
        const USER_STACK_VADDR_BASE: usize = 0x0000_0000_0100_0000;
        const USER_STACK_STRIDE: usize = USER_STACK_PAGES * PAGE_SIZE + PAGE_SIZE; // + guard
        static NEXT_STACK_INDEX: AtomicUsize = AtomicUsize::new(0);
        let stack_index = NEXT_STACK_INDEX.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let stack_base = USER_STACK_VADDR_BASE + stack_index * USER_STACK_STRIDE;
        let user_stack_top_va = stack_base + USER_STACK_PAGES * PAGE_SIZE;
        // Pre-allocate all frames first (under the frame allocator lock), then
        // map them (inside the AS table lock).  Order matches el0.rs.
        let stack_frames: [crate::memory::physical::PAddr; USER_STACK_PAGES] = {
            let mut pfa = PHYSICAL_FRAME_ALLOCATOR.lock();
            core::array::from_fn(|_| {
                pfa.allocate_frame().expect("Failed to allocate user stack frame")
            })
        };
        {
            let mut as_table = ADDRESS_SPACE_TABLE.lock();
            let user_as = as_table.get_mut(asid).expect("Address space not found in AS table");
            for i in 0..USER_STACK_PAGES {
                let vaddr = VAddr::from(stack_base + i * PAGE_SIZE);
                user_as
                    .map_page(MemoryMapping {
                        vaddr,
                        paddr: stack_frames[i],
                        page_type: PageType::UserData,
                    })
                    .expect("Failed to map user stack page");
            }
        }
        let user_stack = UserStack {
            asid,
            base: VAddr::from(stack_base),
        };
        let kernel_stack_buf = allocate_stack(INIT_KERNEL_STACK_PAGES)?;
        let mut kernel_stack_top = kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        // Run the user thread in its own address space's lower half (TTBR0).
        let ttbr0_el1 = ADDRESS_SPACE_TABLE
            .lock()
            .get(asid)
            .expect("Address space not found when creating thread context.")
            .get_ttbr0();
        let frame = InitialFrame {
            ttbr0_el1,
            _pad: 0,
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
        };
        frame.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            saved_sp: <VAddr as Into<u64>>::into(kernel_stack_top),
            on_cpu: 0,
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack: Some(user_stack),
        })
    }
}
