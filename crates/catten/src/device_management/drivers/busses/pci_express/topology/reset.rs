//! QEMU NVMe controller reset at an already retired IOMMU requester fence.
//! The config guard is retained through new-domain creation; rejected reset
//! keeps bus mastering disabled. This hardware adapter never grants authority.
use core::ptr::{
    read_volatile,
    write_volatile,
};

use super::*;
use crate::cpu::{
    isa::interface::memory::address::PhysicalAddress,
    multiprocessor::spin::mutex::MutexCore,
};

type ConfigGuard<'a> = lock_api::MutexGuard<'a, MutexCore, NonNull<PcieCfgSpace>>;

#[must_use]
pub(crate) struct ResetSource<'a> {
    config: ConfigGuard<'a>,
    command: u16,
    activated: bool,
}

impl ResetSource<'_> {
    pub(crate) fn activate(mut self) {
        unsafe {
            write_volatile(self.config.as_ptr().cast::<u8>().add(4).cast::<u16>(), self.command | 6)
        };
        self.activated = true;
    }
}

impl Drop for ResetSource<'_> {
    fn drop(&mut self) {
        if !self.activated {
            unsafe {
                write_volatile(
                    self.config.as_ptr().cast::<u8>().add(4).cast::<u16>(),
                    self.command & !4,
                )
            };
        }
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
        let config = endpoint.cfg_ptr.lock();
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
        let command = unsafe { read_volatile(config.as_ptr().cast::<u8>().add(4).cast::<u16>()) };
        assert_eq!(command & 4, 0, "creation rejection enabled bus mastering");
        found = true;
    }
    assert!(found, "QEMU NVMe config fixture absent");
}

pub(crate) fn qemu_nvme(
    topology: &PcieTopology,
    requester: u32,
    permitted: impl Fn(usize, usize) -> bool,
) -> Result<ResetSource<'_>, ()> {
    // Requester IDs currently have no segment component. Reject ambiguous
    // platforms rather than resetting a same-BDF endpoint on another segment.
    if topology.segments.len() != 1 || topology.segments[0].pcie_segment_group_num != 0 {
        return Err(());
    }
    let endpoint = find_bus(&topology.segments[0].root_bus, requester, 0).ok_or(())?;
    let id = endpoint.identifier;
    if (id.vendor_id, id.device_id, id.class_code, id.subclass, id.prog_if)
        != (0x1b36, 0x0010, 1, 8, 2)
    {
        return Err(());
    }
    let config = endpoint.cfg_ptr.lock();
    let bytes = config.as_ptr().cast::<u8>();
    let mut bars = [None; 6];
    let mut index = 0;
    while index < 6 {
        let low = unsafe { read_volatile(bytes.add(0x10 + index * 4).cast::<u32>()) };
        if low & 1 != 0 {
            return Err(());
        }
        let wide = low & 6 == 4;
        if low & 6 != 0 && !wide {
            return Err(());
        }
        if wide && index == 5 {
            return Err(());
        }
        let high = if wide {
            unsafe { read_volatile(bytes.add(0x14 + index * 4).cast::<u32>()) }
        } else {
            0
        };
        let base = u64::from(low & !15) | u64::from(high) << 32;
        // Only QEMU's default 16-KiB controller BAR and <=8-KiB MSI-X BARs
        // are supported. Probing below confirms these bounds before reset.
        let bound = if index == 0 {
            0x4000
        } else {
            0x2000
        };
        if base != 0 {
            let base = usize::try_from(base).map_err(|_| ())?;
            if base.checked_add(bound).is_none() || !permitted(base, bound) {
                return Err(());
            }
            bars[index] = Some((base, bound, low, high, wide));
        }
        index += if wide {
            2
        } else {
            1
        };
    }
    let (base, _, _, _, _) = bars[0].ok_or(())?;
    let command = unsafe { read_volatile(bytes.add(4).cast::<u16>()) };
    let owner = ResetSource {
        config,
        command,
        activated: false,
    };
    // Disable memory decode while sizing; BME stays disabled through reset and
    // translation publication. BAR writes are restored without fallible work.
    unsafe { write_volatile(bytes.add(4).cast::<u16>(), command & !6) };
    if unsafe { read_volatile(bytes.add(4).cast::<u16>()) } & 6 != 0 {
        return Err(());
    }
    for (index, bar) in bars.iter().enumerate() {
        let Some((_, bound, low, high, wide)) = *bar else {
            continue;
        };
        let lower = unsafe { bytes.add(0x10 + index * 4).cast::<u32>() };
        let upper = unsafe { bytes.add(0x14 + index * 4).cast::<u32>() };
        unsafe {
            write_volatile(lower, u32::MAX);
            if wide {
                write_volatile(upper, u32::MAX);
            }
        }
        let mask_low = unsafe { read_volatile(lower) } & !15;
        let mask_high = if wide {
            unsafe { read_volatile(upper) }
        } else {
            u32::MAX
        };
        unsafe {
            write_volatile(lower, low);
            if wide {
                write_volatile(upper, high);
            }
        }
        let mask = u64::from(mask_low) | u64::from(mask_high) << 32;
        let size = (!mask).checked_add(1).ok_or(())?;
        if !size.is_power_of_two() || size > bound as u64 {
            return Err(());
        }
    }
    unsafe { write_volatile(bytes.add(4).cast::<u16>(), (command | 2) & !4) };
    crate::memory::KERNEL_AS.lock().map_mmio_region(base, 4096).map_err(|_| ())?;
    let registers = unsafe { PAddr::from(base as u64).into_hhdm_mut::<u8>() };
    let configuration = unsafe { registers.add(0x14).cast::<u32>() };
    let status = unsafe { registers.add(0x1c).cast::<u32>() };
    let value = unsafe { read_volatile(configuration) };
    if value == u32::MAX {
        return Err(());
    }
    unsafe { write_volatile(configuration, value & !1) };
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
    Ok(owner)
}
