//! Synthetic containing-owner evidence, separate from real QEMU maintenance.
use super::*;
use crate::device::dma_tables::{
    Scope,
    Tables,
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
    close_cap(root.id(), cap).unwrap();
    crate::memory::close_user_address_space_handle(root).unwrap();

    let root = crate::service::loader::create_user_address_space_handle();
    let cap = grant(root);
    let memory = object::allocate(root.id(), 2).unwrap();
    let operation = DmaOperation::begin(root.id(), cap).unwrap();
    let pin = object::pin_for_dma(root.id(), memory, true, true, false).unwrap();
    let baseline = crate::device::dma_tables::used();
    let mut tables = Tables::new(Scope::Domain);
    tables.allocate_frame().unwrap();
    tables.publish(); // Synthetic state; no hardware-visible base.
    let owner = MappingMaintenance::new(
        Maintenance::new(
            Payload {
                _tables: tables,
                _metadata: alloc::vec![0x444d_415f_5049_4e53u64],
            },
            alloc::vec![0x454e_4749_4e45u64],
        ),
        PendingPin::new(Some(pin)),
    );
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
