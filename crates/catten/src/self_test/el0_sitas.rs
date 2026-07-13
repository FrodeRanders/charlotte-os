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
#[cfg(target_arch = "aarch64")]
const SITAS_CODE_VADDR: usize = 0x0000_0000_0002_0000;
#[cfg(target_arch = "aarch64")]
const SITAS_CQ_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const SITAS_RESULT_VADDR: usize = 0x0000_0000_0001_2000;

#[cfg(target_arch = "aarch64")]
const PAGE_SIZE: usize = 4096;

/// Offset of `_start` within the raw binary.  Computed from the ELF entry
/// point; must be updated when the binary is rebuilt.
#[cfg(target_arch = "aarch64")]
const ENTRY_OFFSET: usize = 0x1158;

/// The Rust-compiled sitas-based catten-user binary (145 KB, position-independent).
#[cfg(target_arch = "aarch64")]
const SITAS_CODE: &[u8] = include_bytes!("sitas-user.bin");

/// The linker inserts a page-alignment gap at the start of the raw binary
/// before the first LOAD segment.  ELF VA 0 maps to this gap's end.
#[cfg(target_arch = "aarch64")]
const GAP_OFFSET: usize = 0x10000;

/// Total pages to map: the gap plus the code/data content.
#[cfg(target_arch = "aarch64")]
const CODE_PAGES: usize = (GAP_OFFSET + SITAS_CODE.len() + PAGE_SIZE - 1) / PAGE_SIZE;

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
                    page_type: PageType::UserCode,
                })
                .expect("[sitas] failed to map code page");

            // The raw binary has a GAP_OFFSET-byte page-alignment gap before
            // the first LOAD segment.  VA = CODE_VADDR + X corresponds to
            // file offset = GAP_OFFSET + X.
            let file_offs = i * PAGE_SIZE;
            if file_offs >= GAP_OFFSET {
                let code_offs = file_offs - GAP_OFFSET;
                if code_offs < SITAS_CODE.len() {
                    let chunk_start = code_offs;
                    let chunk_end = core::cmp::min(chunk_start + PAGE_SIZE, SITAS_CODE.len());
                    let chunk = &SITAS_CODE[chunk_start..chunk_end];
                    let hhdm: *mut u8 = frame.into();
                    unsafe {
                        core::ptr::copy_nonoverlapping(chunk.as_ptr(), hhdm, chunk.len());
                    }
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
        let tid = spawn_thread(asid as crate::memory::AddressSpaceId, entry);
        logln!("[sitas] thread spawned");

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_sitas);
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
        // basic_kv_test writes the total key count (3 for "alpha"/"beta"/"gamma")
        // OR 0xDEAD on error.  A successful run produces 3.
        if sentinel == 3 {
            logln!(
                "[sitas] SUCCESS: catten-user (sitas-based) ran at EL0, basic_kv_test returned key count {}.",
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
