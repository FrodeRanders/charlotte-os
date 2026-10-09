//! Synthetic containing-owner evidence, separate from real QEMU maintenance.
use super::*;
use crate::{
    device::dma_tables::{
        Scope,
        Tables,
    },
    memory::object,
};

fn destroy_fake(id: u64) -> Result<(), dma::Error> {
    assert_eq!(id, u64::MAX);
    Ok(())
}

fn grant(root: crate::memory::AddressSpaceHandle) -> DeviceCap {
    grant_dma_domain_with_backend(
        root.id(),
        |creation| {
            creation.record(u64::MAX); // Kernel fixture, no hardware domain.
            Ok(())
        },
        destroy_fake,
    )
    .unwrap()
}

struct Payload {
    _tables: Tables,
    _metadata: alloc::vec::Vec<u64>,
    #[cfg(target_arch = "aarch64")]
    _walker: super::super::mapping_storage::WalkerCache,
}

pub(super) fn run() {
    let root = crate::service::loader::create_user_address_space_handle();
    let cap = grant(root);
    for fails in [false, true] {
        let result = with_operation(root.id(), cap, |id| {
            assert_eq!(id, u64::MAX);
            assert_eq!(close_cap(root.id(), cap), Err(DeviceError::OperationInFlight));
            assert_eq!(dma_unmap(root.id(), cap, 0), Err(DeviceError::OperationInFlight));
            assert_eq!(
                crate::memory::close_user_address_space_handle(root),
                Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
            );
            if fails {
                Err(dma::Error::UnknownMapping)
            } else {
                Ok(())
            }
        });
        assert_eq!(
            result,
            if fails {
                Err(DeviceError::DmaInvalid)
            } else {
                Ok(())
            }
        );
    }
    let cell = {
        let devices = DEVICES.lock();
        &devices[&root.id()].caps[&cap] as *const DeviceObject
    };
    let charged = crate::capability::admission_tests::test_namespace_used(root.id());
    for error in [dma::Error::OperationInFlight, dma::Error::HardwareTimeout, dma::Error::MapFailed]
    {
        // The complete rejection path must not allocate or deallocate registry
        // nodes. Holding the actual heap would deadlock the former extraction/
        // reinsertion path. All capability/root admission is lookup-only here.
        let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        assert_eq!(
            close_with(
                root.id(),
                cap,
                || {
                    assert_eq!(close_cap(root.id(), cap), Err(DeviceError::OperationInFlight));
                    assert_eq!(dma_unmap(root.id(), cap, 0), Err(DeviceError::OperationInFlight));
                    assert_eq!(
                        crate::memory::close_user_address_space_handle(root),
                        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
                    );
                    assert!(crate::capability::contains(
                        root.id(),
                        cap,
                        crate::capability::ObjectKind::Device
                    ));
                },
                |id| {
                    assert_eq!(id, u64::MAX);
                    Err(error)
                }
            ),
            Err(DeviceError::DmaInvalid)
        );
        drop(heap);
        let devices = DEVICES.lock();
        assert_eq!(&devices[&root.id()].caps[&cap] as *const DeviceObject, cell);
        assert!(matches!(
            devices[&root.id()].caps[&cap],
            DeviceObject::DmaDomain {
                id: u64::MAX,
                operation_in_flight: false
            }
        ));
        drop(devices);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), charged);
    }
    close_cap(root.id(), cap).unwrap();
    crate::memory::close_user_address_space_handle(root).unwrap();

    // An older close lease may finish through a staged root-close fence. Only
    // confirmed backend success consumes authority and lets the same closing
    // owner proceed; it never needs to allocate a replacement capability cell.
    use crate::memory::retirement::{
        CloseProgress,
        ClosingAddressSpace,
    };
    let root = crate::service::loader::create_user_address_space_handle();
    let cap = grant(root);
    let mut closing = None;
    close_with(
        root.id(),
        cap,
        || {
            closing = Some(match ClosingAddressSpace::begin(root).unwrap().poll().unwrap() {
                CloseProgress::Pending(owner) => owner,
                CloseProgress::Complete => panic!("DMA close root finished before backend"),
            });
            assert!(AddressSpaceOperation::acquire(root).is_err());
            assert!(crate::capability::contains(
                root.id(),
                cap,
                crate::capability::ObjectKind::Device
            ));
        },
        destroy_fake,
    )
    .unwrap();
    assert!(!crate::capability::contains(root.id(), cap, crate::capability::ObjectKind::Device));
    assert!(matches!(closing.unwrap().poll().unwrap(), CloseProgress::Complete));
    crate::logln!(
        "[DMA close ownership] busy/timeout/physical rejection with heap held preserves the same \
         payload cell and authority; nested close/map and exact-root close reject; confirmed \
         close consumes authority before the original staged root completes"
    );

    let root = crate::service::loader::create_user_address_space_handle();
    let cap = grant(root);
    let memory = object::allocate(root.id(), 2).unwrap();
    let operation = DmaOperation::begin(root.id(), cap).unwrap();
    let pin = object::pin_for_dma(root.id(), memory, true, true, false).unwrap();
    let baseline = crate::device::dma_tables::used();
    let mut tables = Tables::new(Scope::Domain);
    let frame = tables.allocate_frame().unwrap();
    #[cfg(target_arch = "x86_64")]
    let _ = frame;
    #[cfg(target_arch = "aarch64")]
    let walker = {
        // Metadata fixture borrows this retained table; no hardware walk occurs.
        let mut walker = super::super::mapping_storage::WalkerCache::new();
        walker.prepare().unwrap();
        walker.publish(1, frame);
        walker.prepare().unwrap(); // Partial-walk storage retains with the domain.
        walker
    };
    tables.publish(); // Synthetic state; no hardware-visible base.
    let mut owner = MappingMaintenance::new(
        Maintenance::new(
            Payload {
                _tables: tables,
                _metadata: alloc::vec![0x444d_415f_5049_4e53u64],
                #[cfg(target_arch = "aarch64")]
                _walker: walker,
            },
            alloc::vec![0x454e_4749_4e45u64],
        ),
        PendingPin::new(Some(pin)),
    );
    owner.pending.prepare().unwrap();
    crate::device::dma_tables::test_drop_under_guards(|| {
        let _devices = DEVICES.lock();
        drop(owner);
        drop(operation);
    });
    assert_eq!(crate::device::dma_tables::used(), (baseline.0 + 1, baseline.1 + 1));
    assert_eq!(
        crate::memory::close_user_address_space_handle(root),
        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(close_cap(root.id(), cap), Err(DeviceError::OperationInFlight));
    assert_eq!(dma_unmap(root.id(), cap, 0), Err(DeviceError::OperationInFlight));
    assert_eq!(
        object::try_close_cap(root.id(), memory),
        Err(object::MemoryObjectError::LendingActive)
    );
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_ne!(fresh.id(), root.id());
    crate::memory::close_user_address_space_handle(fresh).unwrap();
    crate::logln!(
        "[DMA mapping ownership] exact-root/capability busy close and ordinary completion; \
         complete domain/engine/pending-pin Drop under guards retains one domain table, two data \
         frames, metadata, original authority and root"
    );
}
