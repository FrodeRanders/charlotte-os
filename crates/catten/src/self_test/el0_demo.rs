//! EL0 (userspace) variant of the async + cross-LP demo.
//!
//! Where [`crate::demo`] drives the completion and shard primitives from
//! EL1 kernel threads by calling the in-kernel APIs directly, this exercise
//! drives the *same* async round-trip entirely from EL0 through the `svc` ABI,
//! and demonstrates cross-LP work placement via the `SPAWN_THREAD` syscall.
//!
//! Two hand-written EL0 stubs run in one user address space:
//!
//! - **coordinator** (spawned by the kernel): `svc #1` COMPLETION_SUBMIT to get
//!   a capability, `svc #7` SPAWN_THREAD to launch the worker pinned to LP1,
//!   `svc #4` COMPLETION_WAIT to block until it completes, then drains the
//!   result from the shared CQ ring (zero-syscall) and writes it to the result
//!   page.
//! - **worker** (spawned by the coordinator onto LP1): `svc #2`
//!   COMPLETION_COMPLETE to post the result, then spins.
//!
//! Because self-tests run before `yield_lp()`, a kernel verifier thread is
//! spawned to observe the result page once the scheduler is active; it asserts
//! the sentinel, the returned capability, and the completion result, panicking
//! on mismatch or timeout.
//!
//! Requires at least two LPs (for the cross-LP placement); it is skipped
//! otherwise.

#[cfg(target_arch = "aarch64")]
use crate::completion;
#[cfg(target_arch = "aarch64")]
use crate::cpu::isa::interface::memory::AddressSpaceInterface;
#[cfg(target_arch = "aarch64")]
use crate::cpu::isa::memory::paging::AddressSpace;
#[cfg(target_arch = "aarch64")]
use crate::cpu::scheduler::spawn_thread;
use crate::logln;
#[cfg(target_arch = "aarch64")]
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
#[cfg(target_arch = "aarch64")]
use crate::memory::{
    linear::{MemoryMapping, PageType, VAddr},
    ADDRESS_SPACE_TABLE, KERNEL_AS,
};

/// User virtual addresses in the demo's address space.
#[cfg(target_arch = "aarch64")]
const COORD_CODE_VADDR: usize = 0x0000_0000_0001_0000;
#[cfg(target_arch = "aarch64")]
const WORKER_CODE_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const CQ_VADDR: usize = 0x0000_0000_0001_2000;
#[cfg(target_arch = "aarch64")]
const RESULT_VADDR: usize = 0x0000_0000_0001_3000;

/// The completion result the worker posts and the coordinator reads back.
#[cfg(target_arch = "aarch64")]
const EXPECTED_RESULT: u32 = 42;

/// Physical frame of the result page, read by the verifier via HHDM.
#[cfg(target_arch = "aarch64")]
static mut DEMO_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

/// Coordinator stub. `asid` is read from `result[4]` (written by the kernel).
///
/// ```asm
///     movz x9, #0x3000; movk x9, #0x1, lsl #16   // x9 = RESULT_VADDR
///     ldr  w20, [x9, #16]                        // x20 = asid
///     mov  x0, x20; mov x1, #0; svc #1           // COMPLETION_SUBMIT -> x0 = cap
///     mov  x19, x0                               // save cap
///     mov  x0, x20; movz x1,#0x1000; movk x1,#0x1,lsl#16; mov x2,#1; svc #7  // SPAWN_THREAD worker@0x11000 on LP1
///     mov  x0, x20; mov x1, x19; svc #4          // COMPLETION_WAIT
///     movz x9, #0x2000; movk x9,#0x1,lsl#16      // x9 = CQ_VADDR
/// 1:  ldr  w10, [x9]; cbz w10, 1b                // poll ring head
///     ldr  w11, [x9, #24]                        // entry[0].result
///     movz x9, #0x3000; movk x9,#0x1,lsl#16      // x9 = RESULT_VADDR
///     movz w12, #0xc0de
///     str  w19, [x9, #4]                         // result[1] = cap
///     str  w11, [x9, #8]                         // result[2] = completion result
///     dmb  ish
///     str  w12, [x9]                             // result[0] = 0xC0DE (last)
/// 2:  nop; b 2b
/// ```
#[cfg(target_arch = "aarch64")]
const COORD_CODE: &[u8] = &[
    0x09, 0x00, 0x86, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0x34, 0x11, 0x40, 0xb9, 0xe0, 0x03, 0x14, 0xaa,
    0x01, 0x00, 0x80, 0xd2, 0x21, 0x00, 0x00, 0xd4, 0xf3, 0x03, 0x00, 0xaa, 0xe0, 0x03, 0x14, 0xaa,
    0x01, 0x00, 0x82, 0xd2, 0x21, 0x00, 0xa0, 0xf2, 0x22, 0x00, 0x80, 0xd2, 0xe1, 0x00, 0x00, 0xd4,
    0xe0, 0x03, 0x14, 0xaa, 0xe1, 0x03, 0x13, 0xaa, 0x81, 0x00, 0x00, 0xd4, 0x09, 0x00, 0x84, 0xd2,
    0x29, 0x00, 0xa0, 0xf2, 0x2a, 0x01, 0x40, 0xb9, 0xea, 0xff, 0xff, 0x34, 0x2b, 0x19, 0x40, 0xb9,
    0x09, 0x00, 0x86, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0xcc, 0x1b, 0x98, 0x52, 0x33, 0x05, 0x00, 0xb9,
    0x2b, 0x09, 0x00, 0xb9, 0xbf, 0x3b, 0x03, 0xd5, 0x2c, 0x01, 0x00, 0xb9, 0x1f, 0x20, 0x03, 0xd5,
    0xff, 0xff, 0xff, 0x17,
];

/// Worker stub. Reads `asid` from `result[4]`, completes capability 0 (the
/// coordinator's first and only submit) with result 42, then spins.
///
/// ```asm
///     movz x9, #0x3000; movk x9,#0x1,lsl#16; ldr w0, [x9, #16]   // x0 = asid
///     mov  x1, #0; mov x2, #42; svc #2                            // COMPLETION_COMPLETE(cap=0, 42)
/// 1:  nop; b 1b
/// ```
#[cfg(target_arch = "aarch64")]
const WORKER_CODE: &[u8] = &[
    0x09, 0x00, 0x86, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0x20, 0x11, 0x40, 0xb9, 0x01, 0x00, 0x80, 0xd2,
    0x42, 0x05, 0x80, 0xd2, 0x41, 0x00, 0x00, 0xd4, 0x1f, 0x20, 0x03, 0xd5, 0xff, 0xff, 0xff, 0x17,
];

#[cfg(target_arch = "aarch64")]
fn map_code_page(asid: usize, vaddr: VAddr, code: &[u8]) {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("el0_demo: failed to allocate code frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("el0_demo: AS not found for code mapping")
        .map_page(MemoryMapping { vaddr, paddr: frame, page_type: PageType::UserCode })
        .expect("el0_demo: failed to map code page");
    let hhdm: *mut u8 = frame.into();
    unsafe {
        core::ptr::copy_nonoverlapping(code.as_ptr(), hhdm, code.len());
    }
}

#[cfg(target_arch = "aarch64")]
fn map_data_page(asid: usize, vaddr: VAddr) -> crate::memory::physical::PAddr {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("el0_demo: failed to allocate data frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("el0_demo: AS not found for data mapping")
        .map_page(MemoryMapping { vaddr, paddr: frame, page_type: PageType::UserData })
        .expect("el0_demo: failed to map data page");
    frame
}

pub fn test_el0_cross_lp_async() {
    #[cfg(target_arch = "aarch64")]
    {
        let lp_count = crate::cpu::multiprocessor::get_lp_count();
        if lp_count < 2 {
            logln!("[EL0 xLP] single LP, skipping EL0 cross-LP async demo");
            return;
        }
        logln!("Testing EL0 cross-LP async round-trip (via svc ABI)...");

        // --- create the demo's user address space ---
        let user_as = {
            let _kas = KERNEL_AS.lock();
            let mut as_ = AddressSpace::get_current();
            as_.set_ttbr0(0);
            as_
        };
        let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
        logln!("[EL0 xLP] user AS asid={}", asid);

        // --- map code (coordinator + worker), CQ ring, and result pages ---
        map_code_page(asid, VAddr::from(COORD_CODE_VADDR), COORD_CODE);
        map_code_page(asid, VAddr::from(WORKER_CODE_VADDR), WORKER_CODE);
        // Make the freshly written code visible to instruction fetch.
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        let cq_frame = map_data_page(asid, VAddr::from(CQ_VADDR));
        let result_frame = map_data_page(asid, VAddr::from(RESULT_VADDR));
        unsafe { DEMO_RESULT_FRAME = Some(result_frame); }

        // Hand the address-space id to the EL0 stubs via result[4]; they read it
        // instead of hard-coding, so this exercise does not depend on the asid
        // the setup happens to be assigned.
        let result_base: *mut u8 = result_frame.into();
        unsafe {
            core::ptr::write_volatile((result_base as *mut u32).add(4), asid as u32);
        }

        // Attach a completion table + CQ ring on the same physical frame that is
        // mapped into the user AS, so the kernel's `complete()` posts entries the
        // EL0 coordinator can drain directly.
        completion::open_address_space_with_cq_phys(asid, 16, cq_frame, 32);

        // --- spawn the EL0 coordinator; it spawns the worker on LP1 itself ---
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(COORD_CODE_VADDR) };
        let tid = spawn_thread(asid as crate::memory::AddressSpaceId, entry);
        logln!("[EL0 xLP] coordinator spawned tid={} asid={}", tid, asid);

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_demo);
        logln!("[EL0 xLP] verifier thread tid={}; assertion deferred to scheduler.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 cross-LP async demo (AArch64 only).");
    }
}

/// Kernel thread that verifies the EL0 demo's result page once the scheduler is
/// running. Polls cooperatively (no timer/blocking), then asserts.
#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_demo() {
    use crate::cpu::scheduler::yield_lp;

    let frame = unsafe { DEMO_RESULT_FRAME }.expect("el0_demo: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let sentinel = unsafe { core::ptr::read_volatile(result) };
        if sentinel == 0xC0DE {
            let cap = unsafe { core::ptr::read_volatile(result.add(1)) };
            let value = unsafe { core::ptr::read_volatile(result.add(2)) };
            assert_eq!(cap, 0, "EL0 xLP: expected coordinator cap 0, got {}", cap);
            assert_eq!(
                value, EXPECTED_RESULT,
                "EL0 xLP: expected completion result {}, got {}",
                EXPECTED_RESULT, value
            );
            logln!(
                "[EL0 xLP] SUCCESS: EL0 coordinator submitted cap {}, worker completed cross-LP, \
                 result {} drained from CQ ring \u{2014} all via svc.",
                cap, value
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 40_000_000,
            "[EL0 xLP] FAILED: coordinator did not post its result page sentinel",
        );
        yield_lp();
    }
}
