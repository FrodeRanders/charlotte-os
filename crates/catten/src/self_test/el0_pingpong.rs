//! EL0 ping-pong: two user shards communicate cross-LP via the full svc ABI.
//!
//! Exercises THREAD_EXIT, mailbox endpoint capabilities, COMPLETION_WAIT_TIMEOUT,
//! and SUBMIT Read-with-buffer in one integrated userspace flow:
//!
//! **Ping** (LP0):
//!   SUBMIT(Nop) → cap · MAILBOX_OPEN_SEND(LP1) → sender cap ·
//!   MAILBOX_SEND_CAP(cap to LP1) · WAIT_TIMEOUT · drain CQ · write result
//!   page · EXIT
//! **Pong** (LP1):
//!   MAILBOX_OPEN_RECV → receiver cap · MAILBOX_RECV_CAP → cap ·
//!   SUBMIT(Read, buffer@0x16000, 32) → read_cap · WAIT(read_cap) · drain CQ
//!   · read buffer → verify 0xFEED_F00D · COMPLETE(peer cap, 99) · write
//!   result page · EXIT
//!
//! Requires >= 2 LPs; skipped otherwise.

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
    linear::{
        MemoryMapping,
        PageType,
        VAddr,
    },
    ADDRESS_SPACE_TABLE,
    KERNEL_AS,
};

/// VADDRs in the demo's user address space.
#[cfg(target_arch = "aarch64")]
const PING_VADDR: usize = 0x0000_0000_0002_0000;
#[cfg(target_arch = "aarch64")]
const PONG_VADDR: usize = 0x0000_0000_0001_0000;
#[cfg(target_arch = "aarch64")]
const PP_CQ_VADDR: usize = 0x0000_0000_0001_4000;
#[cfg(target_arch = "aarch64")]
const PP_RESULT_VADDR: usize = 0x0000_0000_0001_5000;
#[cfg(target_arch = "aarch64")]
const PP_BUF_VADDR: usize = 0x0000_0000_0001_6000;

/// Sentinel Ping writes to result[0] on success.
#[cfg(target_arch = "aarch64")]
const PING_SENTINEL: u32 = 0x9100_1500;
/// Sentinel Pong writes to result[0] on success.
#[cfg(target_arch = "aarch64")]
const PONG_SENTINEL: u32 = 0x1000_1000;

/// Physical frame of the result page, read by the verifier via HHDM.
#[cfg(target_arch = "aarch64")]
static mut PP_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

/// Ping stub (LP0).
///
/// ```asm
///     ldr  w20, [result+16]     // asid
///     mov x0,x20; mov x1,#0; svc #1   // SUBMIT(Nop)
///     mov x19, x0
///     mov x0,x20; mov x1,#1; svc #13  // MAILBOX_OPEN_SEND(LP1)
///     mov x18, x0
///     mov x0,x20; mov x1,x18; mov x2,x19; svc #15  // MAILBOX_SEND_CAP cap→LP1
///     mov x0,x20; mov x1,x19; movz x2,#60000; svc #11  // WAIT_TIMEOUT 60000ms
///     cbz x0, drain          // if success: drain; otherwise write 0xDEAD
///     // ... write 0xDEAD, exit ...
/// drain:
///     mov x11,x1             // WAIT_TIMEOUT returns the completion result
///     result[1]=cap; result[2]=result; dmb ish; result[0]=0x9100_1500
///     svc #8
/// ```
#[cfg(target_arch = "aarch64")]
const PING_CODE: &[u8] = &[
    0x09, 0x00, 0x8a, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0x34, 0x11, 0x40, 0xb9, 0xe0, 0x03, 0x14, 0xaa,
    0x01, 0x00, 0x80, 0xd2, 0x21, 0x00, 0x00, 0xd4, 0xf3, 0x03, 0x00, 0xaa, 0xe0, 0x03, 0x14, 0xaa,
    0x21, 0x00, 0x80, 0xd2, 0xa1, 0x01, 0x00, 0xd4, 0xf2, 0x03, 0x00, 0xaa, 0xe0, 0x03, 0x14, 0xaa,
    0xe1, 0x03, 0x12, 0xaa, 0xe2, 0x03, 0x13, 0xaa, 0xe1, 0x01, 0x00, 0xd4, 0xe0, 0x03, 0x14, 0xaa,
    0xe1, 0x03, 0x13, 0xaa, 0x02, 0x4c, 0x9d, 0xd2, 0x61, 0x01, 0x00, 0xd4, 0xc0, 0x00, 0x00, 0xb4,
    0x09, 0x00, 0x8a, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0xaa, 0xd5, 0x9b, 0x52, 0x2a, 0x01, 0x00, 0xb9,
    0x0d, 0x00, 0x00, 0x14, 0xeb, 0x03, 0x01, 0xaa, 0x1f, 0x20, 0x03, 0xd5, 0x1f, 0x20, 0x03, 0xd5,
    0x1f, 0x20, 0x03, 0xd5, 0x1f, 0x20, 0x03, 0xd5, 0x09, 0x00, 0x8a, 0xd2, 0x29, 0x00, 0xa0, 0xf2,
    0x0a, 0xa0, 0x82, 0x52, 0x0a, 0x20, 0xb2, 0x72, 0x33, 0x05, 0x00, 0xb9, 0x2b, 0x09, 0x00, 0xb9,
    0xbf, 0x3b, 0x03, 0xd5, 0x2a, 0x01, 0x00, 0xb9, 0x01, 0x01, 0x00, 0xd4,
];

/// Pong stub (LP1).
///
/// ```asm
///     ldr w20, [result+16]     // asid
///     mov x0,x20; svc #14      // MAILBOX_OPEN_RECV
///     mov x18, x0
/// spin: mov x0,x20; mov x1,x18; svc #16; cbnz x1, spin  // MAILBOX_RECV_CAP
///     mov x19, x0              // cap from Ping
///     mov x0,x20; mov x1,#1; movz x2,#0x6000; movk x2,#0x1; mov x3,#32; svc #1 // SUBMIT(Read,buf)
///     mov x21, x0
///     mov x0,x20; mov x1,x21; svc #4  // WAIT
///     poll CQ head; read entry[0].result
///     ldr w12, [buf]            // verify buffer contains 0xFEED_F00D
///     mov x0,x20; mov x1,x19; mov x2,#99; svc #2  // COMPLETE(peer,99)
///     result[5..7] = cap, read_result, buffer_val
///     dmb ish
///     result[4] = sentinel      // last; overwrites the bootstrap asid slot
///     svc #8
/// ```
#[cfg(target_arch = "aarch64")]
const PONG_CODE: &[u8] = &[
    0x09, 0x00, 0x8a, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0x34, 0x11, 0x40, 0xb9, 0xe0, 0x03, 0x14, 0xaa,
    0xc1, 0x01, 0x00, 0xd4, 0xf2, 0x03, 0x00, 0xaa, 0xe0, 0x03, 0x14, 0xaa, 0xe1, 0x03, 0x12, 0xaa,
    0x01, 0x02, 0x00, 0xd4, 0x81, 0xff, 0xff, 0xb5, 0xf3, 0x03, 0x00, 0xaa, 0xe0, 0x03, 0x14, 0xaa,
    0x21, 0x00, 0x80, 0xd2, 0x02, 0x00, 0x8c, 0xd2, 0x22, 0x00, 0xa0, 0xf2, 0x03, 0x04, 0x80, 0xd2,
    0x21, 0x00, 0x00, 0xd4, 0xf5, 0x03, 0x00, 0xaa, 0xe0, 0x03, 0x14, 0xaa, 0xe1, 0x03, 0x15, 0xaa,
    0x81, 0x00, 0x00, 0xd4, 0x09, 0x00, 0x88, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0x2a, 0x01, 0x40, 0xb9,
    0xea, 0xff, 0xff, 0x34, 0x2b, 0x19, 0x40, 0xb9, 0x09, 0x00, 0x8c, 0xd2, 0x29, 0x00, 0xa0, 0xf2,
    0x2c, 0x01, 0x40, 0xb9, 0xe0, 0x03, 0x14, 0xaa, 0xe1, 0x03, 0x13, 0xaa, 0x62, 0x0c, 0x80, 0xd2,
    0x41, 0x00, 0x00, 0xd4, 0x09, 0x00, 0x8a, 0xd2, 0x29, 0x00, 0xa0, 0xf2, 0x0a, 0x00, 0x82, 0x52,
    0x0a, 0x00, 0xa2, 0x72, 0x33, 0x15, 0x00, 0xb9, 0x2b, 0x19, 0x00, 0xb9, 0x2c, 0x1d, 0x00, 0xb9,
    0xbf, 0x3b, 0x03, 0xd5, 0x2a, 0x11, 0x00, 0xb9, 0x01, 0x01, 0x00, 0xd4,
];

#[cfg(target_arch = "aarch64")]
fn pp_map_code_page(asid: usize, vaddr: VAddr, code: &[u8]) {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("pp: failed to allocate code frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("pp: AS not found")
        .map_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type: PageType::UserCode,
        })
        .expect("pp: failed to map code page");
    let hhdm: *mut u8 = frame.into();
    unsafe {
        core::ptr::copy_nonoverlapping(code.as_ptr(), hhdm, code.len());
    }
}

#[cfg(target_arch = "aarch64")]
fn pp_map_data_page(asid: usize, vaddr: VAddr) -> crate::memory::physical::PAddr {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("pp: failed to allocate data frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("pp: AS not found")
        .map_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type: PageType::UserData,
        })
        .expect("pp: failed to map data page");
    frame
}

pub fn test_el0_ping_pong() {
    #[cfg(target_arch = "aarch64")]
    {
        let lp_count = crate::cpu::multiprocessor::get_lp_count();
        if lp_count < 2 {
            logln!("[PP] single LP, skipping EL0 ping-pong demo");
            return;
        }
        logln!("Testing EL0 ping-pong (Ping on LP0, Pong on LP1, via svc ABI)...");

        // --- create user address space ---
        let user_as = {
            let _kas = KERNEL_AS.lock();
            let mut as_ = AddressSpace::get_current();
            as_.set_ttbr0(0);
            as_
        };
        let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
        logln!("[PP] user AS asid={}", asid);

        pp_map_code_page(asid, VAddr::from(PING_VADDR), PING_CODE);
        pp_map_code_page(asid, VAddr::from(PONG_VADDR), PONG_CODE);
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags)
            );
        }

        let cq_frame = pp_map_data_page(asid, VAddr::from(PP_CQ_VADDR));
        let result_frame = pp_map_data_page(asid, VAddr::from(PP_RESULT_VADDR));
        let _buf_frame = pp_map_data_page(asid, VAddr::from(PP_BUF_VADDR));
        unsafe {
            PP_RESULT_FRAME = Some(result_frame);
        }

        let result_base: *mut u8 = result_frame.into();
        unsafe {
            core::ptr::write_volatile((result_base as *mut u32).add(4), asid as u32);
        }

        completion::open_address_space_with_cq_phys(asid, 16, cq_frame, 32);

        // --- spawn Ping (LP0) and Pong (LP1) ---
        let ping_entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(PING_VADDR) };
        let pong_entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(PONG_VADDR) };

        {
            use crate::cpu::scheduler::{
                system_scheduler::SYSTEM_SCHEDULER,
                threads::{
                    Thread,
                    MASTER_THREAD_TABLE,
                },
            };
            let t = Thread::new(asid as crate::memory::AddressSpaceId, ping_entry);
            let tid = MASTER_THREAD_TABLE.write().add_element(t);
            SYSTEM_SCHEDULER.read().submit_to_lp(tid, 0).expect("PP: failed to pin Ping to LP0");
            logln!("[PP] Ping spawned tid={}, pinned to LP0", tid);
        }
        {
            use crate::cpu::scheduler::{
                system_scheduler::SYSTEM_SCHEDULER,
                threads::{
                    Thread,
                    MASTER_THREAD_TABLE,
                },
            };
            let t = Thread::new(asid as crate::memory::AddressSpaceId, pong_entry);
            let tid = MASTER_THREAD_TABLE.write().add_element(t);
            SYSTEM_SCHEDULER.read().submit_to_lp(tid, 1).expect("PP: failed to pin Pong to LP1");
            logln!("[PP] Pong spawned tid={}, pinned to LP1", tid);
        }

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_ping_pong);
        logln!("[PP] verifier tid={}; assertion deferred.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 ping-pong demo (AArch64 only).");
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_ping_pong() {
    use crate::cpu::scheduler::yield_lp;

    let frame = unsafe { PP_RESULT_FRAME }.expect("PP: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let s0 = unsafe { core::ptr::read_volatile(result) };
        let s1 = unsafe { core::ptr::read_volatile(result.add(4)) };
        // Both threads write to the same result page at different offsets.
        // Ping writes at [0..2]; Pong at [4..7].
        if s0 == PING_SENTINEL && s1 == PONG_SENTINEL {
            let ping_cap = unsafe { core::ptr::read_volatile(result.add(1)) };
            let ping_result_raw = unsafe { core::ptr::read_volatile(result.add(2)) } as i32;
            let pong_cap = unsafe { core::ptr::read_volatile(result.add(5)) };
            let pong_read_result = unsafe { core::ptr::read_volatile(result.add(6)) };
            let pong_buf_val = unsafe { core::ptr::read_volatile(result.add(7)) };

            assert_eq!(ping_result_raw, 99, "PP Ping: expected result 99, got {}", ping_result_raw);
            assert_eq!(
                pong_cap, ping_cap,
                "PP: Ping and Pong cap mismatch {} vs {}",
                ping_cap, pong_cap
            );
            assert_eq!(
                pong_read_result, 32,
                "PP Pong: expected Read result 32, got {}",
                pong_read_result
            );
            assert_eq!(
                pong_buf_val, 0xfeed_f00d,
                "PP Pong: expected buffer value 0xFEED_F00D, got {:#x}",
                pong_buf_val
            );

            logln!(
                "[PP] SUCCESS: Ping completed with result {}; Pong read buffer {:#x}; all via svc \
                 ABI (EXIT, MAILBOX_CAP, WAIT_TIMEOUT, SUBMIT Read).",
                ping_result_raw,
                pong_buf_val
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 80_000_000,
            "[PP] FAILED: ping-pong did not complete (ping={:#x}, pong={:#x})",
            s0,
            s1
        );
        yield_lp();
    }
}
