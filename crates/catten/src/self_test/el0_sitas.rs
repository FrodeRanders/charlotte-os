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
    linear::{
        MemoryMapping,
        PageType,
        VAddr,
    },
    ADDRESS_SPACE_TABLE,
    KERNEL_AS,
};

// The binary is linked and mapped at 0x20000.
// catten-rt reads the config page at 0x1F000 (ASID at offset 16).
#[cfg(target_arch = "aarch64")]
const SITAS_CODE_VADDR: usize = 0x0000_0000_0002_0000;
#[cfg(target_arch = "aarch64")]
const SITAS_CONFIG_VADDR: usize = 0x0000_0000_0001_0000;
#[cfg(target_arch = "aarch64")]
const SITAS_CQ_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const SITAS_INPUT_VADDR: usize = 0x0000_0000_0001_2000;
#[cfg(target_arch = "aarch64")]
const SITAS_HEAP_VADDR: usize = 0x0000_0000_0001_3000;
#[cfg(target_arch = "aarch64")]
const SITAS_HEAP_PAGES: usize = 13;

#[cfg(target_arch = "aarch64")]
const PAGE_SIZE: usize = 4096;
#[cfg(target_arch = "aarch64")]
const CONFIG_ASID_OFFSET: usize = 16;
#[cfg(target_arch = "aarch64")]
const CONFIG_ARGC_OFFSET: usize = 24;
#[cfg(target_arch = "aarch64")]
const CONFIG_ARGS_OFFSET: usize = 32;

/// Offset of `_start` within the raw binary. The linker script places the
/// image at VA 0x20000 but keeps `_start` first, so raw offset 0 maps to the
/// entry virtual address.
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
                    // Raw flat images can place code and data in the same 4 KiB
                    // page. Until this becomes an ELF loader, these pages must
                    // be both writable and executable.
                    page_type: PageType::UserFlatImage,
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
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        // --- map config page (catten-rt reads ASID from offset 16) ---
        let config_frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[sitas] failed to allocate config frame");
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[sitas] AS not found")
            .map_page(MemoryMapping {
                vaddr: VAddr::from(SITAS_CONFIG_VADDR),
                paddr: config_frame,
                page_type: PageType::UserData,
            })
            .expect("[sitas] failed to map config page");

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

        // --- map launch input page ---
        let input_frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[sitas] failed to allocate input frame");
        unsafe {
            SITAS_RESULT_FRAME = Some(config_frame); // verifier polls config page
        }
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[sitas] AS not found")
            .map_page(MemoryMapping {
                vaddr: VAddr::from(SITAS_INPUT_VADDR),
                paddr: input_frame,
                page_type: PageType::UserData,
            })
            .expect("[sitas] failed to map input page");

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

        // catten-rt reads launch metadata from the config page. The adder's
        // cmain receives Args([42, 7]) and Input<32>; crt0 reads exactly 32
        // bytes into SITAS_INPUT_VADDR before calling cmain.
        let config_base: *mut u8 = config_frame.into();
        unsafe {
            core::ptr::write_volatile(config_base.add(CONFIG_ASID_OFFSET) as *mut usize, asid);
            core::ptr::write_volatile(config_base.add(CONFIG_ARGC_OFFSET) as *mut usize, 2);
            core::ptr::write_volatile(config_base.add(CONFIG_ARGS_OFFSET) as *mut u32, 42);
            core::ptr::write_volatile(config_base.add(CONFIG_ARGS_OFFSET + 4) as *mut u32, 7);
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
        if sentinel == 0xc0de {
            let sum = unsafe { core::ptr::read_volatile(result.add(2)) }; // computed
            let expected = 42u32.wrapping_add(7).wrapping_add(0xfeed_f00d);
            assert_eq!(
                sum, expected,
                "[sitas] adder: expected sum 42+7+0xFEED_F00D = {:#x}, got {:#x}",
                expected, sum
            );
            logln!("[sitas] SUCCESS: adder program computed the correct sum.");
            loop {
                yield_lp();
            }
        }
        if sentinel != 0 && sentinel != 0xc0de {
            // basic_kv or minimal stub: any non-zero sentinel = ran successfully.
            logln!(
                "[sitas] SUCCESS: catten-user Rust binary ran at EL0, produced result {:#x}.",
                sentinel
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 80_000_000,
            "[sitas] FAILED: basic_kv_test did not post result (got {:#x})",
            sentinel,
        );
        yield_lp();
    }
}
