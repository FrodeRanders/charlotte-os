//! QEMU NVMe reset owns a logical claim in its exact endpoint config cell.
//! Short config holds never span reset polling. Abandonment retains the claim;
//! activation/cancellation are explicit, and no destructor touches hardware.
use core::{
    ptr::{
        read_volatile,
        write_volatile,
    },
    sync::atomic::{
        AtomicBool,
        Ordering,
    },
};

use super::*;

// A conservative hint only: every actual claim requires an already available
// topology. Early grants must never trigger lazy topology creation under locks.
static CLAIM_PUBLISHED: AtomicBool = AtomicBool::new(false);
pub(crate) fn any_claim_published() -> bool {
    CLAIM_PUBLISHED.load(Ordering::Acquire)
}
use crate::cpu::isa::interface::memory::address::PhysicalAddress;

#[derive(Clone, Copy)]
struct Bar {
    base: usize,
    bound: usize,
    low: u32,
    high: u32,
    wide: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Claimed,
    Uncertain,
    Ready,
}

#[must_use]
pub(crate) struct ResetSource<'a> {
    endpoint: &'a PcieEndpoint,
    command: u16,
    config_base: usize,
    bars: [Option<Bar>; 6],
    phase: Phase,
}

impl ResetSource<'_> {
    fn ranges(&self) -> [Option<(usize, usize)>; 7] {
        let mut ranges = [None; 7];
        for (index, bar) in self.bars.iter().enumerate() {
            ranges[index] = bar.map(|bar| (bar.base, bar.bound));
        }
        ranges[6] = Some((self.config_base, 4096));
        ranges
    }

    pub(crate) fn test_config_base(&self) -> usize {
        self.config_base
    }

    pub(crate) fn can_cancel(&self) -> bool {
        self.phase != Phase::Uncertain
    }

    /// Only the containing grant invokes this after confirmed translation and
    /// capability publication, while device serialization excludes MMIO access.
    #[allow(clippy::result_large_err)]
    pub(crate) fn activate(self) -> Result<(), Self> {
        if self.phase != Phase::Ready {
            return Err(self);
        }
        let config = self.endpoint.cfg_ptr.lock();
        assert_eq!(config.reset_ranges, Some(self.ranges()), "reset claim replaced");
        let command = unsafe { config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>() };
        unsafe { write_volatile(command, self.command | 6) };
        if unsafe { read_volatile(command) } & 6 != 6 {
            // Unconfirmed activation retains the logical claim and complete grant.
            drop(config);
            let mut retained = self;
            retained.phase = Phase::Uncertain;
            return Err(retained);
        }
        // Use the same exact cell; no allocator, waiter or hardware poll follows.
        let mut config = config;
        config.reset_ranges = None;
        Ok(())
    }

    /// Known unstarted or confirmed-reset cancellation, after backend cleanup.
    #[allow(clippy::result_large_err)]
    pub(crate) fn cancel(self) -> Result<(), Self> {
        if !self.can_cancel() {
            return Err(self);
        }
        let mut config = self.endpoint.cfg_ptr.lock();
        assert_eq!(config.reset_ranges, Some(self.ranges()), "reset claim replaced");
        let command = unsafe { config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>() };
        unsafe { write_volatile(command, self.command & !4) };
        if unsafe { read_volatile(command) } & 4 != 0 {
            drop(config);
            let mut retained = self;
            retained.phase = Phase::Uncertain;
            return Err(retained);
        }
        config.reset_ranges = None;
        Ok(())
    }

    pub(crate) fn reset(&mut self) -> Result<(), ()> {
        self.reset_at(|| {})
    }

    fn reset_at(&mut self, before_wait: impl FnOnce()) -> Result<(), ()> {
        if self.phase != Phase::Claimed {
            return Err(());
        }
        // Fence uncertainty before any hardware mutation; error/abandonment
        // never restores config access or enables bus mastering.
        self.phase = Phase::Uncertain;
        let base = self.bars[0].ok_or(())?.base;
        {
            let config = self.endpoint.cfg_ptr.lock();
            assert_eq!(config.reset_ranges, Some(self.ranges()), "reset claim replaced");
            let bytes = config.ptr.as_ptr().cast::<u8>();
            unsafe { write_volatile(bytes.add(4).cast::<u16>(), self.command & !6) };
            if unsafe { read_volatile(bytes.add(4).cast::<u16>()) } & 6 != 0 {
                return Err(());
            }
            for (index, bar) in self.bars.iter().enumerate() {
                let Some(bar) = bar else {
                    continue;
                };
                let lower = unsafe { bytes.add(0x10 + index * 4).cast::<u32>() };
                let upper = unsafe { bytes.add(0x14 + index * 4).cast::<u32>() };
                unsafe {
                    write_volatile(lower, u32::MAX);
                    if bar.wide {
                        write_volatile(upper, u32::MAX);
                    }
                }
                let mask_low = unsafe { read_volatile(lower) } & !15;
                let mask_high = if bar.wide {
                    unsafe { read_volatile(upper) }
                } else {
                    u32::MAX
                };
                // No fallible work or callback may interrupt the temporary BAR
                // values before their exact originals are restored.
                unsafe {
                    write_volatile(lower, bar.low);
                    if bar.wide {
                        write_volatile(upper, bar.high);
                    }
                }
                let mask = u64::from(mask_low) | u64::from(mask_high) << 32;
                let size = (!mask).checked_add(1).ok_or(())?;
                if !size.is_power_of_two() || size > bar.bound as u64 {
                    return Err(());
                }
            }
            unsafe { write_volatile(bytes.add(4).cast::<u16>(), (self.command | 2) & !4) };
        }
        crate::memory::KERNEL_AS.lock().map_mmio_region(base, 4096).map_err(|_| ())?;
        let registers = unsafe { PAddr::from(base as u64).into_hhdm_mut::<u8>() };
        let configuration = unsafe { registers.add(0x14).cast::<u32>() };
        let status = unsafe { registers.add(0x1c).cast::<u32>() };
        let value = unsafe { read_volatile(configuration) };
        if value == u32::MAX {
            return Err(());
        }
        unsafe { write_volatile(configuration, value & !1) };
        tests::probe(self);
        before_wait();
        let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(100);
        loop {
            let status = unsafe { read_volatile(status) };
            if status != u32::MAX && status & 1 == 0 {
                break;
            }
            if crate::cpu::scheduler::monotonic_millis() >= deadline {
                return Err(());
            }
            core::hint::spin_loop();
        }
        self.phase = Phase::Ready;
        Ok(())
    }
}

fn find_function(
    function: &PcieFunction,
    bus: u8,
    device: u8,
    wanted: u32,
    depth: u8,
) -> Option<&PcieEndpoint> {
    match function {
        PcieFunction::Endpoint(endpoint)
            if (u32::from(bus) << 8
                | u32::from(device) << 3
                | u32::from(endpoint.number.get_inner()))
                == wanted =>
        {
            Some(endpoint)
        }
        PcieFunction::Bridge(next) if depth < MAX_PCIE_BRIDGE_DEPTH => {
            find_bus(next, wanted, depth + 1)
        }
        _ => None,
    }
}

fn find_bus(bus: &PcieBusSegment, wanted: u32, depth: u8) -> Option<&PcieEndpoint> {
    for device in &bus.devices {
        let endpoint = match device {
            PcieDevice::SingleFunc(single) => find_function(
                &single.function,
                bus.number,
                single.number.get_inner(),
                wanted,
                depth,
            ),
            PcieDevice::MultiFunc(multi) => multi.functions.iter().find_map(|function| {
                find_function(function, bus.number, multi.number.get_inner(), wanted, depth)
            }),
            PcieDevice::Empty => None,
        };
        if endpoint.is_some() {
            return endpoint;
        }
    }
    None
}

pub(crate) fn test_target(topology: &PcieTopology) -> Option<(usize, u32)> {
    if topology.segments.len() != 1 {
        return None;
    }
    let bus = &topology.segments[0].root_bus;
    for function in 0..256 {
        let requester = u32::from(bus.number) << 8 | function;
        let Some(endpoint) = find_bus(bus, requester, 0) else {
            continue;
        };
        let id = endpoint.identifier;
        if (id.vendor_id, id.device_id) != (0x1b36, 0x0010) {
            continue;
        }
        let Some(config) = endpoint.config() else {
            continue;
        };
        let bytes = config.as_ptr().cast::<u8>();
        let low = unsafe { read_volatile(bytes.add(0x10).cast::<u32>()) };
        let high = if low & 6 == 4 {
            unsafe { read_volatile(bytes.add(0x14).cast::<u32>()) }
        } else {
            0
        };
        let base = u64::from(low & !15) | u64::from(high) << 32;
        return usize::try_from(base).ok().filter(|&base| base != 0).map(|base| (base, requester));
    }
    None
}

pub(crate) fn supports_qemu_nvme(topology: &PcieTopology, requester: u32) -> bool {
    topology.segments.len() == 1
        && topology.segments[0].pcie_segment_group_num == 0
        && find_bus(&topology.segments[0].root_bus, requester, 0).is_some_and(|endpoint| {
            let id = endpoint.identifier;
            (id.vendor_id, id.device_id, id.class_code, id.subclass, id.prog_if)
                == (0x1b36, 0x0010, 1, 8, 2)
        })
}

/// Pre-driver rollback probe; no config mutation or hardware acknowledgement.
pub(crate) fn test_assert_disabled_config_available(topology: &PcieTopology) {
    assert_eq!(topology.segments.len(), 1);
    let bus = &topology.segments[0].root_bus;
    let mut found = false;
    for function in 0..256 {
        let requester = u32::from(bus.number) << 8 | function;
        let Some(endpoint) = find_bus(bus, requester, 0) else {
            continue;
        };
        if (endpoint.identifier.vendor_id, endpoint.identifier.device_id) != (0x1b36, 0x0010) {
            continue;
        }
        let config = endpoint.cfg_ptr.try_lock().expect("creation rollback holds PCI config");
        let command =
            unsafe { read_volatile(config.ptr.as_ptr().cast::<u8>().add(4).cast::<u16>()) };
        assert_eq!(command & 4, 0, "creation rejection enabled bus mastering");
        found = true;
    }
    assert!(found, "QEMU NVMe config fixture absent");
}

/// Caller holds device serialization through admission, excluding MMIO
/// grant/map/close between the range preflight and installing this claim.
pub(crate) fn prepare_qemu_nvme(
    topology: &PcieTopology,
    requester: u32,
    permitted: impl Fn(usize, usize) -> bool,
) -> Result<ResetSource<'_>, ()> {
    if !supports_qemu_nvme(topology, requester) {
        return Err(());
    }
    let endpoint = find_bus(&topology.segments[0].root_bus, requester, 0).ok_or(())?;
    let config = endpoint.cfg_ptr.lock();
    if config.reset_ranges.is_some() {
        return Err(());
    }
    let bytes = config.ptr.as_ptr().cast::<u8>();
    let raw_bars: [u32; 6] = core::array::from_fn(|index| unsafe {
        read_volatile(bytes.add(0x10 + index * 4).cast::<u32>())
    });
    let mut bars = [None; 6];
    let mut index = 0;
    while index < 6 {
        let low = raw_bars[index];
        if low & 1 != 0 {
            return Err(());
        }
        let wide = low & 6 == 4;
        if (low & 6 != 0 && !wide) || (wide && index == 5) {
            return Err(());
        }
        let high = if wide {
            raw_bars[index + 1]
        } else {
            0
        };
        let base = u64::from(low & !15) | u64::from(high) << 32;
        let bound = if index == 0 {
            0x4000
        } else {
            0x2000
        };
        if base != 0 {
            let base = usize::try_from(base).map_err(|_| ())?;
            if base.checked_add(bound).is_none() {
                return Err(());
            }
            bars[index] = Some(Bar {
                base,
                bound,
                low,
                high,
                wide,
            });
        }
        index += if wide {
            2
        } else {
            1
        };
    }
    bars[0].ok_or(())?;
    let config_base = usize::try_from(u64::from(topology.segments[0].ecam_paddr))
        .ok()
        .and_then(|base| base.checked_add((requester as usize).checked_mul(4096)?))
        .ok_or(())?;
    config_base.checked_add(4096).ok_or(())?;
    let command = unsafe { read_volatile(bytes.add(4).cast::<u16>()) };
    drop(config);
    // Range admission must not recursively enter this endpoint's config lock.
    // Device serialization prevents competing claim/MMIO publication, while
    // revalidation below rejects any intervening ordinary config mutation.
    for bar in bars.iter().flatten() {
        if !permitted(bar.base, bar.bound) {
            return Err(());
        }
    }
    if !permitted(config_base, 4096) {
        return Err(());
    }
    let mut config = endpoint.cfg_ptr.lock();
    if config.reset_ranges.is_some() {
        return Err(());
    }
    let bytes = config.ptr.as_ptr().cast::<u8>();
    if unsafe { read_volatile(bytes.add(4).cast::<u16>()) } != command {
        return Err(());
    }
    for (index, original) in raw_bars.iter().enumerate() {
        if unsafe { read_volatile(bytes.add(0x10 + index * 4).cast::<u32>()) } != *original {
            return Err(());
        }
    }
    let owner = ResetSource {
        endpoint,
        command,
        config_base,
        bars,
        phase: Phase::Claimed,
    };
    config.reset_ranges = Some(owner.ranges());
    CLAIM_PUBLISHED.store(true, Ordering::Release);
    Ok(owner)
}

/// Called under device serialization; range overlap is only an exclusion
/// query of the exact endpoint claim, never reset authority or a retry owner.
pub(crate) fn claimed_range(topology: &PcieTopology, base: usize, bytes: usize) -> bool {
    fn bus_claimed(bus: &PcieBusSegment, base: usize, end: usize) -> bool {
        fn function_claimed(function: &PcieFunction, base: usize, end: usize) -> bool {
            match function {
                PcieFunction::Endpoint(endpoint) => {
                    endpoint.cfg_ptr.lock().reset_ranges.is_some_and(|ranges| {
                        ranges
                            .into_iter()
                            .flatten()
                            .any(|(start, bytes)| start < end && base < start + bytes)
                    })
                }
                PcieFunction::Bridge(bus) => bus_claimed(bus, base, end),
                PcieFunction::Empty => false,
            }
        }
        bus.devices.iter().any(|device| match device {
            PcieDevice::SingleFunc(device) => function_claimed(&device.function, base, end),
            PcieDevice::MultiFunc(device) => {
                device.functions.iter().any(|function| function_claimed(function, base, end))
            }
            PcieDevice::Empty => false,
        })
    }
    let Some(end) = base.checked_add(bytes) else {
        return true;
    };
    topology.segments.iter().any(|segment| bus_claimed(&segment.root_bus, base, end))
}

pub(crate) mod tests;
pub(crate) fn test_admission() {
    tests::run();
}
