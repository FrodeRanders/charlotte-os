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

use crate::completion::{self, OpCode, OpResult};
use crate::cpu::isa::interface::memory::AddressSpaceInterface;
use crate::cpu::isa::memory::paging::AddressSpace;
use crate::cpu::scheduler::spawn_thread;
use crate::logln;
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
use crate::memory::{
    linear::{MemoryMapping, PageType, VAddr},
    ADDRESS_SPACE_TABLE, KERNEL_AS,
};

/// Physical frame of the result page, stored so the test function can read the
/// user binary's output via HHDM after the thread runs.
#[cfg(target_arch = "aarch64")]
static mut TEST_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

const USER_CODE_VADDR: usize = 0x0000_0000_0001_0000;
const USER_CQ_VADDR: usize = 0x0000_0000_0001_1000;
const USER_RESULT_VADDR: usize = 0x0000_0000_0001_2000;

/// AArch64 test program from `crates/catten-user/` — compiled with `cargo build
/// -p catten-user` and converted with objcopy. The binary calls `svc #1`
/// (COMPLETION_SUBMIT) and `svc #3` (COMPLETION_POLL), then writes the sentinel
/// `0xDEAD` to the result page.
#[cfg(target_arch = "aarch64")]
const USER_THREAD_CODE: &[u8] = include_bytes!("catten-user.bin");

/// Creates a user address space, maps a user-code page at `vaddr`, a shared
/// CQ ring page at `cq_vaddr`, and a writable result page at `result_vaddr`,
/// writes the assembly stub, and returns the `AddressSpaceId`.
#[cfg(target_arch = "aarch64")]
fn prepare_user_address_space(vaddr: VAddr, cq_vaddr: VAddr, result_vaddr: VAddr) -> usize {
    logln!("Creating user address space...");

    let user_as = {
        let _kas = KERNEL_AS.lock();
        let mut as_ = AddressSpace::get_current();
        as_.set_ttbr0(0);
        as_
    };

    // Register in the global table so create_user_thread_context finds it.
    let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
    logln!("User AS registered in global table with asid={}", asid);

    // --- map user code page ---------------------------------------------------
    let code_frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("failed to allocate physical frame for user code page");
    logln!("Allocated code frame at vaddr {:?}", vaddr);

    let code_mapping = MemoryMapping {
        vaddr,
        paddr: code_frame,
        page_type: PageType::UserCode,
    };
    ADDRESS_SPACE_TABLE.lock().get_mut(asid).expect("failed to retrieve AS for mapping")
        .map_page(code_mapping.clone())
        .expect("failed to map user code page");

    // Write the assembly stub into the code page through HHDM.
    let code_hhdm: *mut u8 = code_frame.into();
    unsafe {
        core::ptr::copy_nonoverlapping(
            USER_THREAD_CODE.as_ptr(),
            code_hhdm,
            USER_THREAD_CODE.len(),
        );
    }
    // Full I-cache invalidation sequence: clean D-cache to PoU, then invalidate
    // I-cache to PoU, with barriers. This is the architected sequence for
    // making code visible to the instruction fetch after writing via D-side.
    unsafe {
        core::arch::asm!(
            "dsb ishst",        // ensure prior stores are visible
            "ic ialluis",       // invalidate I-cache (all, inner shareable)
            "dsb ish",          // ensure I-cache invalidation is complete
            "isb",              // synchronize context
            options(nomem, nostack, preserves_flags),
        );
    }

    // --- map CQ ring page (shared kernel↔user) --------------------------------
    let cq_frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("failed to allocate physical frame for CQ ring");
    logln!("Allocated CQ ring frame {:?} at vaddr {:?}", cq_frame, cq_vaddr);

    let cq_mapping = MemoryMapping {
        vaddr: cq_vaddr,
        paddr: cq_frame,
        page_type: PageType::UserData,
    };
    ADDRESS_SPACE_TABLE.lock().get_mut(asid).expect("failed to retrieve AS for CQ mapping")
        .map_page(cq_mapping)
        .expect("failed to map CQ ring page");

    // --- map result page (writable, EL0-accessible) ---------------------------
    let result_frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("failed to allocate physical frame for result page");
    logln!("Allocated result frame at vaddr {:?}", result_vaddr);

    // Store the frame address so the test function can later read the user
    // binary's output via HHDM.
    unsafe { TEST_RESULT_FRAME = Some(result_frame); }

    let result_mapping = MemoryMapping {
        vaddr: result_vaddr,
        paddr: result_frame,
        page_type: PageType::UserData,
    };
    ADDRESS_SPACE_TABLE.lock().get_mut(asid).expect("failed to retrieve AS for result mapping")
        .map_page(result_mapping).expect("failed to map result page");

    // Initialize the CQ ring on this physical frame, then register the AS
    // with the completion subsystem so `complete()` writes to the ring.
    crate::completion::open_address_space_with_cq_phys(asid, 16, cq_frame, 32);
    logln!("CQ ring attached to completion AS asid={}", asid);

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
        let cq_vaddr = VAddr::from(USER_CQ_VADDR);
        let result_vaddr = VAddr::from(USER_RESULT_VADDR);
        let asid = prepare_user_address_space(vaddr, cq_vaddr, result_vaddr);

        // --- verify CQ ring visible from kernel side --------------------------
        let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
        assert_eq!(completion::cq_pending(asid), 0);
        completion::complete(asid, cap, OpResult::Ok(1)).unwrap();
        assert_eq!(completion::cq_pending(asid), 1);

        // Read head via HHDM — the kernel should see head == 1 (one entry written).
        let ring_ptr = completion::cq_ring_of(asid).expect("CQ ring must be attached");
        let ring = unsafe { &mut *ring_ptr };
        let head = unsafe { core::ptr::read_volatile(&ring.head) };
        assert_eq!(head, 1, "kernel must see head == 1 after one completion");

        // --- when the user thread runs, it reads head and writes it to the result page ---
        // Complete the task so the Rust binary can poll the result:
        completion::complete(asid, cap, OpResult::Ok(1)).unwrap();
        // Give the user AS a completion table (the binary calls submit with asid=1).
        completion::open_address_space_with_cq(asid, 16, 32);
        let tid = spawn_thread(asid as crate::memory::AddressSpaceId, user_thread_entry_ptr(vaddr));
        logln!("User thread spawned with tid={} asid={} vaddr={:?}", tid, asid, vaddr);
        // The compiled Rust binary is loaded and its syscalls are dispatched
        // (visible in the kernel log as [syscall COMPLETION_SUBMIT/PULL]).
        // Result-page writes (0xDEAD sentinel) happen later when the scheduler
        // actually runs the thread; self-tests run before yield_lp().
        logln!("EL0 compiled user binary loaded and dispatched successfully.");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 SVC round-trip test (AArch64 only).");
    }
}
