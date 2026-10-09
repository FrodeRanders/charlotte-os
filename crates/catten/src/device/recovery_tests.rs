//! Pre-driver QEMU fixture uses raw IDs only at the kernel ABI/hardware boundary.
//! Root teardown is the fixture's resource owner; no quarantined state is adopted.
use super::*;
use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    memory::object,
};

static CREATION_IRQ: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn creation_rollback_unlocked() {
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), CREATION_IRQ.load(Ordering::Relaxed));
    dma::test_assert_backend_available();
    drop(
        crate::memory::ADDRESS_SPACE_LIFECYCLE
            .try_lock()
            .expect("creation rollback holds lifecycle"),
    );
    drop(DEVICES.try_lock().expect("creation rollback holds devices"));
    drop(
        crate::memory::ADDRESS_SPACE_TABLE.try_lock().expect("creation rollback holds root table"),
    );
    drop(
        crate::memory::PHYSICAL_FRAME_ALLOCATOR
            .try_lock()
            .expect("creation rollback holds physical allocator"),
    );
    drop(
        crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR
            .try_lock()
            .expect("creation rollback holds heap"),
    );
    crate::device_management::drivers::busses::pci_express::topology::reset::test_assert_disabled_config_available(&crate::DEVICE_TOPOLOGY.pcie);
}

fn destroy_failed_creation(id: u64) -> Result<(), dma::Error> {
    dma::destroy_domain_at(id, creation_rollback_unlocked, creation_rollback_unlocked)
}

pub(crate) fn run() {
    let Some((base, requester)) =
        crate::device_management::drivers::busses::pci_express::topology::reset::test_target(
            &crate::DEVICE_TOPOLOGY.pcie,
        )
    else {
        crate::logln!("[device recovery] QEMU NVMe fixture skipped: supported controller absent");
        return;
    };
    let owner = crate::service::loader::create_user_address_space_handle();
    let mmio = grant_mmio(owner.id(), base, 4).unwrap();
    let memory = object::allocate(owner.id(), 2).unwrap();
    let baseline = dma_tables::used().1;
    let domain = grant_dma_domain(owner.id(), requester, None).unwrap();
    mmio_map_any(owner.id(), mmio, true).unwrap();
    let address = dma_map(owner.id(), domain, memory, 3).unwrap();
    crate::memory::KERNEL_AS.lock().map_mmio_region(base, 4096).unwrap();
    let registers = unsafe { PAddr::from(base as u64).into_hhdm_mut::<u8>() };
    // Configure a real, enabled controller with DMA-addressed admin queues.
    // No command is submitted; its saved queue state must nevertheless be reset
    // before a new domain can reuse the same requester and IOVA aperture.
    unsafe {
        core::ptr::write_volatile(registers.add(0x24).cast::<u32>(), 3 | (3 << 16));
        core::ptr::write_volatile(registers.add(0x28).cast::<u64>(), address);
        core::ptr::write_volatile(registers.add(0x30).cast::<u64>(), address + 4096);
        core::ptr::write_volatile(registers.add(0x14).cast::<u32>(), 1 | (6 << 16) | (4 << 20));
    }
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while unsafe { core::ptr::read_volatile(registers.add(0x1c).cast::<u32>()) } & 1 == 0 {
        deadline.assert_pending("QEMU NVMe ready for recovery fixture");
        core::hint::spin_loop();
    }
    let live = dma_tables::used();
    REJECT_RETIREMENT.store(true, Ordering::Release);
    assert_eq!(close_cap(owner.id(), domain), Err(DeviceError::DmaInvalid));
    assert!(!REJECT_RETIREMENT.load(Ordering::Acquire));
    assert_eq!(dma_tables::used(), live);
    assert_eq!(
        object::try_close_cap(owner.id(), memory),
        Err(object::MemoryObjectError::LendingActive)
    );
    assert_eq!(dma_map(owner.id(), domain, memory, 3), Err(DeviceError::DmaInvalid));
    let id = {
        let devices = DEVICES.lock();
        let DeviceObject::DmaDomain {
            id,
        } = devices[&owner.id()].caps[&domain]
        else {
            panic!("DMA fixture cap")
        };
        id
    };
    let irq_state = crate::cpu::isa::lp::ops::get_int_state();
    dma::destroy_domain_at(
        id,
        || {
            // The actual engine is exclusively moved. Check before any hardware
            // completion; no other domain/reset may mutate its shared command state.
            assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq_state);
            dma::test_assert_backend_available();
            drop(
                crate::memory::ADDRESS_SPACE_LIFECYCLE
                    .try_lock()
                    .expect("maintenance holds lifecycle"),
            );
            drop(DEVICES.try_lock().expect("maintenance holds device registry"));
            drop(
                crate::memory::PHYSICAL_FRAME_ALLOCATOR
                    .try_lock()
                    .expect("maintenance holds physical allocator"),
            );
            drop(
                crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR
                    .try_lock()
                    .expect("maintenance holds heap allocator"),
            );
            assert_eq!(dma_tables::used(), live);
            assert_eq!(dma::initialize_early(), Err(dma::Error::OperationInFlight));
            assert_eq!(dma::destroy_domain(id), Err(dma::Error::OperationInFlight));
            assert_eq!(dma::destroy_domain(u64::MAX), Err(dma::Error::OperationInFlight));
            assert_eq!(dma::unmap(id, address), Err(dma::Error::OperationInFlight));
            assert_eq!(
                dma::map(id, owner.id(), memory, dma::Direction::from_bits(3).unwrap(), false),
                Err(dma::Error::OperationInFlight)
            );
            for sid in [requester, u32::MAX] {
                assert_eq!(
                    dma::create_domain_with_reset(sid, None, &mut DmaCreation::new(), |_| panic!(
                        "claimed engine reached reset"
                    )),
                    Err(dma::Error::OperationInFlight)
                );
            }
            assert_eq!(
                object::try_close_cap(owner.id(), memory),
                Err(object::MemoryObjectError::LendingActive)
            );
            assert_eq!(dma_tables::used(), live);
        },
        || {
            // This exact boundary is after real hardware maintenance, before any
            // table release or data unpin. No acknowledgement is fabricated.
            assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq_state);
            dma::test_assert_backend_available();
            drop(
                crate::memory::ADDRESS_SPACE_LIFECYCLE
                    .try_lock()
                    .expect("DMA release holds lifecycle"),
            );
            drop(DEVICES.try_lock().expect("DMA release holds device registry"));
            drop(
                crate::memory::PHYSICAL_FRAME_ALLOCATOR
                    .try_lock()
                    .expect("DMA release holds physical allocator"),
            );
            drop(
                crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR
                    .try_lock()
                    .expect("DMA release holds heap allocator"),
            );
            assert_eq!(dma_tables::used(), live);
            assert_eq!(dma::destroy_domain(id), Err(dma::Error::UnknownDomain));
            assert_eq!(dma::unmap(id, address), Err(dma::Error::UnknownDomain));
            assert_eq!(
                dma::map(id, owner.id(), memory, dma::Direction::from_bits(3).unwrap(), false),
                Err(dma::Error::UnknownDomain)
            );
            assert_eq!(
                dma::create_domain_with_reset(
                    requester,
                    None,
                    &mut DmaCreation::new(),
                    |_| panic!("claimed requester reached reset")
                ),
                Err(dma::Error::StreamInUse)
            );
            assert_eq!(
                object::try_close_cap(owner.id(), memory),
                Err(object::MemoryObjectError::LendingActive)
            );
            assert_eq!(dma_tables::used(), live);
        },
    )
    .unwrap();
    // Complete the still-owned fixture capability only after backend success.
    close_cap(owner.id(), domain).unwrap();
    assert_eq!(dma_tables::used().1, baseline);
    let successor = crate::service::loader::create_user_address_space_handle();
    assert_eq!(grant_dma_domain(successor.id(), requester, None), Err(DeviceError::DmaUnavailable));
    // Old register authority must disappear before reset can clear the source
    // fence. Root completion cannot precede its mapping invalidation.
    close_cap_with(owner.id(), mmio, || {
        assert_eq!(close_cap(owner.id(), mmio), Err(DeviceError::OperationInFlight));
        assert_eq!(
            grant_dma_domain(successor.id(), requester, None),
            Err(DeviceError::DmaUnavailable)
        );
        assert!(!crate::capability::contains(
            owner.id(),
            mmio,
            crate::capability::ObjectKind::Device
        ));
    })
    .unwrap();
    object::close_cap(owner.id(), memory).unwrap();
    crate::memory::close_user_address_space_handle(owner).unwrap();
    {
        let _pressure = dma_tables::ClientPressure::new();
        let caps = crate::capability::admission_tests::test_namespace_used(successor.id());
        assert_eq!(
            grant_dma_domain(successor.id(), requester, None),
            Err(DeviceError::DmaUnavailable)
        );
        assert_eq!(crate::capability::admission_tests::test_namespace_used(successor.id()), caps);
    }
    assert_eq!(dma_tables::used().1, baseline);
    let rejected = crate::service::loader::create_user_address_space_handle();
    let charges = dma_tables::used();
    CREATION_IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    REJECT_CREATION.store(true, Ordering::Release);
    assert_eq!(
        grant_dma_domain_with_backend(
            rejected.id(),
            |creation| create_dma_domain(rejected.id(), requester, None, creation),
            destroy_failed_creation,
        ),
        Err(DeviceError::DmaUnavailable)
    );
    assert!(!REJECT_CREATION.load(Ordering::Acquire));
    assert_eq!(dma_tables::used(), charges);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(rejected.id()), 0);
    assert!(!DEVICES.lock().contains_key(&rejected.id()));
    crate::memory::close_user_address_space_handle(rejected).unwrap();
    crate::logln!(
        "[DMA creation rollback] real configuration followed by injected rejection; unlocked \
         maintenance/physical phase with PCI config released and bus mastering disabled, exact \
         charge/capability refund and root close passed"
    );
    let domain = grant_dma_domain(successor.id(), requester, None).unwrap();
    assert_eq!(unsafe { core::ptr::read_volatile(registers.add(0x14).cast::<u32>()) } & 1, 0);
    assert_eq!(unsafe { core::ptr::read_volatile(registers.add(0x1c).cast::<u32>()) } & 1, 0);
    // Prime cached leaf tables, then reject a two-page map at their boundary.
    let memory = object::allocate(successor.id(), 2).unwrap();
    let address = dma_map(successor.id(), domain, memory, 3).unwrap();
    dma_unmap(successor.id(), domain, address).unwrap();
    let id = {
        let devices = DEVICES.lock();
        let DeviceObject::DmaDomain {
            id,
        } = devices[&successor.id()].caps[&domain]
        else {
            panic!("DMA fixture cap")
        };
        id
    };
    dma::test_reject_sparse_map(id);
    let charged = dma_tables::used();
    assert_eq!(dma_map(successor.id(), domain, memory, 3), Err(DeviceError::DmaInvalid));
    assert_eq!(dma_tables::used(), charged);
    object::close_cap(successor.id(), memory).unwrap();
    let held = object::allocate(successor.id(), 2).unwrap();
    REJECT_MAP_ROLLBACK.store(true, Ordering::Release);
    assert_eq!(dma_map(successor.id(), domain, held, 3), Err(DeviceError::DmaInvalid));
    assert!(!REJECT_MAP_ROLLBACK.load(Ordering::Acquire));
    assert_eq!(dma_tables::used(), charged);
    assert_eq!(
        object::try_close_cap(successor.id(), held),
        Err(object::MemoryObjectError::LendingActive)
    );
    assert_eq!(dma_map(successor.id(), domain, held, 3), Err(DeviceError::DmaInvalid));
    close_cap(successor.id(), domain).unwrap();
    assert_eq!(dma_tables::used().1, baseline);
    object::close_cap(successor.id(), held).unwrap();
    crate::logln!(
        "[IOMMU recovery] detached physical-release boundary keeps pins/source claim, guards \
         available and competing operations fenced; hardware table charges retained until drain, \
         node pressure refunded capability, rejected sparse-prefix cleanup retained pin until \
         real retirement"
    );
    crate::memory::close_user_address_space_handle(successor).unwrap();
    crate::logln!(
        "[device recovery] rejected drain retained/fenced DMA; real retry, old-MMIO exclusion and \
         QEMU NVMe reset/reassignment passed"
    );
}
