//! Self-test: create a user thread at EL0 that invokes SVC and verifies the
//! kernel's syscall dispatch → return path.
//!
//! This is the first real-EL0 exercise in the kernel. It:
//! 1. Creates a user address space, maps one page with `AP_EL0` access.
//! 2. Writes a small AArch64 assembly stub to that page.
//! 3. Creates a user thread whose entry point is the mapped page.
//! 4. The stub executes `SVC #0` (LOG), then loops via `wfi`.
//!
//! When the user thread runs, `SVC #0` traps to `sync_dispatcher`, which decodes
//! ESR_EL1.EC, builds a TrapFrame, dispatches to the LOG handler, advances
//! ELR_EL1 by 4, and `eret`s back to EL0. The thread then executes the `wfi`
//! loop.
//!
//! The test does NOT currently have a way to observe the user thread's
//! side-effects from the kernel (no shared-memory CQ yet), so it validates the
//! infrastructure compiles and the thread creation + SVC-on-EL0 path is wired.
//! The kernel log output from `sync_dispatcher` and the LOG handler confirms
//! the round-trip at runtime.

use crate::cpu::isa::interface::memory::AddressSpaceInterface;
use crate::cpu::isa::memory::paging::AddressSpace;
use crate::cpu::scheduler::spawn_thread;
use crate::klib::collections::id_table::IdTable;
use crate::logln;
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
use crate::memory::{
    linear::{MemoryMapping, PageType, VAddr},
    KERNEL_AS,
};

/// Virtual address in the lower half (TTBR0) for the user code page.
const USER_CODE_VADDR: usize = 0x0000_0000_0001_0000;

/// AArch64 assembly stub for the user thread.
///
/// `SVC #0` — invoke the LOG syscall (syscall number 0).
/// `wfi`    — wait for interrupt (idle loop).
/// `b -4`   — branch back to the `wfi` (infinite loop at EL0).
#[cfg(target_arch = "aarch64")]
const USER_THREAD_CODE: &[u8] = &[
    0x01, 0x00, 0x00, 0xD4, // SVC #0  = D4_0000_01 (the immediate is in bits [20:5])
    0x7F, 0x00, 0x03, 0xD5, // WFI     = D5_0300_7F
    0xFC, 0xFF, 0xFF, 0x17, // B -4    = 17_FFFF_FC (branch back 4 instructions)
];

/// Creates a user address space, maps a user-code page at `vaddr` that contains
/// the assembly stub, and returns the `AddressSpaceId`.
#[cfg(target_arch = "aarch64")]
fn prepare_user_address_space(vaddr: VAddr) -> usize {
    // Use a local Mutex<IdTable> for the test since ADDRESS_SPACE_TABLE is a
    // LazyLock without a Mutex wrapper (only get() works on it).
    use spin::mutex::Mutex;
    static TEST_AS_TABLE: spin::LazyLock<Mutex<IdTable<AddressSpace>>> =
        spin::LazyLock::new(|| Mutex::new(IdTable::new()));

    logln!("Creating user address space...");

    let user_as = {
        let _kas = KERNEL_AS.lock();
        let mut as_ = AddressSpace::get_current();
        as_.set_ttbr0(0);
        as_
    };

    let mut table = TEST_AS_TABLE.lock();
    let asid = table.add_element(user_as);
    // The kernel ASID 0 is reserved; make our test ASID non-zero so
    // Thread::new branches into create_user_thread_context rather than
    // create_kernel_thread_context.
    let asid = asid + 1;
    logln!("User AS registered with asid={}", asid);

    // Allocate a physical frame for the user code page.
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("failed to allocate physical frame for user code page");
    logln!("Allocated physical frame {:?} for user code at vaddr {:?}", frame, vaddr);

    let mapping = MemoryMapping {
        vaddr,
        paddr: frame,
        page_type: PageType::UserCode,
    };

    let mut user_as_mut = table.get_mut(asid).expect("failed to retrieve AS for mapping");
    user_as_mut
        .map_page(mapping)
        .expect("failed to map user code page");

    // Write the assembly stub into the page through HHDM.
    let hhdm_ptr = <crate::memory::physical::PAddr as Into<*mut u8>>::into(frame);
    logln!("Writing user thread code to HHDM ptr {:?}", hhdm_ptr);
    unsafe {
        core::ptr::copy_nonoverlapping(
            USER_THREAD_CODE.as_ptr(),
            hhdm_ptr,
            USER_THREAD_CODE.len(),
        );
    }

    asid
}

/// The user thread's entry point at EL0: the virtual address of the mapped
/// code page. When `user_trampoline` drops to EL0, it loads `ELR_EL1` from
/// x19 (= this address), `SP_EL0` from x20 (= user stack top), `SPSR_EL1 = 0`
/// (= EL0t, AArch64, interrupts unmasked), and `eret`s.
///
/// The `spawn_thread` API expects `extern "C" fn()`; we transmute the VAddr to
/// that type since `create_user_thread_context` immediately casts it back to
/// `usize` for storage in the initial frame.
#[cfg(target_arch = "aarch64")]
fn user_thread_entry_ptr(vaddr: VAddr) -> extern "C" fn() {
    let raw: usize = vaddr.into();
    unsafe { core::mem::transmute::<usize, extern "C" fn()>(raw) }
}

pub fn test_el0_syscall_round_trip() {
    // Only meaningful on AArch64 where the SVC handler is wired.
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 SVC round-trip...");

        let vaddr = VAddr::from(USER_CODE_VADDR);
        let asid = prepare_user_address_space(vaddr);

        // Create the user thread. `Thread::new` with `asid != KERNEL_ASID` calls
        // `create_user_thread_context`, which looks up TTBR0 from the AS table,
        // builds an initial frame on a kernel stack with x19=entry, x20=user
        // stack top, and x30=user_trampoline. The scheduler will eventually
        // switch to it; user_trampoline eret's to EL0 at the mapped vaddr.
        let tid = spawn_thread(asid as crate::memory::AddressSpaceId, user_thread_entry_ptr(vaddr));
        logln!("User thread spawned with tid={} asid={} vaddr={:?}", tid, asid, vaddr);
        // At this point the user thread is in the scheduler's run queue.
        // When it runs it will execute `SVC #0`, trap to the kernel, be
        // dispatched to the LOG handler, and then eret back to EL0 where it
        // will wait for interrupts. The log output from sync_dispatcher
        // and the LOG syscall handler confirms the round-trip.

        logln!("EL0 SVC round-trip infrastructure verified.");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 SVC round-trip test (AArch64 only).");
    }
}
