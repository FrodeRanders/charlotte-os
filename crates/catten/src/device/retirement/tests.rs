//! Raw capability IDs here are kernel ABI fixtures inspecting exact cleanup
//! ownership. Rejected receipts retain their real root, claims and backing;
//! there is no test-only recovery path or simulated hardware acknowledgement.

use super::*;
use crate::{
    memory::{
        self,
        AddressSpaceCloseError,
        backing_budget,
        object::{
            self,
            MemoryObjectError,
        },
        retirement::CloseProgress,
    },
    service::loader,
};

fn unlocked() {
    assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(DEVICES.try_lock().is_some());
}

fn present(handle: AddressSpaceHandle, cap: DeviceCap) -> bool {
    crate::capability::contains(handle.id(), cap, ObjectKind::Device)
}

fn translated(handle: AddressSpaceHandle, base: VAddr) -> Option<PAddr> {
    memory::ADDRESS_SPACE_TABLE.lock().get_mut(handle.id()).unwrap().translate_address(base).ok()
}

pub(crate) fn run() {
    successful_cleanup();
    preparation_failure();
    for failure in
        [Failure::Detach, Failure::Invalidate, Failure::Scratch, Failure::Dma, Failure::Abandon]
    {
        failed_cleanup(failure);
    }
    crate::logln!(
        "[device retirement] unlocked MMIO/DMA completion, exact leaf ownership, partial failure, \
         preparation rejection and abandonment passed; six closing roots retained"
    );
}

fn successful_cleanup() {
    let handle = loader::create_user_address_space_handle();
    let cap = grant_mmio(handle.id(), 0x0900_0000, 2).unwrap();
    let base = mmio_map_any(handle.id(), cap, true).unwrap();
    let direct = grant_mmio(handle.id(), 0x0900_2000, 1).unwrap();
    let direct_base = VAddr::from(0x3000_0000usize);
    mmio_map(handle.id(), direct, direct_base, false).unwrap();
    let irq = grant_interrupt(handle.id(), 225).unwrap();
    let domain = grant_dma_domain_with_backend(
        handle.id(),
        |creation| {
            creation.record(u64::MAX);
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap();
    let mut closing = ClosingAddressSpace::begin_ready(handle).unwrap();
    let mut unmapped = 0;
    let mut invalidated = 0;
    let mut destroyed = false;
    assert!(
        closing
            .start_cleanup_with_devices(|receipt| {
                unlocked();
                assert!(present(handle, cap) && present(handle, domain));
                assert!(!DEVICES.lock().contains_key(&handle.id()));
                assert_eq!(
                    grant_mmio(handle.id(), 0x0900_0000, 1),
                    Err(DeviceError::NamespaceRetired)
                );
                receipt.finish_with(
                    |root, base, frame| {
                        unlocked();
                        unmapped += 1;
                        unmap_owned_mmio(root, base, frame)
                    },
                    |root, base, pages| {
                        unlocked();
                        assert_eq!(memory::current_address_space_handle(root.id()), Some(handle));
                        assert_eq!(translated(root, base), None);
                        crate::cpu::isa::memory::tlb::inval_range_user(root.id(), base, pages);
                        invalidated += 1;
                        true
                    },
                    |asid, base, pages| {
                        unlocked();
                        object::release_scratch(asid, base, pages)
                    },
                    |id| {
                        unlocked();
                        assert_eq!(id, u64::MAX);
                        destroyed = true;
                        Ok(())
                    },
                )
            })
            .unwrap()
    );
    assert_eq!(unmapped, 3);
    assert_eq!(invalidated, 2);
    assert!(destroyed);
    assert!(
        !present(handle, cap)
            && !present(handle, direct)
            && !present(handle, irq)
            && !present(handle, domain)
    );
    assert_eq!(translated(handle, base), None);
    assert_eq!(translated(handle, direct_base), None);
    // Only confirmed MMIO cleanup makes its exact scratch extent reusable.
    let reused = object::reserve_scratch(handle.id(), 2).unwrap();
    assert_eq!(reused, base);
    object::release_scratch(handle.id(), reused, 2).unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    let fresh = loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let replacement = grant_mmio(fresh.id(), 0x0900_0000, 1).unwrap();
    assert!(unmap_owned_mmio(handle, base, PAddr::from(0x0900_0000u64)).is_err());
    assert!(present(fresh, replacement));
    memory::close_user_address_space_handle(fresh).unwrap();
}

fn preparation_failure() {
    let handle = loader::create_user_address_space_handle();
    let cap = grant_mmio(handle.id(), 0x0900_0000, 1).unwrap();
    // Deliberately manufacture an abandoned operation claim at this ABI
    // boundary; public MMIO operations also retain an address-space lease.
    drop(MmioOperation::begin(handle.id(), cap).unwrap());
    let mut closing = ClosingAddressSpace::begin_ready(handle).unwrap();
    assert_eq!(
        closing.start_cleanup_with_devices(|_| panic!("failed prepare reached hardware")),
        Err(AddressSpaceCloseError::DeviceCleanupFailed)
    );
    assert!(present(handle, cap));
    assert!(DEVICES.lock().contains_key(&handle.id()));
    assert_eq!(closing.start_cleanup(), Err(AddressSpaceCloseError::DeviceCleanupFailed));
    drop(closing);
    assert_eq!(memory::current_address_space_handle(handle.id()), Some(handle));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    Detach,
    Invalidate,
    Scratch,
    Dma,
    Abandon,
}

fn failed_cleanup(failure: Failure) {
    let before = backing_budget::test_used_pages(backing_budget::Kind::Heap);
    let handle = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(handle, charlotte_launch::HEAP_VADDR));
    let first = grant_mmio(handle.id(), 0x0900_0000, 1).unwrap();
    let cap = grant_mmio(handle.id(), 0x0900_0000, 2).unwrap();
    let base = mmio_map_any(handle.id(), cap, true).unwrap();
    let domain = grant_dma_domain_with_backend(
        handle.id(),
        |creation| {
            creation.record(u64::MAX);
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap();
    // Foreign leaf identity must be checked before removal. Keep the root
    // quarantined so neither this replacement nor its mapping record is reused.
    if failure == Failure::Detach {
        arch_unmap(handle.id(), base + PAGE_SIZE).unwrap();
        arch_map_user_mmio(handle.id(), base + PAGE_SIZE, PAddr::from(0x0900_3000u64), true)
            .unwrap();
    }
    let free = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut closing = ClosingAddressSpace::begin_ready(handle).unwrap();
    let mut destroyed = false;
    let mut released = false;
    assert_eq!(
        closing.start_cleanup_with_devices(|receipt| {
            unlocked();
            if failure == Failure::Abandon {
                drop(receipt);
                return Err(DeviceError::UnmapFailed);
            }
            receipt.finish_with(
                unmap_owned_mmio,
                |root, base, pages| {
                    unlocked();
                    if failure == Failure::Invalidate {
                        return false;
                    }
                    crate::cpu::isa::memory::tlb::inval_range_user(root.id(), base, pages);
                    true
                },
                |asid, base, pages| {
                    unlocked();
                    released = true;
                    if failure == Failure::Scratch {
                        return Err(MemoryObjectError::OutOfScratch);
                    }
                    object::release_scratch(asid, base, pages)
                },
                |_| {
                    unlocked();
                    destroyed = true;
                    Err(dma::Error::Unsupported)
                },
            )
        }),
        Err(AddressSpaceCloseError::DeviceCleanupFailed)
    );
    assert_eq!(destroyed, failure == Failure::Dma);
    assert_eq!(released, matches!(failure, Failure::Scratch | Failure::Dma));
    assert_eq!(present(handle, first), failure == Failure::Abandon);
    assert_eq!(present(handle, cap), failure != Failure::Dma);
    assert!(present(handle, domain));
    assert!(!DEVICES.lock().contains_key(&handle.id()));
    if failure == Failure::Detach {
        assert_eq!(translated(handle, base), None);
        assert_eq!(translated(handle, base + PAGE_SIZE), Some(PAddr::from(0x0900_3000u64)));
    }
    if failure != Failure::Dma {
        let other = object::reserve_scratch(handle.id(), 2).unwrap();
        assert_ne!(other, base, "uncertain MMIO scratch claim was recycled");
        object::release_scratch(handle.id(), other, 2).unwrap();
    }
    assert_eq!(closing.start_cleanup(), Err(AddressSpaceCloseError::DeviceCleanupFailed));
    assert_eq!(closing.poll().err(), Some(AddressSpaceCloseError::DeviceCleanupFailed));
    assert_eq!(memory::current_address_space_handle(handle.id()), Some(handle));
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_budget::test_used_pages(backing_budget::Kind::Heap), before + 1);
    assert_eq!(
        memory::close_user_address_space_handle(handle),
        Err(AddressSpaceCloseError::CloseInProgress)
    );
    let other = loader::create_user_address_space_handle();
    assert_ne!(other.id(), handle.id());
    memory::close_user_address_space_handle(other).unwrap();
}

/// Real MMIO detach/invalidation after secondary LPs start. Roots carry no
/// application threads and device register contents are never accessed.
pub(crate) fn run_runtime() {
    let handle = loader::create_user_address_space_handle();
    for (phys, scratch) in [(0x0900_0000, true), (0x0900_2000, false)] {
        let cap = grant_mmio(handle.id(), phys, 2).unwrap();
        if scratch {
            mmio_map_any(handle.id(), cap, true).unwrap();
        } else {
            mmio_map(handle.id(), cap, VAddr::from(0x3000_0000usize), false).unwrap();
        }
    }
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(memory::current_address_space_handle(handle.id()), None);
    crate::logln!(
        "[device retirement runtime] scratch/direct MMIO retired with secondary LPs online"
    );
}
