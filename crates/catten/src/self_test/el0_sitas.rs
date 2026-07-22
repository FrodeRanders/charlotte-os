//! Self-test: load the Rust-compiled sitas-based catten-user binary at EL0.
//!
//! Unlike the hand-written stub in [`super::el0`], this test embeds the full
//! ELF produced by `cargo +nightly build -p catten-user --target …`
//! (which links against sitas-core and sitas-charlotte).  The kernel maps
//! PT_LOAD segments at their linked virtual addresses with page permissions
//! derived from ELF flags.
//!
//! The binary calls `basic_kv::basic_kv_test`, which exercises `ShardedKv`
//! over `CharlotteReactor`: it creates a KV store, puts keys, reads one back,
//! and writes the total key count to the result page.

#[cfg(target_arch = "aarch64")]
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

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
    ADDRESS_SPACE_TABLE,
    KERNEL_AS,
    linear::{
        MemoryMapping,
        PageType,
        VAddr,
    },
};

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
const SITAS_STATUS_VADDR: usize = charlotte_launch::STATUS_VADDR;

#[cfg(target_arch = "aarch64")]
const PAGE_SIZE: usize = 4096;
#[cfg(target_arch = "aarch64")]

/// The Rust-compiled sitas-based catten-user ELF.
#[cfg(target_arch = "aarch64")]
const SITAS_ELF: &[u8] = include_bytes!("sitas-user.elf");

#[cfg(target_arch = "aarch64")]
static mut SITAS_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;
#[cfg(target_arch = "aarch64")]
static SITAS_ASID: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(target_arch = "aarch64")]
const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
#[cfg(target_arch = "aarch64")]
const ELFCLASS64: u8 = 2;
#[cfg(target_arch = "aarch64")]
const ELFDATA2LSB: u8 = 1;
#[cfg(target_arch = "aarch64")]
const ET_EXEC: u16 = 2;
#[cfg(target_arch = "aarch64")]
const EM_AARCH64: u16 = 0xb7;
#[cfg(target_arch = "aarch64")]
const PT_LOAD: u32 = 1;
#[cfg(target_arch = "aarch64")]
const PF_X: u32 = 1;
#[cfg(target_arch = "aarch64")]
const PF_W: u32 = 2;

#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
struct ElfLoadSegment {
    offset: usize,
    vaddr: usize,
    filesz: usize,
    memsz: usize,
    flags: u32,
}

#[cfg(target_arch = "aarch64")]
fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

#[cfg(target_arch = "aarch64")]
fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

#[cfg(target_arch = "aarch64")]
fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(target_arch = "aarch64")]
fn align_down(value: usize, align: usize) -> usize {
    value & !(align - 1)
}

#[cfg(target_arch = "aarch64")]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[cfg(target_arch = "aarch64")]
fn segment_page_type(flags: u32) -> PageType {
    let executable = flags & PF_X != 0;
    let writable = flags & PF_W != 0;

    match (executable, writable) {
        (true, false) => PageType::UserCode,
        (false, true) => PageType::UserData,
        (false, false) => PageType::UserRoData,
        (true, true) => panic!("[sitas] ELF LOAD segment requests writable executable memory"),
    }
}

#[cfg(target_arch = "aarch64")]
fn parse_elf_header(image: &[u8]) -> (usize, usize, usize, usize) {
    assert!(image.len() >= 64, "[sitas] ELF image is too small");
    assert_eq!(&image[0..4], ELF_MAGIC, "[sitas] invalid ELF magic");
    assert_eq!(image[4], ELFCLASS64, "[sitas] ELF image is not 64-bit");
    assert_eq!(image[5], ELFDATA2LSB, "[sitas] ELF image is not little-endian");
    assert_eq!(read_u16_le(image, 16), ET_EXEC, "[sitas] ELF image must be ET_EXEC");
    assert_eq!(read_u16_le(image, 18), EM_AARCH64, "[sitas] ELF image is not AArch64");

    let entry = read_u64_le(image, 24) as usize;
    let phoff = read_u64_le(image, 32) as usize;
    let phentsize = read_u16_le(image, 54) as usize;
    let phnum = read_u16_le(image, 56) as usize;
    assert_eq!(phentsize, 56, "[sitas] unexpected ELF64 program-header size");
    assert!(
        phoff + phentsize * phnum <= image.len(),
        "[sitas] ELF program-header table exceeds image"
    );

    (entry, phoff, phentsize, phnum)
}

#[cfg(target_arch = "aarch64")]
fn parse_load_segment(image: &[u8], offset: usize) -> Option<ElfLoadSegment> {
    let p_type = read_u32_le(image, offset);
    if p_type != PT_LOAD {
        return None;
    }

    let flags = read_u32_le(image, offset + 4);
    let file_offset = read_u64_le(image, offset + 8) as usize;
    let vaddr = read_u64_le(image, offset + 16) as usize;
    let filesz = read_u64_le(image, offset + 32) as usize;
    let memsz = read_u64_le(image, offset + 40) as usize;

    assert!(filesz <= memsz, "[sitas] ELF LOAD filesz exceeds memsz");
    assert!(file_offset + filesz <= image.len(), "[sitas] ELF LOAD file range exceeds image");
    assert!(
        file_offset & (PAGE_SIZE - 1) == vaddr & (PAGE_SIZE - 1),
        "[sitas] ELF LOAD file and virtual offsets are not page-congruent"
    );

    Some(ElfLoadSegment {
        offset: file_offset,
        vaddr,
        filesz,
        memsz,
        flags,
    })
}

#[cfg(target_arch = "aarch64")]
fn map_elf_load_segment(asid: usize, image: &[u8], segment: ElfLoadSegment) {
    if segment.memsz == 0 {
        return;
    }

    let page_type = segment_page_type(segment.flags);
    let seg_start = segment.vaddr;
    let mem_end = segment.vaddr + segment.memsz;
    let file_end = segment.vaddr + segment.filesz;
    let map_start = align_down(seg_start, PAGE_SIZE);
    let map_end = align_up(mem_end, PAGE_SIZE);

    for page_base in (map_start..map_end).step_by(PAGE_SIZE) {
        let vaddr = VAddr::from(page_base);
        {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            let as_ = table.get_mut(asid).expect("[sitas] AS not found");
            assert!(
                !as_.is_mapped(vaddr).expect("[sitas] failed to query mapped page"),
                "[sitas] ELF LOAD segments overlap within one page; relink with page-separated \
                 segments"
            );
        }

        let frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[sitas] failed to allocate ELF LOAD frame");
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[sitas] AS not found")
            .map_page(MemoryMapping {
                vaddr,
                paddr: frame,
                page_type,
            })
            .expect("[sitas] failed to map ELF LOAD page");

        let copy_start = core::cmp::max(page_base, seg_start);
        let copy_end = core::cmp::min(page_base + PAGE_SIZE, file_end);
        if copy_start < copy_end {
            let src_offset = segment.offset + (copy_start - segment.vaddr);
            let dst_offset = copy_start - page_base;
            let len = copy_end - copy_start;
            let hhdm: *mut u8 = frame.into();
            unsafe {
                core::ptr::copy_nonoverlapping(
                    image.as_ptr().add(src_offset),
                    hhdm.add(dst_offset),
                    len,
                );
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn load_user_elf(asid: usize, image: &[u8]) -> usize {
    let (entry, phoff, phentsize, phnum) = parse_elf_header(image);
    let mut load_segments = 0usize;

    for i in 0..phnum {
        let ph_offset = phoff + i * phentsize;
        if let Some(segment) = parse_load_segment(image, ph_offset) {
            map_elf_load_segment(asid, image, segment);
            load_segments += 1;
        }
    }
    assert!(load_segments > 0, "[sitas] ELF image has no LOAD segments");

    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "ic ialluis",
            "dsb ish",
            "isb",
            options(nomem, nostack, preserves_flags),
        );
    }

    entry
}

pub fn test_el0_sitas() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 sitas (Rust-compiled catten-user binary)...");

        // --- create user address space ---
        let user_as = {
            let _kas = KERNEL_AS.lock();
            let mut as_ = AddressSpace::get_current();
            as_.set_ttbr0(0);
            as_
        };
        let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
        SITAS_ASID.store(asid, Ordering::Release);

        let entry_vaddr = load_user_elf(asid, SITAS_ELF);

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
                page_type: PageType::UserRoData,
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

        // --- map the mutable program status page separately from launch data ---
        let status_frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[sitas] failed to allocate status frame");
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[sitas] AS not found")
            .map_page(MemoryMapping {
                vaddr: VAddr::from(SITAS_STATUS_VADDR),
                paddr: status_frame,
                page_type: PageType::UserData,
            })
            .expect("[sitas] failed to map status page");
        let status_base: *mut u8 = status_frame.into();
        unsafe {
            core::ptr::write_bytes(status_base, 0, PAGE_SIZE);
            SITAS_RESULT_FRAME = Some(status_frame);
        }

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

        // `basic_kv` receives an empty launch Context; crt0 therefore enters main
        // without consuming a launch input stream. ASID stays kernel-private.
        crate::service::bootstrap::write_launch_header(config_frame);
        crate::service::bootstrap::write_manifest(config_frame, &[]);

        // Spawn the EL0 thread at the ELF entry point.
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(entry_vaddr) };
        let _tid = spawn_thread(asid as crate::memory::AddressSpaceId, entry);

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
            teardown_sitas_domain();
            return;
        }
        if sentinel != 0 && sentinel != 0xc0de {
            assert_eq!(
                sentinel, 3,
                "[sitas] basic_kv: expected total_len result 3, got {:#x}",
                sentinel
            );
            logln!("[sitas] SUCCESS: basic_kv ran at EL0, produced total_len {:#x}.", sentinel);
            teardown_sitas_domain();
            return;
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

#[cfg(target_arch = "aarch64")]
fn teardown_sitas_domain() {
    let asid = SITAS_ASID.swap(usize::MAX, Ordering::AcqRel);
    if asid != usize::MAX {
        // `basic_kv` spawns pinned no-std shard executors whose raw join
        // handles currently have no shutdown protocol. Once the committed
        // result is verified, terminate every thread in the test domain so
        // those executors do not keep two LPs permanently runnable.
        crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.read().abort_as_threads(asid);
    }
}
