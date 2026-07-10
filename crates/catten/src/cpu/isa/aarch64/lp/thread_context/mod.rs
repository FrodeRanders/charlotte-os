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

use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::cpu::isa::interface::memory::AddressSpaceInterface;
use crate::cpu::isa::lp::ops::{kernel_thread_trampoline, user_trampoline};
use crate::cpu::isa::memory::paging::{AddressSpace, PAGE_SIZE};
use crate::memory::allocators::stack_allocator::{allocate_stack, deallocate_stack, Error};
use crate::memory::{ADDRESS_SPACE_TABLE, AddressSpaceId, VAddr};

const INIT_KERNEL_STACK_PAGES: usize = 16;

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
    _kernel_stack_buf: VAddr,
    _user_stack_buf: Option<VAddr>,
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        if let Some(user_stack_buf) = self._user_stack_buf {
            deallocate_stack(user_stack_buf)
                .expect("Failed to deallocate user stack for thread context.");
        }
        deallocate_stack(self._kernel_stack_buf)
            .expect("Failed to deallocate kernel stack for thread context.");
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
            x30: kernel_thread_trampoline as usize as u64,
        };
        frame.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            saved_sp: <VAddr as Into<u64>>::into(kernel_stack_top),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack_buf: None,
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
        let user_stack_buf = allocate_stack(INIT_KERNEL_STACK_PAGES)?;
        let user_stack_top = user_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
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
            x20: <VAddr as Into<u64>>::into(user_stack_top),
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            x29: 0,
            x30: user_trampoline as usize as u64,
        };
        frame.push_to_stack(&mut kernel_stack_top);
        Ok(ThreadContext {
            saved_sp: <VAddr as Into<u64>>::into(kernel_stack_top),
            _kernel_stack_buf: kernel_stack_buf,
            _user_stack_buf: Some(user_stack_buf),
        })
    }
}
