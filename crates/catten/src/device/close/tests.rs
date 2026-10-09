//! Kernel ABI fixtures use real roots and metadata, never real timeout ACKs.
use super::*;
use crate::{
    capability::admission_tests::test_namespace_used as used,
    memory::{
        self,
        retirement::{
            CloseProgress,
            ClosingAddressSpace,
        },
    },
    service::loader,
};

fn unlocked() {
    assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(DEVICES.try_lock().is_some());
    crate::capability::record_tests::assert_local_available();
}
fn visible(root: memory::AddressSpaceHandle, cap: DeviceCap) -> bool {
    crate::capability::contains(root.id(), cap, crate::capability::ObjectKind::Device)
}

pub(in crate::device) fn run() {
    registry::tests::begin_real();
    for category in 0..3 {
        let root = loader::create_user_address_space_handle();
        let cap = if category == 2 {
            grant_interrupt(root.id(), 225).unwrap()
        } else {
            let cap = grant_mmio(root.id(), 0x0900_0000, 1).unwrap();
            if category == 0 {
                mmio_map_any(root.id(), cap, true).unwrap();
            } else {
                mmio_map(root.id(), cap, VAddr::from(0x3000_0000usize), false).unwrap();
            }
            cap
        };
        let mut owner = PreparedClose::new(root.id(), cap).unwrap();
        {
            let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            owner.claim().unwrap();
            assert_eq!(used(root.id()), 1);
            assert!(!visible(root, cap));
            drop(heap);
        }
        unlocked();
        assert_eq!(
            close_cap(root.id(), cap),
            if category == 2 {
                Err(DeviceError::UnknownCapability)
            } else {
                Err(DeviceError::OperationInFlight)
            },
        );
        let closing = match ClosingAddressSpace::begin(root).unwrap().poll().unwrap() {
            CloseProgress::Pending(owner) => owner,
            CloseProgress::Complete => panic!("device close root completed before metadata"),
        };
        owner.clean().unwrap();
        assert_eq!(used(root.id()), 1, "confirmed cleanup alone cannot refund metadata");
        owner.finish().unwrap();
        assert_eq!(used(root.id()), 0);
        assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    }

    let root = loader::create_user_address_space_handle();
    assert_eq!(close_cap(root.id(), u64::MAX), Err(DeviceError::UnknownCapability));
    memory::close_user_address_space_handle(root).unwrap();
    for failure in 0..5 {
        rejected_or_abandoned(failure);
    }
    registry::tests::finish_real();
    crate::logln!(
        "[device authority close] heap-held MMIO/IRQ claim retains original charge; \
         scratch/direct/IRQ success disposes metadata before staged root completion; \
         detach/invalidation/scratch rejection and MMIO/IRQ guarded abandonment retain five root \
         leases, five authority charges and four scratch pages"
    );
}

fn rejected_or_abandoned(failure: usize) {
    let root = loader::create_user_address_space_handle();
    let cap = if failure == 4 {
        grant_interrupt(root.id(), 225).unwrap()
    } else {
        let cap = grant_mmio(root.id(), 0x0900_0000, 1).unwrap();
        mmio_map_any(root.id(), cap, true).unwrap();
        cap
    };
    let mut owner = PreparedClose::new(root.id(), cap).unwrap();
    owner.claim().unwrap();
    let base = match owner.0.object {
        Some(DeviceObject::Mmio(region)) => region.mapped,
        _ => None,
    };
    if failure < 3 {
        assert_eq!(
            owner.clean_with(
                |asid, base| {
                    unlocked();
                    if failure == 0 {
                        Err(())
                    } else {
                        arch_unmap(asid, base)
                    }
                },
                |asid, base, pages| {
                    unlocked();
                    let confirmed =
                        crate::cpu::isa::memory::tlb::try_inval_range_user(asid, base, pages)
                            .is_ok();
                    confirmed && failure != 1
                },
                |asid, base, pages| {
                    unlocked();
                    if failure == 2 {
                        Err(memory::object::MemoryObjectError::UnknownCapability)
                    } else {
                        memory::object::release_scratch(asid, base, pages)
                    }
                },
            ),
            Err(DeviceError::UnmapFailed),
        );
    }
    assert!(!visible(root, cap));
    assert_eq!(used(root.id()), 1);
    dma_tables::test_drop_under_guards(|| {
        let _devices = DEVICES.lock();
        drop(owner);
    });
    assert_eq!(used(root.id()), 1);
    assert_eq!(memory::current_address_space_handle(root.id()), Some(root));
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight),
    );
    if let Some(base) = base {
        assert_eq!(close_cap(root.id(), cap), Err(DeviceError::OperationInFlight));
        let next = memory::object::reserve_scratch(root.id(), 1).unwrap();
        assert_ne!(next, base, "uncertain device close recycled scratch");
        memory::object::release_scratch(root.id(), next, 1).unwrap();
    } else {
        assert_eq!(close_cap(root.id(), cap), Err(DeviceError::UnknownCapability));
        // The old retained IRQ owner must not touch a new route/capability.
        let fresh = loader::create_user_address_space_handle();
        assert_ne!(fresh.id(), root.id());
        grant_interrupt(fresh.id(), 225).unwrap();
        memory::close_user_address_space_handle(fresh).unwrap();
    }
}
