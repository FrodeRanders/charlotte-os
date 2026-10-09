//! Pre-driver QEMU fixture uses raw IDs only at the kernel ABI/hardware boundary.
//! Root teardown is the fixture's resource owner; no quarantined state is adopted.
use super::*;
use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    memory::object,
};

static CREATION_IRQ: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

// Other LPs can legitimately use global allocators during this pre-driver
// fixture. One rejected try_lock proves contention, not that this caller owns
// the lock. Bound observation without yielding or enabling the caller's IRQs;
// a lock retained by this caller cannot pass, and timeout remains a failure.
fn assert_available(mut available: impl FnMut() -> bool, boundary: &'static str) {
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    let mut contended = false;
    while !available() {
        contended = true;
        deadline.assert_pending(boundary);
        core::hint::spin_loop();
    }
    if contended {
        crate::logln!(
            "[device recovery] lock availability recovered after contention: {}",
            boundary
        );
    }
}

fn creation_rollback_unlocked() {
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), CREATION_IRQ.load(Ordering::Relaxed));
    dma::test_assert_backend_available();
    assert_available(
        || crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "creation rollback lifecycle availability",
    );
    assert_available(|| DEVICES.try_lock().is_some(), "creation rollback devices availability");
    assert_available(
        || crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some(),
        "creation rollback root table availability",
    );
    assert_available(
        || crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "creation rollback physical allocator availability",
    );
    assert_available(
        || crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some(),
        "creation rollback heap availability",
    );
    crate::device_management::drivers::busses::pci_express::topology::reset::test_assert_disabled_config_available(&crate::DEVICE_TOPOLOGY.pcie);
}

static PRIVATE_RELEASE_PROBE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static PRIVATE_RELEASE_PROBES: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

pub(super) fn probe_private_release() {
    if PRIVATE_RELEASE_PROBE.load(Ordering::Acquire) {
        creation_rollback_unlocked();
        PRIVATE_RELEASE_PROBES.fetch_add(1, Ordering::Relaxed);
    }
}

fn destroy_failed_creation(id: u64) -> Result<(), dma::Error> {
    dma::destroy_domain_at(id, creation_rollback_unlocked, creation_rollback_unlocked)
}

fn mapping_unlocked(
    root: crate::memory::AddressSpaceHandle,
    cap: DeviceCap,
    memory: u64,
    id: u64,
    irq: bool,
) {
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    dma::test_assert_backend_available();
    assert_available(
        || crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "mapping lifecycle availability",
    );
    assert_available(|| DEVICES.try_lock().is_some(), "mapping device availability");
    assert_available(
        || crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some(),
        "mapping root table availability",
    );
    assert_available(
        || crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "mapping physical availability",
    );
    assert_available(
        || crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some(),
        "mapping heap availability",
    );
    assert_eq!(dma::initialize_early(), Err(dma::Error::OperationInFlight));
    assert_eq!(dma::destroy_domain(id), Err(dma::Error::OperationInFlight));
    assert_eq!(dma::destroy_domain(u64::MAX), Err(dma::Error::OperationInFlight));
    assert_eq!(dma::unmap(id, u64::MAX), Err(dma::Error::OperationInFlight));
    assert_eq!(
        dma::map(id, root.id(), memory, dma::Direction::from_bits(3).unwrap(), false),
        Err(dma::Error::OperationInFlight)
    );
    assert_eq!(
        dma::create_domain_with_reset(u32::MAX, None, &mut DmaCreation::new(), |_, _| panic!(
            "mapping engine reached reset"
        )),
        Err(dma::Error::OperationInFlight)
    );
    assert_eq!(close_cap(root.id(), cap), Err(DeviceError::OperationInFlight));
    assert_eq!(dma_unmap(root.id(), cap, u64::MAX), Err(DeviceError::OperationInFlight));
    assert_eq!(
        crate::memory::close_user_address_space_handle(root),
        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(
        object::try_close_cap(root.id(), memory),
        Err(object::MemoryObjectError::LendingActive)
    );
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
    use crate::device_management::drivers::busses::pci_express::topology::reset;
    let source = {
        let devices = DEVICES.lock();
        reset::prepare_qemu_nvme(&crate::DEVICE_TOPOLOGY.pcie, requester, |base, bytes| {
            reset_registers_available(&devices, owner.id(), base, bytes)
        })
        .unwrap_or_else(|_| panic!("real unstarted reset claim"))
    };
    assert_eq!(grant_mmio(owner.id(), base, 1), Err(DeviceError::OperationInFlight));
    assert_eq!(
        grant_mmio(owner.id(), source.test_config_base(), 1),
        Err(DeviceError::OperationInFlight)
    );
    assert_eq!(mmio_map_any(owner.id(), mmio, true), Err(DeviceError::OperationInFlight));
    assert_eq!(close_cap(owner.id(), mmio), Err(DeviceError::OperationInFlight));
    assert_eq!(mmio_unmap(owner.id(), mmio), Err(DeviceError::OperationInFlight));
    {
        let _devices = DEVICES.lock();
        source.cancel().unwrap_or_else(|_| panic!("real unstarted reset cancellation"));
    }
    reset::tests::begin_real();
    let memory = object::allocate(owner.id(), 2).unwrap();
    let baseline = dma_tables::used().1;
    let domain = grant_dma_domain(owner.id(), requester, None).unwrap();
    mmio_map_any(owner.id(), mmio, true).unwrap();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let mut mapped = 0;
    let address = mapping::with_operation(owner.id(), domain, |id| {
        dma::map_at(id, owner.id(), memory, dma::Direction::from_bits(3).unwrap(), false, |phase| {
            assert_eq!(phase, mapping::Phase::Map);
            mapping_unlocked(owner, domain, memory, id, irq);
            mapped += 1;
        })
    })
    .unwrap();
    assert_eq!(mapped, 1);
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
            ..
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
            assert_available(
                || crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
                "maintenance lifecycle availability",
            );
            assert_available(
                || DEVICES.try_lock().is_some(),
                "maintenance device registry availability",
            );
            assert_available(
                || crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
                "maintenance physical allocator availability",
            );
            assert_available(
                || {
                    crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR
                        .try_lock()
                        .is_some()
                },
                "maintenance heap allocator availability",
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
                    dma::create_domain_with_reset(
                        sid,
                        None,
                        &mut DmaCreation::new(),
                        |_, _| panic!("claimed engine reached reset")
                    ),
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
            assert_available(
                || crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
                "DMA release lifecycle availability",
            );
            assert_available(
                || DEVICES.try_lock().is_some(),
                "DMA release device registry availability",
            );
            assert_available(
                || crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
                "DMA release physical allocator availability",
            );
            assert_available(
                || {
                    crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR
                        .try_lock()
                        .is_some()
                },
                "DMA release heap allocator availability",
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
                    |_, _| panic!("claimed requester reached reset")
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
    // Fail every private root/CD/MSI prefix after its actual physical allocation
    // and once after complete metadata preparation, before hardware publication.
    #[cfg(target_arch = "x86_64")]
    let prefixes = if crate::environment::acpi::sdt::dmar::discover_vtd().is_some() {
        4
    } else {
        1
    };
    #[cfg(target_arch = "aarch64")]
    let prefixes = 5;
    for prefix in 1..=prefixes + 1 {
        let rejected = crate::service::loader::create_user_address_space_handle();
        let charges = dma_tables::used();
        CREATION_IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
        PRIVATE_RELEASE_PROBES.store(0, Ordering::Relaxed);
        PRIVATE_RELEASE_PROBE.store(true, Ordering::Release);
        if prefix <= prefixes {
            REJECT_PRIVATE_ALLOCATION.store(prefix, Ordering::Release);
        } else {
            REJECT_PRIVATE_COMPLETE.store(true, Ordering::Release);
        }
        assert_eq!(
            grant_dma_domain(rejected.id(), requester, Some(0xfee0_0000)),
            Err(DeviceError::DmaUnavailable)
        );
        PRIVATE_RELEASE_PROBE.store(false, Ordering::Release);
        assert_eq!(REJECT_PRIVATE_ALLOCATION.load(Ordering::Acquire), 0);
        assert!(!REJECT_PRIVATE_COMPLETE.load(Ordering::Acquire));
        assert_eq!(PRIVATE_RELEASE_PROBES.load(Ordering::Relaxed), 2);
        assert_eq!(dma_tables::used(), charges);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(rejected.id()), 0);
        assert!(!DEVICES.lock().contains_key(&rejected.id()));
        crate::memory::close_user_address_space_handle(rejected).unwrap();
    }
    crate::logln!(
        "[DMA private rollback] all {} allocated prefixes plus complete preparation rejected; \
         physical/metadata cleanup outside backend/lifecycle/device/config guards, exact \
         charge/capability refund and root close passed",
        prefixes
    );
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
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let mut unmapped = 0;
    mapping::with_operation(successor.id(), domain, |id| {
        dma::unmap_at(id, address, |phase| {
            assert_eq!(phase, mapping::Phase::Unmap);
            mapping_unlocked(successor, domain, memory, id, irq);
            unmapped += 1;
        })
    })
    .unwrap();
    assert_eq!(unmapped, 1);
    let id = {
        let devices = DEVICES.lock();
        let DeviceObject::DmaDomain {
            id,
            ..
        } = devices[&successor.id()].caps[&domain]
        else {
            panic!("DMA fixture cap")
        };
        id
    };
    dma::test_reject_sparse_map(id);
    let charged = dma_tables::used();
    let mut rolled_back = 0;
    assert_eq!(
        mapping::with_operation(successor.id(), domain, |id| {
            dma::map_at(
                id,
                successor.id(),
                memory,
                dma::Direction::from_bits(3).unwrap(),
                false,
                |phase| {
                    assert_eq!(phase, mapping::Phase::Rollback);
                    mapping_unlocked(successor, domain, memory, id, irq);
                    rolled_back += 1;
                },
            )
        }),
        Err(DeviceError::DmaInvalid)
    );
    assert_eq!(rolled_back, 1);
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
    // A failed unmap detaches leaves but never discharges their data pin. The
    // admitted quarantine is terminal to this unmap, completed by real domain
    // retirement rather than an allocating error-path mapping reinsertion.
    let domain = grant_dma_domain(successor.id(), requester, None).unwrap();
    let held = object::allocate(successor.id(), 2).unwrap();
    let address = dma_map(successor.id(), domain, held, 3).unwrap();
    let charged = dma_tables::used();
    REJECT_UNMAP_COMPLETION.store(true, Ordering::Release);
    assert_eq!(dma_unmap(successor.id(), domain, address), Err(DeviceError::DmaInvalid));
    assert!(!REJECT_UNMAP_COMPLETION.load(Ordering::Acquire));
    assert_eq!(dma_tables::used(), charged);
    assert_eq!(
        object::try_close_cap(successor.id(), held),
        Err(object::MemoryObjectError::LendingActive)
    );
    assert_eq!(dma_unmap(successor.id(), domain, address), Err(DeviceError::DmaInvalid));
    assert_eq!(dma_map(successor.id(), domain, held, 3), Err(DeviceError::DmaInvalid));
    close_cap(successor.id(), domain).unwrap();
    object::close_cap(successor.id(), held).unwrap();
    assert_eq!(dma_tables::used().1, baseline);
    crate::logln!(
        "[DMA mapping maintenance] real map/unmap/prefix completion outside \
         backend/lifecycle/device/table guards; exact-root/capability busy close, unit-wide \
         mutation/reset exclusion and rejected-unmap pin retention until real retirement passed"
    );
    crate::logln!(
        "[IOMMU recovery] detached physical-release boundary keeps pins/source claim, guards \
         available and competing operations fenced; hardware table charges retained until drain, \
         node pressure refunded capability, rejected sparse-prefix cleanup retained pin until \
         real retirement"
    );
    crate::memory::close_user_address_space_handle(successor).unwrap();
    reset::tests::finish_real();
    crate::logln!(
        "[device recovery] rejected drain retained/fenced DMA; real retry, old-MMIO exclusion and \
         QEMU NVMe reset/reassignment passed"
    );
}
