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

#[derive(Debug, Clone, Copy)]
struct UserStack {
    asid: AddressSpaceId,
    base: VAddr,
    pages: usize,
}

fn deallocate_user_stack(stack: UserStack) -> bool {
    let mut ok = true;
    let mut as_table = ADDRESS_SPACE_TABLE.lock();
    let Ok(user_as) = as_table.get_mut(stack.asid) else {
        return false;
    };

    for page_idx in 0..stack.pages {
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
}

impl Drop for ThreadContext {
    fn drop(&mut self) {
        if let Some(user_stack) = self._user_stack
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
        let base: usize = self._kernel_stack_buf.into();
        (base..base + INIT_KERNEL_STACK_PAGES * PAGE_SIZE).contains(&address)
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
        })
    }

    /// Create the context for a user thread that begins executing at
    /// `entry_point` at EL0 in the address space identified by `asid`, using a
    /// dedicated kernel stack for the in-kernel trampoline and a separate user
    /// stack for EL0 execution.
    pub fn create_user_thread_context(
        asid: AddressSpaceId,
        entry_point: extern "C" fn(),
        user_stack_pages: usize,
    ) -> Result<Self, Error> {
        assert!(
            (1..=charlotte_launch::MAX_USER_STACK_PAGES).contains(&user_stack_pages),
            "invalid userspace stack limit"
        );
        // Allocate user stack pages from physical frames and map them into the
        // user address space.  The kernel stack allocator returns higher-half
        // VAs that have no TTBR0 mapping; EL0 can only use TTBR0.  Because
        // this prototype has no virtual-memory manager we place each user
        // thread's stack at a fixed VA region, offset by a per-thread index.
        //
        // Known limitation: `NEXT_STACK_INDEX` is monotonic and never recycled,
        // and there is no runtime guard against the region eventually colliding
        // with the ELF load region or the scratch window (base 0x0000_0000_4000_0000).
        // The fixed-address loader means this is a bounded-but-unchecked VA
        // reservation, not a dynamically managed one; it is far beyond any
        // practical thread count today but is deliberately not a guarantee.
        const USER_STACK_VADDR_BASE: usize = 0x0000_0000_0100_0000;
        const USER_STACK_STRIDE: usize =
            charlotte_launch::MAX_USER_STACK_PAGES * PAGE_SIZE + PAGE_SIZE; // + guard
        static NEXT_STACK_INDEX: AtomicUsize = AtomicUsize::new(0);
        let stack_index = NEXT_STACK_INDEX.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let stack_base = USER_STACK_VADDR_BASE + stack_index * USER_STACK_STRIDE;
        let user_stack_top_va = stack_base + user_stack_pages * PAGE_SIZE;
        // Pre-allocate all frames first (under the frame allocator lock), then
        // map them (inside the AS table lock).  Order matches el0.rs.
        let mut stack_frames = alloc::vec![None; user_stack_pages];
        {
            let mut pfa = PHYSICAL_FRAME_ALLOCATOR.lock();
            for index in 0..user_stack_pages {
                let frame = match pfa.allocate_frame() {
                    Ok(frame) => frame,
                    Err(_) => {
                        for allocated in stack_frames.iter_mut().filter_map(Option::take) {
                            let _ = pfa.deallocate_frame(allocated);
                        }
                        return Err(Error::InvalidStack);
                    }
                };
                // The physical allocator deliberately only tracks ownership;
                // it does not scrub pages. Every frame must therefore be
                // cleared before it becomes visible in an EL0 address space.
                let page: *mut u8 = frame.into();
                unsafe {
                    core::ptr::write_bytes(page, 0, PAGE_SIZE);
                }
                stack_frames[index] = Some(frame);
            }
        }
        let map_result = {
            let mut as_table = ADDRESS_SPACE_TABLE.lock();
            let Ok(user_as) = as_table.get_mut(asid) else {
                drop(as_table);
                let mut pfa = PHYSICAL_FRAME_ALLOCATOR.lock();
                for frame in stack_frames.iter_mut().filter_map(Option::take) {
                    let _ = pfa.deallocate_frame(frame);
                }
                return Err(Error::InvalidStack);
            };
            let mut result = Ok(());
            for (index, frame) in stack_frames.iter().flatten().copied().enumerate() {
                let vaddr = VAddr::from(stack_base + index * PAGE_SIZE);
                if user_as
                    .map_page(MemoryMapping {
                        vaddr,
                        paddr: frame,
                        page_type: PageType::UserData,
                    })
                    .is_err()
                {
                    for cleanup_index in 0..index {
                        let _ =
                            user_as.unmap_page(VAddr::from(stack_base + cleanup_index * PAGE_SIZE));
                    }
                    result = Err(Error::InvalidStack);
                    break;
                }
            }
            result
        };
        if let Err(error) = map_result {
            let mut pfa = PHYSICAL_FRAME_ALLOCATOR.lock();
            for frame in stack_frames.iter_mut().filter_map(Option::take) {
                let _ = pfa.deallocate_frame(frame);
            }
            return Err(error);
        }
        let user_stack = UserStack {
            asid,
            base: VAddr::from(stack_base),
            pages: user_stack_pages,
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
        })
    }
}
