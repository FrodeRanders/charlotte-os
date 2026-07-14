//! ELF loading and standard runtime-page setup for EL0 service domains.
//!
//! Generalized from the `el0_sitas` smoke-test loader so every spawned
//! service domain shares one loading path: `PT_LOAD` segments are mapped at
//! their linked virtual addresses with page permissions derived from ELF
//! flags, and the canonical `catten-rt` runtime pages (config, input, heap)
//! are mapped and zeroed.
#![cfg(target_arch = "aarch64")]

use crate::{
    cpu::isa::{
        interface::memory::AddressSpaceInterface,
        memory::paging::AddressSpace,
    },
    memory::{
        ADDRESS_SPACE_TABLE,
        AddressSpaceId,
        KERNEL_AS,
        PHYSICAL_FRAME_ALLOCATOR,
        linear::{
            MemoryMapping,
            PageType,
            VAddr,
        },
        physical::PAddr,
    },
};

/// The canonical config-page virtual address (`catten-rt` contract).
pub const CONFIG_VADDR: usize = 0x0000_0000_0001_0000;
/// Canonical launch input buffer virtual address.
pub const INPUT_VADDR: usize = 0x0000_0000_0001_2000;
/// Canonical user heap base (`catten-rt`'s allocator arena).
pub const HEAP_VADDR: usize = 0x0000_0000_0001_3000;
/// Number of heap pages backing the `catten-rt` allocator arena (0xd000).
pub const HEAP_PAGES: usize = 13;

pub const PAGE_SIZE: usize = 4096;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_AARCH64: u16 = 0xb7;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

/// A user protection domain prepared by the loader but not yet running.
pub struct LoadedDomain {
    pub asid: AddressSpaceId,
    pub entry_vaddr: usize,
    pub config_frame: PAddr,
}

#[derive(Clone, Copy)]
struct ElfLoadSegment {
    offset: usize,
    vaddr: usize,
    filesz: usize,
    memsz: usize,
    flags: u32,
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

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

fn align_down(value: usize, align: usize) -> usize {
    value & !(align - 1)
}

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

fn segment_page_type(flags: u32) -> PageType {
    let executable = flags & PF_X != 0;
    let writable = flags & PF_W != 0;

    match (executable, writable) {
        (true, false) => PageType::UserCode,
        (false, true) => PageType::UserData,
        (false, false) => PageType::UserRoData,
        (true, true) => panic!("[loader] ELF LOAD segment requests writable executable memory"),
    }
}

fn parse_elf_header(image: &[u8]) -> (usize, usize, usize, usize) {
    assert!(image.len() >= 64, "[loader] ELF image is too small");
    assert_eq!(&image[0..4], ELF_MAGIC, "[loader] invalid ELF magic");
    assert_eq!(image[4], ELFCLASS64, "[loader] ELF image is not 64-bit");
    assert_eq!(image[5], ELFDATA2LSB, "[loader] ELF image is not little-endian");
    assert_eq!(read_u16_le(image, 16), ET_EXEC, "[loader] ELF image must be ET_EXEC");
    assert_eq!(read_u16_le(image, 18), EM_AARCH64, "[loader] ELF image is not AArch64");

    let entry = read_u64_le(image, 24) as usize;
    let phoff = read_u64_le(image, 32) as usize;
    let phentsize = read_u16_le(image, 54) as usize;
    let phnum = read_u16_le(image, 56) as usize;
    assert_eq!(phentsize, 56, "[loader] unexpected ELF64 program-header size");
    assert!(
        phoff + phentsize * phnum <= image.len(),
        "[loader] ELF program-header table exceeds image"
    );

    (entry, phoff, phentsize, phnum)
}

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

    assert!(filesz <= memsz, "[loader] ELF LOAD filesz exceeds memsz");
    assert!(file_offset + filesz <= image.len(), "[loader] ELF LOAD file range exceeds image");
    assert!(
        file_offset & (PAGE_SIZE - 1) == vaddr & (PAGE_SIZE - 1),
        "[loader] ELF LOAD file and virtual offsets are not page-congruent"
    );

    Some(ElfLoadSegment {
        offset: file_offset,
        vaddr,
        filesz,
        memsz,
        flags,
    })
}

fn map_elf_load_segment(asid: AddressSpaceId, image: &[u8], segment: ElfLoadSegment) {
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
            let as_ = table.get_mut(asid).expect("[loader] AS not found");
            assert!(
                !as_.is_mapped(vaddr).expect("[loader] failed to query mapped page"),
                "[loader] ELF LOAD segments overlap within one page; relink with page-separated \
                 segments"
            );
        }

        let frame = PHYSICAL_FRAME_ALLOCATOR
            .lock()
            .allocate_frame()
            .expect("[loader] failed to allocate ELF LOAD frame");
        ADDRESS_SPACE_TABLE
            .lock()
            .get_mut(asid)
            .expect("[loader] AS not found")
            .map_page(MemoryMapping {
                vaddr,
                paddr: frame,
                page_type,
            })
            .expect("[loader] failed to map ELF LOAD page");

        let hhdm: *mut u8 = frame.into();
        unsafe {
            core::ptr::write_bytes(hhdm, 0, PAGE_SIZE);
        }
        let copy_start = core::cmp::max(page_base, seg_start);
        let copy_end = core::cmp::min(page_base + PAGE_SIZE, file_end);
        if copy_start < copy_end {
            let src_offset = segment.offset + (copy_start - segment.vaddr);
            let dst_offset = copy_start - page_base;
            let len = copy_end - copy_start;
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

/// Create a fresh user address space.
pub fn create_user_address_space() -> AddressSpaceId {
    let user_as = {
        let _kas = KERNEL_AS.lock();
        let mut as_ = AddressSpace::get_current();
        as_.set_ttbr0(0);
        as_
    };
    ADDRESS_SPACE_TABLE.lock().add_element(user_as)
}

/// Map all `PT_LOAD` segments of `image` into `asid` and return the entry
/// virtual address.
pub fn load_user_elf(asid: AddressSpaceId, image: &[u8]) -> usize {
    let (entry, phoff, phentsize, phnum) = parse_elf_header(image);
    let mut load_segments = 0usize;

    for i in 0..phnum {
        let ph_offset = phoff + i * phentsize;
        if let Some(segment) = parse_load_segment(image, ph_offset) {
            map_elf_load_segment(asid, image, segment);
            load_segments += 1;
        }
    }
    assert!(load_segments > 0, "[loader] ELF image has no LOAD segments");

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

/// Map one zeroed `UserData` page at `vaddr` and return its backing frame.
pub fn map_user_data_page(asid: AddressSpaceId, vaddr: usize) -> PAddr {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("[loader] failed to allocate user data frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("[loader] AS not found")
        .map_page(MemoryMapping {
            vaddr: VAddr::from(vaddr),
            paddr: frame,
            page_type: PageType::UserData,
        })
        .expect("[loader] failed to map user data page");
    let hhdm: *mut u8 = frame.into();
    unsafe {
        core::ptr::write_bytes(hhdm, 0, PAGE_SIZE);
    }
    frame
}

/// Create an address space, load `image`, and map the standard `catten-rt`
/// runtime pages (config, input, heap). The domain is not started.
pub fn load_domain(image: &[u8]) -> LoadedDomain {
    let asid = create_user_address_space();
    let entry_vaddr = load_user_elf(asid, image);

    let config_frame = map_user_data_page(asid, CONFIG_VADDR);
    let _input_frame = map_user_data_page(asid, INPUT_VADDR);
    for i in 0..HEAP_PAGES {
        let _ = map_user_data_page(asid, HEAP_VADDR + i * PAGE_SIZE);
    }

    LoadedDomain {
        asid,
        entry_vaddr,
        config_frame,
    }
}
