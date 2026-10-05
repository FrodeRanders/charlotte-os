use crate::{
    cpu::isa::{
        interface::memory::{
            AddressSpaceInterface,
            address::VirtualAddress,
        },
        memory::paging::AddressSpace,
    },
    logln,
    memory::{
        PHYSICAL_FRAME_ALLOCATOR,
        linear::{
            MemoryMapping,
            PageType,
            VAddr,
        },
    },
};

pub fn test_vmem() {
    #[cfg(target_arch = "x86_64")]
    {
        crate::cpu::isa::memory::paging::pte::PageTableEntry::self_test_pat_encoding();
        AddressSpace::self_test_root_preparation();
        logln!("x86-64 PAT page-table encoding tests passed.");
    }

    test_retained_tables();

    logln!("Entering Virtual Memory Subsystem Self Test");
    logln!("Allocating physical frame");
    let frame = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().unwrap();
    logln!("Physical frame allocated");
    logln!("Obtaining current address space");
    let mut current_as = AddressSpace::get_current();
    logln!("Obtained current address space.");
    logln!("Creating MemoryMapping struct.");
    let higher_half_start: VAddr = VAddr::from(0xffff_ffff_ffff_f000usize);
    let mapping = MemoryMapping {
        vaddr: higher_half_start,
        paddr: frame,
        page_type: PageType::KernelData,
    };
    logln!(
        "Created MemoryMapping struct... Mapping the allocated frame to the beginning of the \
         higher half."
    );
    match current_as.map_page(mapping) {
        Ok(_) => logln!("Page mapped successfully."),
        Err(e) => panic!("Error mapping page: {:?}", e),
    }
    let addr: *mut u32 = higher_half_start.into_mut();
    const MAGIC_NUMBER: u32 = 0xcafebabe;
    unsafe {
        logln!(
            "Writing magic number {:x?}_16 to virtual address {:?}",
            MAGIC_NUMBER,
            higher_half_start
        );
        addr.write(MAGIC_NUMBER);
        logln!("Reading magic number back from {:?}", higher_half_start);
        let read_value = addr.read();
        assert_eq!(read_value, MAGIC_NUMBER);
        logln!("Magic number matches.");
        logln!("Test completed successfully.");
        logln!("Unmapping test page.");
        let unmapped = current_as.unmap_page(higher_half_start).expect("Error unmapping page.");
        assert_eq!(unmapped, frame);
        // This boot fixture runs before secondary LP scheduler admission.
        crate::cpu::isa::memory::tlb::inval_range_kernel(higher_half_start, 1);
        PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(unmapped).unwrap();
        logln!("Test page successfully unmapped.");
        logln!("All virtual memory tests passed!");
    }
}

/// These private roots are never installed on any LP. The fixture checks
/// ownership and reuse, not concurrent hardware walk/shootdown correctness.
fn test_retained_tables() {
    use crate::memory::PreparingUserFrame;

    let baseline = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let frame = PreparingUserFrame::allocate_zeroed().expect("table fixture backing");
    let with_backing = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut first = AddressSpace::try_new_user().expect("table fixture first root");
    let mut second = AddressSpace::try_new_user().expect("table fixture second root");
    // Three leaf tables and two next-level tables under one first-level table:
    // seven frames per private hierarchy, including its root on either ISA.
    let addresses = [0x2000_0000usize, 0x2020_0000, 0x6000_0000];
    for address_space in [&mut first, &mut second] {
        for raw in addresses {
            address_space
                .map_existing_page(MemoryMapping {
                    vaddr: VAddr::from(raw),
                    paddr: frame.frame(),
                    page_type: PageType::UserData,
                })
                .expect("sparse shared-data mapping");
        }
    }
    let retained = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(with_backing - retained, 14);
    assert!(
        first
            .map_existing_page(MemoryMapping {
                vaddr: VAddr::from(addresses[0]),
                paddr: frame.frame(),
                page_type: PageType::UserData,
            })
            .is_err()
    );
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), retained);
    for round in 0..16 {
        for raw in addresses {
            let vaddr = VAddr::from(raw);
            assert_eq!(first.unmap_page(vaddr).unwrap(), frame.frame());
            assert!(!first.is_mapped(vaddr).unwrap());
            assert!(first.unmap_page(vaddr).is_err());
            assert_eq!(second.translate_address(vaddr).unwrap(), frame.frame());
        }
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), retained);
        // Empty tables are retained, not silently promoted to a block mapping.
        assert!(
            first
                .map_large_page(MemoryMapping {
                    vaddr: VAddr::from(addresses[0]),
                    paddr: frame.frame(),
                    page_type: PageType::UserData,
                })
                .is_err()
        );
        if round != 15 {
            for raw in addresses {
                first
                    .map_existing_page(MemoryMapping {
                        vaddr: VAddr::from(raw),
                        paddr: frame.frame(),
                        page_type: PageType::UserData,
                    })
                    .unwrap();
            }
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), retained);
        }
    }
    drop(first);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), retained + 7);
    for raw in addresses {
        assert_eq!(second.translate_address(VAddr::from(raw)).unwrap(), frame.frame());
        second.unmap_page(VAddr::from(raw)).unwrap();
    }
    drop(second);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), with_backing);
    drop(frame);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), baseline);
    logln!(
        "[table lifetime] sparse aliases, 16 unmap/remap rounds, private-tree isolation, retained \
         high-water and teardown refunds passed"
    );
}
