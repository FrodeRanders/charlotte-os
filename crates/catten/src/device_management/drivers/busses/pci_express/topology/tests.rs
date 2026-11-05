//! Run the real builder against private RAM, before any hardware/reset claim.
use super::*;

pub(super) fn run() {
    let mut backing = alloc::vec![u64::MAX; (2 * 1024 * 1024) / 8];
    let bytes = backing.as_mut_ptr().cast::<u8>();
    let function =
        |bus: usize, device: usize, function: usize, device_id: u16, header: u8, secondary: u8| {
            let offset = (bus << 20) | (device << 15) | (function << 12);
            // Private fixture backing: no hardware accesses or owning capability.
            unsafe {
                bytes.add(offset).cast::<u16>().write_volatile(0x1af4);
                bytes.add(offset + 2).cast::<u16>().write_volatile(device_id);
                bytes.add(offset + 0x0e).write_volatile(header);
                if header & 0x7f == 1 {
                    bytes.add(offset + 0x19).write_volatile(secondary);
                }
            }
        };
    function(0, 0, 0, 0x9990, 1, 1);
    function(0, 1, 0, 0x9991, 0x80, 0);
    function(0, 1, 7, 0x9997, 0, 0);
    function(1, 0, 0, 0x9992, 1, 0); // cycle
    function(1, 1, 0, 0x9993, 1, 200); // outside advertised range
    let base = VAddr::from_mut(bytes);
    let mut storage = Vec::new();
    storage.try_reserve_exact(2).unwrap();
    let buses = PcieSegmentGroup::enumerate_prepared(
        base,
        0,
        &mut Budget::new(),
        Buses::new(0, 1),
        storage,
    )
    .unwrap();
    assert_eq!(buses.iter().map(|bus| bus.number).collect::<Vec<_>>(), alloc::vec![0, 1]);
    let endpoint = buses[0].endpoints().find(|(_, _, function)| *function == 7).unwrap().0;
    let stable = endpoint as *const PcieEndpoint;
    assert_eq!(endpoint.identifier.device_id, 0x9997);
    let topology = PcieTopology::new(alloc::vec![PcieSegmentGroup {
        pcie_segment_group_num: 0,
        ecam_paddr: PAddr::from(0u64),
        ecam_vaddr: base,
        start_bus_num: 0,
        end_bus_num: 1,
        buses,
    }]);
    let endpoint = reset::find_group(&topology.segments[0], 15).unwrap();
    assert_eq!(endpoint as *const PcieEndpoint, stable);
    assert_eq!(topology.segments[0].endpoints().count(), 2);
    assert!(!reset::claimed_range(&topology, 0x4000_0000, 4096));
    // The real multifunction constructor must return a rejected private prefix
    // when the quota ends between function zero and function seven.
    let mut exhausted = Budget::new();
    for _ in 0..enumeration::MAX_FUNCTIONS - 2 {
        exhausted.accept_function().unwrap();
    }
    let mut storage = Vec::new();
    storage.try_reserve_exact(2).unwrap();
    assert!(matches!(
        PcieSegmentGroup::enumerate_prepared(base, 0, &mut exhausted, Buses::new(0, 1), storage),
        Err(EnumerationError::FunctionCapacity)
    ));
    drop(topology);
    drop(backing);
    crate::logln!(
        "[PCI boot] real RAM builder: cycle/range fences, multifunction seven, stable reset cell \
         and rejected private prefix passed; device={} endpoint={} bus={} metadata ceiling={} \
         bytes",
        size_of::<PcieDevice>(),
        size_of::<PcieEndpoint>(),
        size_of::<PcieBusSegment>(),
        enumeration::MAX_FUNCTIONS * (size_of::<PcieDevice>() + size_of::<PcieEndpoint>())
            + enumeration::MAX_REGIONS
                * (size_of::<PcieSegmentGroup>() + 256 * size_of::<PcieBusSegment>())
    );
}
