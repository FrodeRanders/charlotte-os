//! Self-test: create a user thread at EL0 that invokes SVC and verifies the
//! kernel's syscall dispatch → return path *and* its side effects.
//!
//! This is the first real-EL0 exercise in the kernel. It:
//! 1. Creates a user address space, maps a code page (`AP_EL0`), a shared CQ ring page, and a
//!    writable result page.
//! 2. Writes a small hand-written AArch64 stub to the code page.
//! 3. Creates a user thread whose entry point is the mapped code page.
//! 4. The stub executes `SVC #1` (COMPLETION_SUBMIT), stores the returned capability and a sentinel
//!    to the result page, then loops via `wfe`.
//!
//! When the user thread runs, `SVC #1` traps to `sync_dispatcher`, which decodes
//! ESR_EL1.EC, builds a TrapFrame, dispatches to the submit handler (which
//! writes the allocated cap back into x0), advances ELR_EL1 by 4, and `eret`s
//! back to EL0. The stub then writes `0xDEAD` and the returned cap to the
//! result page.
//!
//! Because self-tests run on the boot path *before* `yield_lp()`, the spawned
//! user thread cannot run inline. A companion kernel thread
//! ([`verify_el0_result`]) is spawned to poll the result page (via its HHDM
//! alias) once the scheduler is active and assert the sentinel and returned
//! cap, panicking on mismatch or timeout.

#[cfg(target_arch = "aarch64")]
use crate::{
    completion::{
        self,
        OpCode,
        OpResult,
    },
    cpu::{
        isa::{
            interface::memory::AddressSpaceInterface,
            memory::paging::AddressSpace,
        },
        scheduler::spawn_thread,
    },
    logln,
    memory::{
        ADDRESS_SPACE_TABLE,
        KERNEL_AS,
        PHYSICAL_FRAME_ALLOCATOR,
        linear::{
            MemoryMapping,
            PageType,
            VAddr,
        },
    },
};

/// Physical frame of the result page, stored so the test function can read the
/// user binary's output via HHDM after the thread runs.
#[cfg(target_arch = "aarch64")]
static mut TEST_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

const USER_CODE_VADDR: usize = 0x0000_0000_0001_0000;
const USER_CQ_VADDR: usize = 0x0000_0000_0001_1000;
const USER_RESULT_VADDR: usize = 0x0000_0000_0001_2000;

/// Hand-written, position-independent AArch64 EL0 stub. Replaces the previously
/// embedded (and stale) `catten-user.bin` so the test validates the *current*
/// syscall ABI rather than a committed binary. It exercises the submit path and
/// the syscall return-value contract:
///
/// ```asm
///     mov   x0, #0                 // unused by the ASID authority path
///     mov   x1, #0                 // OpCode::Nop
///     svc   #1                     // COMPLETION_SUBMIT -> kernel returns cap in x0
///     movz  x2, #0x2000
///     movk  x2, #0x1, lsl #16      // x2 = 0x0001_2000 (USER_RESULT_VADDR)
///     movz  w3, #0xdead            // sentinel proving the stub ran
///     str   w3, [x2]               // result[0] = 0xDEAD
///     str   w0, [x2, #4]            // result[1] = returned cap
/// 1:  nop
///     b     1b
/// ```
///
/// The spin loop uses `nop` rather than `wfe`: at EL0, `WFE` traps to EL1 under
/// hypervisors such as Apple's HVF (`EC=1`, "trapped WF*"), whereas a plain
/// branch loop does not and is simply preempted by the scheduler tick.
///
/// Assembled with `clang -arch arm64`; the little-endian encodings below are
/// copied verbatim from `llvm-objdump -d`.
#[cfg(target_arch = "aarch64")]
const USER_THREAD_CODE: &[u8] = &[
    0x00, 0x00, 0x80, 0xd2, // mov  x0, #0
    0x01, 0x00, 0x80, 0xd2, // mov  x1, #0
    0x21, 0x00, 0x00, 0xd4, // svc  #1
    0x02, 0x00, 0x84, 0xd2, // mov  x2, #0x2000
    0x22, 0x00, 0xa0, 0xf2, // movk x2, #0x1, lsl #16
    0xa3, 0xd5, 0x9b, 0x52, // mov  w3, #0xdead
    0x43, 0x00, 0x00, 0xb9, // str  w3, [x2]
    0x40, 0x04, 0x00, 0xb9, // str  w0, [x2, #4]
    0x1f, 0x20, 0x03, 0xd5, // nop
    0xff, 0xff, 0xff, 0x17, // b .-4
];

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
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("failed to retrieve AS for mapping")
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
            "dsb ishst",  // ensure prior stores are visible
            "ic ialluis", // invalidate I-cache (all, inner shareable)
            "dsb ish",    // ensure I-cache invalidation is complete
            "isb",        // synchronize context
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
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("failed to retrieve AS for CQ mapping")
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
    unsafe {
        TEST_RESULT_FRAME = Some(result_frame);
    }

    let result_mapping = MemoryMapping {
        vaddr: result_vaddr,
        paddr: result_frame,
        page_type: PageType::UserData,
    };
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("failed to retrieve AS for result mapping")
        .map_page(result_mapping)
        .expect("failed to map result page");

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
        assert_eq!(completion::cq_pending(asid, 0), 0);
        completion::complete(asid, cap, OpResult::Ok(1)).unwrap();
        assert_eq!(completion::cq_pending(asid, 0), 1);

        // Read head via HHDM — the kernel should see head == 1 (one entry written).
        let ring_ptr = completion::cq_ring_of(asid, 0).expect("CQ ring must be attached");
        let ring = unsafe { &mut *ring_ptr };
        let head = unsafe { core::ptr::read_volatile(&ring.head) };
        assert_eq!(head, 1, "kernel must see head == 1 after one completion");

        // Free this cap so the freed id (0) is what the user thread's own
        // COMPLETION_SUBMIT will be assigned. A returned cap of 0 proves the
        // kernel actually wrote x0 on the way out.
        completion::close(asid, cap).unwrap();

        // Spawn the EL0 user thread. The completion table + phys-mapped CQ that
        // `prepare_user_address_space` attached stay in place — we deliberately
        // do NOT reopen the AS with a heap-backed CQ, which would detach the
        // page mapped into the user address space.
        let tid = spawn_thread(asid as crate::memory::AddressSpaceId, user_thread_entry_ptr(vaddr));
        logln!("User thread spawned with tid={} asid={} vaddr={:?}", tid, asid, vaddr);

        // The verification thread runs after `yield_lp()` (self-tests run on the
        // boot path before the scheduler is entered), polls the result page via
        // HHDM, and asserts the sentinel + returned cap. It panics on timeout or
        // mismatch, so a broken EL0/submit path fails the boot rather than
        // silently logging success.
        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_result);
        logln!("EL0 verifier thread spawned with tid={}; assertion deferred to scheduler.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 SVC round-trip test (AArch64 only).");
    }
}

/// Kernel thread that verifies the EL0 user thread's side effects. Polls the
/// result page (via its HHDM alias) until the stub writes its sentinel, then
/// asserts the returned completion cap. Panics on mismatch or timeout.
///
/// Uses cooperative `yield_lp` polling rather than `sleep` so it does not add a
/// blocking waiter to the timer path while the rest of the system is coming up.
#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_result() {
    use crate::cpu::scheduler::yield_lp;

    let frame = unsafe { TEST_RESULT_FRAME }.expect("EL0 test: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let sentinel = unsafe { core::ptr::read_volatile(result) };
        if sentinel == 0xdead {
            let cap = unsafe { core::ptr::read_volatile(result.add(1)) };
            assert_eq!(
                cap, 0,
                "EL0: COMPLETION_SUBMIT must return the kernel cap (0) in x0, got {}",
                cap
            );
            logln!(
                "[EL0] SUCCESS: user thread ran at EL0, submit returned cap {}, result page \
                 verified.",
                cap
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 20_000_000,
            "[EL0] FAILED: user thread did not write the result-page sentinel",
        );
        yield_lp();
    }
}
