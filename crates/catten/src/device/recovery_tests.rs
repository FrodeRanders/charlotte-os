//! Pre-driver QEMU fixture uses raw IDs only at the kernel ABI/hardware boundary.
//! Root teardown is the fixture's resource owner; no quarantined state is adopted.
use super::*;
use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    memory::object,
};

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
    REJECT_RETIREMENT.store(true, Ordering::Release);
    assert_eq!(close_cap(owner.id(), domain), Err(DeviceError::DmaInvalid));
    assert!(!REJECT_RETIREMENT.load(Ordering::Acquire));
    assert_eq!(
        object::try_close_cap(owner.id(), memory),
        Err(object::MemoryObjectError::LendingActive)
    );
    assert_eq!(dma_map(owner.id(), domain, memory, 3), Err(DeviceError::DmaInvalid));
    close_cap(owner.id(), domain).unwrap();
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
    let domain = grant_dma_domain(successor.id(), requester, None).unwrap();
    assert_eq!(unsafe { core::ptr::read_volatile(registers.add(0x14).cast::<u32>()) } & 1, 0);
    assert_eq!(unsafe { core::ptr::read_volatile(registers.add(0x1c).cast::<u32>()) } & 1, 0);
    close_cap(successor.id(), domain).unwrap();
    crate::memory::close_user_address_space_handle(successor).unwrap();
    crate::logln!(
        "[device recovery] rejected drain retained/fenced DMA; real retry, old-MMIO exclusion and \
         QEMU NVMe reset/reassignment passed"
    );
}
