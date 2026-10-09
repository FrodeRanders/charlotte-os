//! RAM config claims are separate from real QEMU reset acknowledgements.
use core::sync::atomic::AtomicUsize;

use super::*;

static ACTIVATION_PROBES: AtomicUsize = AtomicUsize::new(0);
static REAL_PROBES: AtomicUsize = AtomicUsize::new(0);
static REAL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) fn probe(source: &ResetSource<'_>) {
    if !REAL_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(1000);
    let config = loop {
        if let Some(config) = source.endpoint.cfg_ptr.try_lock() {
            break config;
        }
        assert!(
            crate::cpu::scheduler::monotonic_millis() < deadline,
            "reset polling holds config guard"
        );
        core::hint::spin_loop();
    };
    assert_eq!(config.reset_ranges, Some(source.ranges()));
    let command = unsafe { read_volatile(config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>()) };
    assert_eq!(command & 4, 0);
    drop(config);
    assert!(source.endpoint.config().is_none());
    while !crate::device::test_reset_devices_available() {
        assert!(
            crate::cpu::scheduler::monotonic_millis() < deadline,
            "reset polling holds device guard"
        );
        core::hint::spin_loop();
    }
    let bar = source.bars[0].unwrap();
    assert!(claimed_range(&crate::DEVICE_TOPOLOGY.pcie, bar.base, bar.bound));
    assert!(claimed_range(&crate::DEVICE_TOPOLOGY.pcie, source.config_base, 4096));
    assert!(lookup_first_nvme(&crate::DEVICE_TOPOLOGY.pcie).is_none());
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    REAL_PROBES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn begin_real() {
    ACTIVATION_PROBES.store(0, Ordering::Relaxed);
    REAL_PROBES.store(0, Ordering::Relaxed);
    REAL_ACTIVE.store(true, Ordering::Release);
}
pub(crate) fn finish_real() {
    REAL_ACTIVE.store(false, Ordering::Release);
    let count = REAL_PROBES.load(Ordering::Acquire);
    assert!(count > 0);
    let publications = ACTIVATION_PROBES.load(Ordering::Acquire);
    assert!(publications > 0 && publications < count);
    crate::logln!(
        "[PCI reset publication] {} real grants: exact root and published capability remain busy \
         with BME disabled until explicit post-publication activation",
        publications
    );
    crate::logln!(
        "[PCI reset claim] {} real reset wait boundaries: config/device guards available, bus \
         mastering disabled, ordinary config/MSI lookup rejected and captured BAR ranges fenced; \
         wider lifecycle/backend guards remain",
        count
    );
}

pub(crate) fn before_activation(
    source: &ResetSource<'_>,
    owner: crate::memory::AddressSpaceId,
    cap: crate::device::DeviceCap,
) {
    if !REAL_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    use crate::device::*;
    let root = crate::memory::current_address_space_handle(owner).unwrap();
    assert_eq!(close_cap(owner, cap), Err(DeviceError::OperationInFlight));
    assert_eq!(dma_unmap(owner, cap, u64::MAX), Err(DeviceError::OperationInFlight));
    assert_eq!(
        crate::memory::close_user_address_space_handle(root),
        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(grant_mmio(owner, source.config_base, 1), Err(DeviceError::OperationInFlight));
    let config = source.endpoint.cfg_ptr.lock();
    assert_eq!(config.reset_ranges, Some(source.ranges()));
    assert_eq!(
        unsafe { read_volatile(config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>()) } & 4,
        0
    );
    drop(config);
    ACTIVATION_PROBES.fetch_add(1, Ordering::Relaxed);
}

fn fixture() -> (PcieTopology, Box<[u64; 512]>) {
    let mut registers = Box::new([0u64; 512]);
    let ptr = NonNull::new(registers.as_mut_ptr().cast::<PcieCfgSpace>()).unwrap();
    unsafe {
        write_volatile(ptr.as_ptr().cast::<u8>().add(4).cast::<u16>(), 6);
        write_volatile(ptr.as_ptr().cast::<u8>().add(0x10).cast::<u32>(), 0x4000_0000);
    }
    let topology = PcieTopology::new(alloc::vec![PcieSegmentGroup {
        pcie_segment_group_num: 0,
        ecam_paddr: PAddr::from(0x5000_0000u64),
        ecam_vaddr: VAddr::from(0u64),
        start_bus_num: 0,
        end_bus_num: 0,
        root_bus: Box::new(PcieBusSegment {
            number: 0,
            devices: alloc::vec![PcieDevice::SingleFunc(PcieSingleFuncDevice {
                number: PcieDeviceNum(0),
                function: PcieFunction::Endpoint(Box::new(PcieEndpoint {
                    number: PcieFunctionNum(0),
                    identifier: PciIdentifier {
                        vendor_id: 0x1b36,
                        device_id: 0x0010,
                        class_code: 1,
                        subclass: 8,
                        prog_if: 2
                    },
                    cfg_ptr: SpinMutex::new(ConfigSpace {
                        ptr,
                        reset_ranges: None
                    }),
                })),
            })],
        }),
    }]);
    (topology, registers)
}

pub(super) fn run() {
    if cfg!(feature = "hvf_compat") {
        return;
    } // This profile intentionally avoids ECAM.
    // A fake topology must not set the publication hint before the global one
    // exists. Force once here outside lifecycle/device/allocator/config guards.
    spin::LazyLock::force(&crate::DEVICE_TOPOLOGY);
    let (topology, _ram) = fixture();
    assert!(prepare_qemu_nvme(&topology, 0, |_, _| false).is_err());
    assert!(
        prepare_qemu_nvme(&topology, 0, |_, _| {
            // Change only an unused BAR's type flags while range admission runs.
            // Exact six-register revalidation must reject before claim/mutation.
            let endpoint = find_bus(&topology.segments[0].root_bus, 0, 0).unwrap();
            let config = endpoint.config().unwrap();
            unsafe { write_volatile(config.as_ptr().cast::<u8>().add(0x24).cast::<u32>(), 1) };
            true
        })
        .is_err()
    );
    {
        let endpoint = find_bus(&topology.segments[0].root_bus, 0, 0).unwrap();
        let config = endpoint.config().unwrap();
        unsafe { write_volatile(config.as_ptr().cast::<u8>().add(0x24).cast::<u32>(), 0) };
    }
    let source =
        prepare_qemu_nvme(&topology, 0, |_, _| true).unwrap_or_else(|_| panic!("RAM reset claim"));
    assert!(claimed_range(&topology, 0x4000_1000, 4096));
    assert!(!claimed_range(&topology, 0x4000_4000, 4096));
    assert!(claimed_range(&topology, 0x5000_0000, 4096));
    assert!(
        prepare_qemu_nvme(&topology, 0, |_, _| panic!("claimed source reached range admission"))
            .is_err()
    );
    assert!(source.endpoint.config().is_none());
    assert!(lookup_first_nvme(&topology).is_none());
    source.cancel().unwrap_or_else(|_| panic!("unstarted reset cancellation"));
    assert!(!claimed_range(&topology, 0x4000_0000, 4096));
    let mut source =
        prepare_qemu_nvme(&topology, 0, |_, _| true).unwrap_or_else(|_| panic!("RAM reset claim"));
    source.phase = Phase::Ready; // State adapter, never a hardware completion.
    source.activate().unwrap_or_else(|_| panic!("RAM activation"));
    let endpoint = find_bus(&topology.segments[0].root_bus, 0, 0).unwrap();
    let config = endpoint.config().unwrap();
    assert_eq!(unsafe { read_volatile(config.as_ptr().cast::<u8>().add(4).cast::<u16>()) } & 6, 6);
    drop(config);

    for uncertain in [false, true] {
        // Stable fixture metadata follows terminal claim lifetime. These RAM
        // allocations are deliberately retained, never physical device owners.
        let (topology, ram) = fixture();
        let topology = Box::leak(Box::new(topology));
        let _ram = Box::leak(ram);
        let mut source = prepare_qemu_nvme(topology, 0, |_, _| true)
            .unwrap_or_else(|_| panic!("RAM reset claim"));
        if uncertain {
            source.phase = Phase::Uncertain;
        }
        let source = if uncertain {
            source.activate().expect_err("uncertain reset activated")
        } else {
            source
        };
        let endpoint = source.endpoint;
        let config = endpoint.cfg_ptr.lock();
        let before =
            unsafe { read_volatile(config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>()) };
        crate::device::admission_tests::test_reset_grant_retention(source, uncertain);
        let after = unsafe { read_volatile(config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>()) };
        assert_eq!(before, after, "reset fallback touched hardware");
        assert!(config.reset_ranges.is_some());
        drop(config);
        assert!(endpoint.config().is_none());
        assert!(prepare_qemu_nvme(topology, 0, |_, _| panic!("retained claim replay")).is_err());
    }
    crate::logln!(
        "[PCI reset ownership] RAM claim rejection, explicit unstarted cancellation/activation \
         and complete-grant abandonment/uncertain cancellation under config and all guards \
         passed; two exact roots/reservations and endpoint claims retained without hardware writes"
    );
}
