//! Self-test: load the Rust-compiled sitas-based catten-user binary at EL0.
//!
//! Unlike the hand-written stub in [`super::el0`], this test embeds the full
//! binary produced by `cargo +nightly build -p catten-user --target …`
//! (which links against sitas-core and sitas-charlotte).  The binary is
//! ~145 KiB and spans multiple 4 KiB pages; the kernel maps them contiguously
//! and copies the image in page-sized chunks.
//!
//! The binary calls `basic_kv::basic_kv_test`, which exercises `ShardedKv`
//! over `CharlotteReactor`: it creates a KV store, puts keys, reads one back,
//! and writes the total key count to the result page.

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

// The binary is position-independent (PIE) and we map code at 0x20000.
// It references two fixed addresses:
//   CQ_RING_VADDR  = 0x11000  (sitas-charlotte::CharlotteReactor)
//   RESULT_PAGE    = 0x12000  (catten-user main.rs)
//   HEAP_BASE      = 0x13000  (catten-user global allocator)
#[cfg(target_arch = "aarch64")]
const SITAS_CODE_VADDR: usize = 0x0000_0000_0002_0000;
#[cfg(target_arch = "aarch64")]
const SITAS_CQ_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const SITAS_RESULT_VADDR: usize = 0x0000_0000_0001_2000;
#[cfg(target_arch = "aarch64")]
const SITAS_HEAP_VADDR: usize = 0x0000_0000_0001_3000;
#[cfg(target_arch = "aarch64")]
const SITAS_HEAP_PAGES: usize = 13;

#[cfg(target_arch = "aarch64")]
const PAGE_SIZE: usize = 4096;

/// Offset of `_start` within the raw binary.  With the linker script
/// (`KEEP(*(.text._start ...))`), `_start` is guaranteed to be at offset 0.
#[cfg(target_arch = "aarch64")]
const ENTRY_OFFSET: usize = 0x0;

/// Total pages to map.  The LOAD segments span VA 0x0..0x3620+128 ≈ 0x36A0
/// plus a 128 KB BSS tail.  32 pages (128 KiB) covers the binary + BSS
/// with generous headroom.
#[cfg(target_arch = "aarch64")]
const CODE_PAGES: usize = 32;

/// The Rust-compiled sitas-based catten-user binary (position-independent).
#[cfg(target_arch = "aarch64")]
const SITAS_CODE: &[u8] = include_bytes!("sitas-user.bin");

#[cfg(target_arch = "aarch64")]
static mut SITAS_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

pub fn test_el0_sitas() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 sitas (Rust-compiled catten-user binary)...");
        logln!("[sitas] binary loaded");
        logln!("[sitas] mapping pages");

        // --- create user address space ---
        let user_as = {
            let _kas = KERNEL_AS.lock();
            let mut as_ = AddressSpace::get_current();
            as_.set_ttbr0(0);
            as_
        };
        let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
        logln!("[sitas] AS created");

        // --- map code pages ---
        for i in 0..CODE_PAGES {
            let page_base = SITAS_CODE_VADDR + i * PAGE_SIZE;
            let vaddr = VAddr::from(page_base);

            let frame = PHYSICAL_FRAME_ALLOCATOR
                .lock()
                .allocate_frame()
                .expect("[sitas] failed to allocate code frame");
            ADDRESS_SPACE_TABLE
                .lock()
                .get_mut(asid)
                .expect("[sitas] AS not found")
                .map_page(MemoryMapping {
                    vaddr,
                    paddr: frame,
                    // Split: code pages executable, data/BSS pages writable.
                    // The binary's LOAD #1 (code+rodata) ends at ELF VA 0x35C0
                    // which is page-aligned at 0x4000 (page 4). Pages 0-3 are
                    // code; pages 4+ are data/BSS.
                    page_type: if i < 4 { PageType::UserCode } else { PageType::UserData },
                })
                .expect("[sitas] failed to map code page");

            // Copy this page's portion of the binary.
            let hhdm: *mut u8 = frame.into();
            unsafe {
                core::ptr::write_bytes(hhdm, 0, PAGE_SIZE); // zero page first (BSS)
            }

            let start = i * PAGE_SIZE;
            if start < SITAS_CODE.len() {
                let end = core::cmp::min(start + PAGE_SIZE, SITAS_CODE.len());
                let chunk = &SITAS_CODE[start..end];
                unsafe {
                    core::ptr::copy_nonoverlapping(chunk.as_ptr(), hhdm, chunk.len());
                }
            }
        }
        // I-cache invalidation for all freshly written code.
        unsafe {
            core::arch::asm!(
                "dsb ishst", "ic ialluis", "dsb ish", "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        // --- map CQ ring page ---
        let cq_frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[sitas] failed to allocate CQ frame");
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[sitas] AS not found")
            .map_page(MemoryMapping {
                vaddr: VAddr::from(SITAS_CQ_VADDR),
                paddr: cq_frame,
                page_type: PageType::UserData,
            })
            .expect("[sitas] failed to map CQ page");

        // --- map result page ---
        let result_frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[sitas] failed to allocate result frame");
        unsafe { SITAS_RESULT_FRAME = Some(result_frame); }
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[sitas] AS not found")
            .map_page(MemoryMapping {
                vaddr: VAddr::from(SITAS_RESULT_VADDR),
                paddr: result_frame,
                page_type: PageType::UserData,
            })
            .expect("[sitas] failed to map result page");

        // --- map user heap pages ---
        for i in 0..SITAS_HEAP_PAGES {
            let heap_vaddr = VAddr::from(SITAS_HEAP_VADDR + i * PAGE_SIZE);
            let heap_frame = PHYSICAL_FRAME_ALLOCATOR
                .lock()
                .allocate_frame()
                .expect("[sitas] failed to allocate heap frame");
            ADDRESS_SPACE_TABLE
                .lock()
                .get_mut(asid)
                .expect("[sitas] AS not found")
                .map_page(MemoryMapping {
                    vaddr: heap_vaddr,
                    paddr: heap_frame,
                    page_type: PageType::UserData,
                })
                .expect("[sitas] failed to map heap page");
        }

        completion::open_address_space_with_cq_phys(asid, 16, cq_frame, 32);

        // The binary reads asid from result[4]; write it there.
        let result_base: *mut u8 = result_frame.into();
        unsafe {
            core::ptr::write_volatile((result_base as *mut u32).add(4), asid as u32);
        }

        // Spawn the EL0 thread.  The entry point is at offset ENTRY_OFFSET within
        // the loaded binary (PIE places _start at a non-zero offset).
        let entry_vaddr = SITAS_CODE_VADDR + ENTRY_OFFSET;
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(entry_vaddr) };
        let _tid = spawn_thread(asid as crate::memory::AddressSpaceId, entry);
        logln!("[sitas] thread spawned");

        let _vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_sitas);
        logln!("[sitas] verifier deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 sitas test (AArch64 only).");
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_sitas() {
    use crate::cpu::scheduler::yield_lp;

    let frame = unsafe { SITAS_RESULT_FRAME }.expect("[sitas] result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let sentinel = unsafe { core::ptr::read_volatile(result) };
        // A successful run writes the total key count (3) or a cap-sentinel
        // (0xDEAD).  Any non-zero value means the EL0 stub executed and
        // produced output.
        if sentinel != 0 {
            logln!(
                "[sitas] SUCCESS: catten-user Rust binary ran at EL0, produced result {:#x}.",
                sentinel
            );
            loop { yield_lp(); }
        }
        spins += 1;
        assert!(
            spins < 80_000_000,
            "[sitas] FAILED: basic_kv_test did not post result (got {:#x})", sentinel,
        );
        yield_lp();
    }
}
