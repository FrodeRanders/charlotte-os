use alloc::{
    boxed::Box,
    vec::Vec,
};
use core::{
    ops::Deref,
    ptr::NonNull,
};

use crate::{
    cpu::{
        isa::interface::memory::address::VirtualAddress,
        multiprocessor::spin::mutex::Mutex as SpinMutex,
    },
    device_management::drivers::busses::pci_express::{
        Error,
        MAX_DEVICES_PER_BUS,
        MAX_FUNCTIONS_PER_DEVICE,
        device_class::PciIdentifier,
        ecam,
        ecam::pcie::PcieCfgSpace,
    },
    logln,
    memory::{
        PAddr,
        VAddr,
    },
};

pub type PcieSegmentGroupNum = u16;
pub type PcieBusSegmentNum = u8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcieDeviceNum(u8);
impl TryFrom<u8> for PcieDeviceNum {
    type Error = ();

    fn try_from(num: u8) -> Result<Self, Self::Error> {
        if num < MAX_DEVICES_PER_BUS as u8 {
            Ok(PcieDeviceNum(num))
        } else {
            Err(())
        }
    }
}
impl PcieDeviceNum {
    pub fn get_inner(self) -> u8 {
        self.0
    }
}
impl Deref for PcieDeviceNum {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcieFunctionNum(u8);
impl TryFrom<u8> for PcieFunctionNum {
    type Error = ();

    fn try_from(num: u8) -> Result<Self, Self::Error> {
        if num < MAX_FUNCTIONS_PER_DEVICE as u8 {
            Ok(PcieFunctionNum(num))
        } else {
            Err(())
        }
    }
}
impl PcieFunctionNum {
    pub fn get_inner(self) -> u8 {
        self.0
    }
}
impl Deref for PcieFunctionNum {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct PcieLocation {
    segment_group: PcieSegmentGroupNum,
    bus_segment: PcieBusSegmentNum,
    device: PcieDeviceNum,
    function: PcieFunctionNum,
}

impl PcieLocation {
    const BUS_SEGMENT_SHIFT: usize = 20;
    /* Each bus occupies 1 MiB of ECAM address space */
    const DEVICE_SHIFT: usize = 15;
    /* Each device occupies 32 KiB of ECAM address space */
    const FUNCTION_SHIFT: usize = 12;

    pub fn new(
        segment_group: PcieSegmentGroupNum,
        bus_segment: PcieBusSegmentNum,
        device: PcieDeviceNum,
        function: PcieFunctionNum,
    ) -> Self {
        PcieLocation {
            segment_group,
            bus_segment,
            device,
            function,
        }
    }

    /* Each function occupies 4 KiB of ECAM address space */
    pub fn get_ecam_offset(&self) -> usize {
        let bus_offset = (self.bus_segment as usize) << Self::BUS_SEGMENT_SHIFT; /* Each bus occupies 1 MiB of ECAM address space */
        let device_offset = (self.device.get_inner() as usize) << Self::DEVICE_SHIFT; /* Each device occupies 32 KiB of ECAM address space */
        let function_offset = (self.function.get_inner() as usize) << Self::FUNCTION_SHIFT; /* Each function occupies 4 KiB of ECAM address space */
        bus_offset + device_offset + function_offset
    }
}

#[derive(Debug)]
pub struct PcieTopology {
    segments: Vec<PcieSegmentGroup>,
}

impl PcieTopology {
    pub fn new(segments: Vec<PcieSegmentGroup>) -> Self {
        PcieTopology {
            segments,
        }
    }

    #[allow(dead_code)]
    pub(super) fn get_cfg_space_vaddr(
        &self,
        segment_group: PcieSegmentGroupNum,
        bus_segment: PcieBusSegmentNum,
        device_num: PcieDeviceNum,
        function_num: PcieFunctionNum,
    ) -> Result<VAddr, Error> {
        let segment_group = self
            .segments
            .iter()
            .find(|sg| sg.pcie_segment_group_num == segment_group)
            .ok_or(Error::InvalidLocation)?;
        if bus_segment < segment_group.start_bus_num || bus_segment > segment_group.end_bus_num {
            return Err(Error::InvalidLocation);
        }
        let location = PcieLocation::new(
            segment_group.pcie_segment_group_num,
            bus_segment,
            device_num,
            function_num,
        );
        Ok(segment_group.ecam_vaddr + location.get_ecam_offset())
    }
}

#[derive(Debug)]
pub struct PcieSegmentGroup {
    pcie_segment_group_num: PcieSegmentGroupNum,
    ecam_vaddr: VAddr, /* Virtual address where this segment's ECAM is mapped in the kernel's
                        * address space */
    start_bus_num: PcieBusSegmentNum,
    end_bus_num: PcieBusSegmentNum,
    root_bus: Box<PcieBusSegment>, /* Root bus of this segment's topology; the rest of the
                                    * topology
                                    * can be traversed from here */
}

impl PcieSegmentGroup {
    pub fn new(
        pcie_segment_group_num: PcieSegmentGroupNum,
        ecam_paddr: PAddr,
        start_bus_num: PcieBusSegmentNum,
        end_bus_num: PcieBusSegmentNum,
    ) -> Self {
        let ecam_vaddr = ecam::map_ecam(ecam_paddr);
        PcieSegmentGroup {
            pcie_segment_group_num,
            ecam_vaddr,
            start_bus_num,
            end_bus_num,
            root_bus: PcieBusSegment::new_boxed(ecam_vaddr, pcie_segment_group_num, start_bus_num),
        }
    }
}

#[derive(Debug)]
pub struct PcieBusSegment {
    number: PcieBusSegmentNum,
    devices: Vec<PcieDevice>,
}

impl PcieBusSegment {
    /// Allocate the bus before descending through any bridges it contains.
    ///
    /// A bus used to own a comparatively large fixed device array. Constructing
    /// that array recursively retained one copy on the kernel stack for every
    /// bridge level. VMware's PCIe topology is deep enough to exhaust that
    /// stack, while QEMU's shallow topology did not expose the problem. Store
    /// only occupied slots; every device retains its PCI device number.
    fn new_boxed(
        ecam_vaddr: VAddr,
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
    ) -> Box<Self> {
        logln!(
            "[drivers::busses::pci_express] Enumerating PCIe bus segment {} of segment group {}",
            bus_num,
            segment_group_num
        );
        let mut bus = Box::new(PcieBusSegment {
            number: bus_num,
            devices: Vec::new(),
        });
        logln!(
            "[drivers::busses::pci_express] Initialized device array for bus segment {} of \
             segment group {}. Starting device enumeration...",
            bus_num,
            segment_group_num
        );
        for device_num in 0..MAX_DEVICES_PER_BUS {
            let device = PcieDevice::new(ecam_vaddr, segment_group_num, bus_num, device_num as u8);
            if !matches!(device, PcieDevice::Empty) {
                bus.devices.push(device);
            }
        }
        bus
    }
}

#[derive(Debug)]
pub enum PcieDevice {
    Empty,
    SingleFunc(PcieSingleFuncDevice),
    MultiFunc(PcieMultiFuncDevice),
}

impl PcieDevice {
    fn new(
        ecam_vaddr: VAddr,
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
    ) -> Self {
        let cfg_space_vaddr = ecam_vaddr
            + PcieLocation::new(
                segment_group_num,
                bus_num,
                PcieDeviceNum(device_num),
                PcieFunctionNum(0),
            )
            .get_ecam_offset();

        let cfg_space = unsafe { &*cfg_space_vaddr.into_ptr::<PcieCfgSpace>() };
        if !cfg_space.has_device_present() {
            PcieDevice::Empty
        } else if cfg_space.device_is_multifunction() {
            PcieDevice::MultiFunc(PcieMultiFuncDevice::new(
                ecam_vaddr,
                segment_group_num,
                bus_num,
                device_num,
            ))
        } else {
            PcieDevice::SingleFunc(PcieSingleFuncDevice::new(
                ecam_vaddr,
                segment_group_num,
                bus_num,
                device_num,
            ))
        }
    }
}

#[derive(Debug)]
pub struct PcieSingleFuncDevice {
    number: PcieDeviceNum,
    function: PcieFunction,
}

impl PcieSingleFuncDevice {
    fn new(
        ecam_vaddr: VAddr,
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
    ) -> Self {
        PcieSingleFuncDevice {
            number: PcieDeviceNum(device_num),
            function: PcieFunction::new(
                ecam_vaddr,
                segment_group_num,
                bus_num,
                device_num,
                PcieFunctionNum(0),
            ),
        }
    }
}

#[derive(Debug)]
pub struct PcieMultiFuncDevice {
    number: PcieDeviceNum,
    functions: [PcieFunction; MAX_FUNCTIONS_PER_DEVICE],
}

impl PcieMultiFuncDevice {
    fn new(
        ecam_vaddr: VAddr,
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
    ) -> Self {
        let mut functions: [PcieFunction; MAX_FUNCTIONS_PER_DEVICE] =
            [const { PcieFunction::Empty }; MAX_FUNCTIONS_PER_DEVICE];
        for (i, function) in functions.iter_mut().enumerate() {
            *function = PcieFunction::new(
                ecam_vaddr,
                segment_group_num,
                bus_num,
                device_num,
                PcieFunctionNum::try_from(i as u8).unwrap(),
            );
        }

        PcieMultiFuncDevice {
            number: PcieDeviceNum(device_num),
            functions,
        }
    }
}

/* Number of 32-bit BARs */
const MAX_BAR_NUM: usize = 6;
/* Number of 64-bit BARs */
const MAX_EXT_BARS: usize = 3;

#[derive(Clone, Copy)]
#[allow(dead_code)]
union BarIoAddrs {
    pub bar32: [Option<VAddr>; MAX_BAR_NUM],
    pub bar64: [Option<VAddr>; MAX_EXT_BARS],
}

impl core::fmt::Debug for BarIoAddrs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        unsafe { write!(f, "BarIoAddrs {{ bar32: {:?}, bar64: {:?} }}", self.bar32, self.bar64) }
    }
}

#[derive(Debug)]
pub enum PcieFunction {
    Empty,
    Endpoint(Box<PcieEndpoint>), /* If this function is a normal endpoint device, then it has
                                  * no bus segment behind it and
                                  * can be represented as an endpoint struct containing its
                                  * relevant config space info
                                  * and BAR addresses */
    Bridge(Box<PcieBusSegment>), /* If this function is a bridge, then it has a bus segment
                                  * behind it which can be
                                  * traversed like the root bus segments in the
                                  * topology */
}

impl PcieFunction {
    fn new(
        ecam_vaddr: VAddr,
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
        function_num: PcieFunctionNum,
    ) -> Self {
        let cfg_space_vaddr = ecam_vaddr
            + PcieLocation::new(
                segment_group_num,
                bus_num,
                PcieDeviceNum(device_num),
                function_num,
            )
            .get_ecam_offset();
        let cfg_space = unsafe { &*(cfg_space_vaddr.into_ptr::<PcieCfgSpace>()) };
        if !cfg_space.has_device_present() {
            PcieFunction::Empty
        } else if cfg_space.device_is_bridge() {
            let secondary_bus_segment_number =
                unsafe { cfg_space.header.bridge.get_secondary_bus_num() };
            PcieFunction::Bridge(PcieBusSegment::new_boxed(
                ecam_vaddr,
                segment_group_num,
                secondary_bus_segment_number,
            ))
        } else {
            PcieFunction::Endpoint(Box::new(PcieEndpoint::new(
                ecam_vaddr,
                segment_group_num,
                bus_num,
                device_num,
                function_num,
            )))
        }
    }
}

#[derive(Debug)]
pub struct PcieEndpoint {
    #[allow(dead_code)]
    number: PcieFunctionNum,
    identifier: PciIdentifier,
    /* Raw pointer to this function's configuration space in the kernel's address space;
     * used for reading/writing config space registers inside this PCIe bus driver ONLY
     * other drivers and the rest of the kernel should use safe functions exposed by this bus
     * driver */
    cfg_ptr: SpinMutex<NonNull<PcieCfgSpace>>,
}

impl PcieEndpoint {
    fn new(
        ecam_vaddr: VAddr,
        segment_group_num: PcieSegmentGroupNum,
        bus_num: PcieBusSegmentNum,
        device_num: u8,
        function_num: PcieFunctionNum,
    ) -> Self {
        let cfg_space_vaddr = ecam_vaddr
            + PcieLocation::new(
                segment_group_num,
                bus_num,
                PcieDeviceNum(device_num),
                function_num,
            )
            .get_ecam_offset();
        let cfg_space = cfg_space_vaddr.into_ptr::<PcieCfgSpace>();
        let identifier = unsafe { (*cfg_space).header.common.get_identifier() };

        PcieEndpoint {
            number: function_num,
            identifier,
            cfg_ptr: SpinMutex::new(
                NonNull::new(cfg_space_vaddr.into_mut())
                    .expect("Invalid PCIe config space pointer"),
            ),
        }
    }
}

unsafe impl Send for PcieEndpoint {}
unsafe impl Sync for PcieEndpoint {}

/// Number of spaces each level of the topology tree is indented by when rendered for logging.
const TREE_INDENT_STEP: usize = 2;

impl core::fmt::Display for PcieTopology {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.segments.is_empty() {
            return write!(f, "  (no PCIe segment groups)");
        }
        for segment in &self.segments {
            segment.fmt_tree(f, TREE_INDENT_STEP)?;
        }
        Ok(())
    }
}

impl PcieSegmentGroup {
    fn fmt_tree(&self, f: &mut core::fmt::Formatter<'_>, indent: usize) -> core::fmt::Result {
        let ecam: u64 = self.ecam_vaddr.into();
        writeln!(
            f,
            "{:indent$}Segment Group {} (ECAM @ {:#018x}, buses {:#04x}-{:#04x})",
            "",
            self.pcie_segment_group_num,
            ecam,
            self.start_bus_num,
            self.end_bus_num,
            indent = indent
        )?;
        self.root_bus.fmt_tree(f, indent + TREE_INDENT_STEP)
    }
}

impl PcieBusSegment {
    fn fmt_tree(&self, f: &mut core::fmt::Formatter<'_>, indent: usize) -> core::fmt::Result {
        writeln!(f, "{:indent$}Bus {:#04x}", "", self.number, indent = indent)?;
        let child_indent = indent + TREE_INDENT_STEP;
        // Label the columns directly above the rows they describe (column widths must match the
        // formatting in `PcieFunction::fmt_tree`). Skipped for buses with no occupied slots.
        if self.devices.iter().any(|device| !matches!(device, PcieDevice::Empty)) {
            writeln!(
                f,
                "{:indent$}{:<7}  {:<9}  Class (cc:sc:pi)",
                "",
                "B:D.F",
                "VID:DID",
                indent = child_indent
            )?;
        }
        for device in &self.devices {
            match device {
                PcieDevice::Empty => {}
                PcieDevice::SingleFunc(dev) => {
                    dev.function.fmt_tree(
                        f,
                        child_indent,
                        self.number,
                        dev.number.get_inner(),
                        0,
                    )?;
                }
                PcieDevice::MultiFunc(dev) => {
                    for (func_num, function) in dev.functions.iter().enumerate() {
                        function.fmt_tree(
                            f,
                            child_indent,
                            self.number,
                            dev.number.get_inner(),
                            func_num as u8,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl PcieFunction {
    /// Renders a single function as one line of the topology tree, prefixed with its
    /// `bus:device.function` (BDF) address. Bridges additionally recurse into the bus segment
    /// behind them.
    fn fmt_tree(
        &self,
        f: &mut core::fmt::Formatter<'_>,
        indent: usize,
        bus: PcieBusSegmentNum,
        device: u8,
        function: u8,
    ) -> core::fmt::Result {
        match self {
            PcieFunction::Empty => Ok(()),
            PcieFunction::Endpoint(endpoint) => writeln!(
                f,
                "{:indent$}{:02x}:{:02x}.{:x}  {}",
                "",
                bus,
                device,
                function,
                endpoint.identifier,
                indent = indent
            ),
            PcieFunction::Bridge(secondary_bus) => {
                writeln!(
                    f,
                    "{:indent$}{:02x}:{:02x}.{:x}  PCI-to-PCI bridge -> bus {:#04x}",
                    "",
                    bus,
                    device,
                    function,
                    secondary_bus.number,
                    indent = indent
                )?;
                secondary_bus.fmt_tree(f, indent + TREE_INDENT_STEP)
            }
        }
    }
}

/// Add every secondary bus reachable through this device list to a depth-first
/// traversal. A PCIe root-port device is commonly multifunction (VMware puts
/// NVMe behind function 0 and E1000E behind function 1), so inspecting only
/// the first function silently hides otherwise enumerated endpoints.
fn push_bridge_buses<'a>(devices: &'a [PcieDevice], stack: &mut Vec<&'a PcieBusSegment>) {
    for device in devices {
        match device {
            PcieDevice::SingleFunc(single) => {
                if let PcieFunction::Bridge(bus) = &single.function {
                    stack.push(bus);
                }
            }
            PcieDevice::MultiFunc(multi) => {
                for function in &multi.functions {
                    if let PcieFunction::Bridge(bus) = function {
                        stack.push(bus);
                    }
                }
            }
            PcieDevice::Empty => {}
        }
    }
}

/// Scan the PCI topology for the first virtio-net device, configure MSI-X
/// vector zero when possible, and return its delegated-device coordinates.
pub fn lookup_first_virtio_net(
    topology: &PcieTopology,
) -> Option<(u64, usize, u32, u32, Option<u64>)> {
    for group in &topology.segments {
        // Walk the bus hierarchy starting at the root bus of each segment.
        let mut stack = alloc::vec![&*group.root_bus];
        while let Some(bus) = stack.pop() {
            for dev in &bus.devices {
                let (ep, device, function) = match dev {
                    PcieDevice::SingleFunc(sfd) => match &sfd.function {
                        PcieFunction::Endpoint(ep) => {
                            (ep, sfd.number.get_inner(), ep.number.get_inner())
                        }
                        _ => continue,
                    },
                    PcieDevice::MultiFunc(mfd) => {
                        match mfd.functions.first().and_then(|f| {
                            if let PcieFunction::Endpoint(ep) = f {
                                Some((ep, mfd.number.get_inner(), ep.number.get_inner()))
                            } else {
                                None
                            }
                        }) {
                            Some(ep) => ep,
                            None => continue,
                        }
                    }
                    _ => continue,
                };
                if ep.identifier.vendor_id != 0x1af4 {
                    continue;
                }
                if ep.identifier.device_id < 0x1000 || ep.identifier.device_id > 0x107f {
                    continue;
                }
                if (ep.identifier.class_code, ep.identifier.subclass) != (0x02, 0x00) {
                    continue;
                }
                let requester_id =
                    ((bus.number as u32) << 8) | ((device as u32) << 3) | function as u32;
                let cfg = ep.cfg_ptr.lock();
                let header = unsafe { &(*cfg.as_ptr()).header.endpoint };
                // QEMU places all modern virtio regions in BAR 4. Verify the
                // vendor capability instead of mistaking the transitional
                // legacy I/O BAR for a DMA-isolatable transport.
                let cfg_bytes = cfg.as_ptr().cast::<u8>();
                let mut capability =
                    header.get_capabilities_offset().map(|offset| offset as usize).unwrap_or(0);
                let mut modern_bar = None;
                for _ in 0..48 {
                    if capability < 0x40 || capability + 16 > 0x100 {
                        break;
                    }
                    let id = unsafe { core::ptr::read_volatile(cfg_bytes.add(capability)) };
                    let next =
                        unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 1)) } as usize;
                    let len =
                        unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 2)) } as usize;
                    let cfg_type =
                        unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 3)) };
                    if id == 0x09 && len >= 16 {
                        let bar =
                            unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 4)) };
                        let offset = unsafe {
                            core::ptr::read_unaligned(cfg_bytes.add(capability + 8).cast::<u32>())
                        };
                        let length = unsafe {
                            core::ptr::read_unaligned(cfg_bytes.add(capability + 12).cast::<u32>())
                        };
                        logln!(
                            "[net] virtio cap type={} bar={} offset={:#x} length={:#x}",
                            cfg_type,
                            bar,
                            offset,
                            length
                        );
                    }
                    if id == 0x09 && len >= 16 && cfg_type == 1 {
                        modern_bar = Some(unsafe {
                            core::ptr::read_volatile(cfg_bytes.add(capability + 4))
                        });
                    }
                    if next == 0 || next == capability {
                        break;
                    }
                    capability = next;
                }
                let bar_index = modern_bar? as usize;
                if bar_index >= 6 {
                    continue;
                }
                let bar = header.bar(bar_index) as u64;
                if bar & 1 != 0 {
                    continue;
                }
                let phys_base = if bar & 0x4 != 0 {
                    if bar_index + 1 >= 6 {
                        continue;
                    }
                    (bar & 0xffff_fff0) | ((header.bar(bar_index + 1) as u64 & 0xffff_ffff) << 32)
                } else {
                    bar & 0xffff_fff0
                };
                let legacy_irq = header.interrupt_line() as u32;
                if phys_base != 0 {
                    if crate::device::msi_available()
                        && let Some(message) = crate::device::allocate_msi(requester_id)
                        && crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msix::program_vector0(
                            cfg.as_ptr(),
                            message,
                        )
                        .is_ok()
                    {
                        logln!(
                            "[net] MSI-X vector 0: address={:#x} data={} intid={}",
                            message.address,
                            message.data,
                            message.intid
                        );
                        return Some((phys_base, 4, message.intid, requester_id, Some(message.address)));
                    }
                    return Some((phys_base, 4, legacy_irq, requester_id, None));
                }
            }
            push_bridge_buses(&bus.devices, &mut stack);
        }
    }
    None
}

/// Scan the PCI topology for an Intel 82574L controller, the device emulated
/// by VMware's `e1000e` virtual NIC and QEMU's `e1000e` model. Configure MSI-X
/// vector zero when available and return BAR0 plus the authority needed by the
/// userspace driver.
pub fn lookup_first_e1000e(topology: &PcieTopology) -> Option<(u64, usize, u32, u32, Option<u64>)> {
    for group in &topology.segments {
        let mut stack = alloc::vec![&*group.root_bus];
        while let Some(bus) = stack.pop() {
            for dev in &bus.devices {
                let (ep, device, function) = match dev {
                    PcieDevice::SingleFunc(sfd) => match &sfd.function {
                        PcieFunction::Endpoint(ep) => {
                            (ep, sfd.number.get_inner(), ep.number.get_inner())
                        }
                        _ => continue,
                    },
                    PcieDevice::MultiFunc(mfd) => match mfd.functions.first().and_then(|f| {
                        if let PcieFunction::Endpoint(ep) = f {
                            Some((ep, mfd.number.get_inner(), ep.number.get_inner()))
                        } else {
                            None
                        }
                    }) {
                        Some(ep) => ep,
                        None => continue,
                    },
                    _ => continue,
                };
                if ep.identifier.vendor_id != 0x8086
                    || ep.identifier.device_id != 0x10d3
                    || (ep.identifier.class_code, ep.identifier.subclass) != (0x02, 0x00)
                {
                    continue;
                }

                let requester_id =
                    ((bus.number as u32) << 8) | ((device as u32) << 3) | function as u32;
                let cfg = ep.cfg_ptr.lock();
                let header = unsafe { &(*cfg.as_ptr()).header.endpoint };
                let bar0 = header.bar(0) as u64;
                logln!(
                    "[e1000e] found Intel 82574L at {:02x}:{:02x}.{} (BAR0={:#x}, IRQ line={})",
                    bus.number,
                    device,
                    function,
                    bar0,
                    header.interrupt_line()
                );
                if bar0 & 1 != 0 {
                    continue;
                }
                let phys_base = if bar0 & 0x4 != 0 {
                    (bar0 & 0xffff_fff0) | ((header.bar(1) as u64) << 32)
                } else {
                    bar0 & 0xffff_fff0
                };
                if phys_base == 0 {
                    continue;
                }

                let legacy_irq = header.interrupt_line() as u32;
                if crate::device::msi_available()
                    && let Some(message) = crate::device::allocate_msi(requester_id)
                    && crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msix::program_vector0(
                        cfg.as_ptr(),
                        message,
                    )
                    .is_ok()
                {
                    logln!(
                        "[e1000e] MSI-X vector 0: address={:#x} data={} intid={}",
                        message.address,
                        message.data,
                        message.intid
                    );
                    // BAR0 is a 128-KiB register aperture on the 82574L.
                    return Some((phys_base, 32, message.intid, requester_id, Some(message.address)));
                }

                // Keep the fallback usable on platforms without an MSI
                // allocator. MSI-X setup normally enables these command bits.
                let cfg_bytes = cfg.as_ptr().cast::<u8>();
                let command =
                    unsafe { core::ptr::read_volatile(cfg_bytes.add(0x04).cast::<u16>()) };
                unsafe {
                    core::ptr::write_volatile(
                        cfg_bytes.add(0x04).cast::<u16>(),
                        command | (1 << 1) | (1 << 2),
                    )
                };
                return Some((phys_base, 32, legacy_irq, requester_id, None));
            }
            push_bridge_buses(&bus.devices, &mut stack);
        }
    }
    None
}

/// Find the first NVMe controller and configure MSI-X vector zero when the
/// platform exposes a GICv2m MSI frame. Falls back to the legacy interrupt-line
/// value when MSI-X is unavailable.
pub fn lookup_first_nvme(topology: &PcieTopology) -> Option<(u64, u32, u32, Option<u64>)> {
    for group in &topology.segments {
        let mut stack = alloc::vec![&*group.root_bus];
        while let Some(bus) = stack.pop() {
            for dev in &bus.devices {
                let (ep, device, function) = match dev {
                    PcieDevice::SingleFunc(sfd) => match &sfd.function {
                        PcieFunction::Endpoint(ep) => {
                            (ep, sfd.number.get_inner(), ep.number.get_inner())
                        }
                        _ => continue,
                    },
                    PcieDevice::MultiFunc(mfd) => {
                        match mfd.functions.first().and_then(|f| {
                            if let PcieFunction::Endpoint(ep) = f {
                                Some((ep, mfd.number.get_inner(), ep.number.get_inner()))
                            } else {
                                None
                            }
                        }) {
                            Some(ep) => ep,
                            None => continue,
                        }
                    }
                    _ => continue,
                };
                let requester_id =
                    ((bus.number as u32) << 8) | ((device as u32) << 3) | function as u32;
                let (class, subclass, prog_if) =
                    (ep.identifier.class_code, ep.identifier.subclass, ep.identifier.prog_if);
                if (class, subclass, prog_if) != (0x01, 0x08, 0x02) {
                    continue;
                }
                let cfg = ep.cfg_ptr.lock();
                let (phys_base, legacy_irq) = {
                    let header = unsafe { &(*cfg.as_ptr()).header.endpoint };
                    let bar0 = header.bar(0) as u64;
                    let bar0_phys = if bar0 & 0x4 != 0 {
                        let bar1 = header.bar(1) as u64;
                        (bar0 & 0xffff_fff0) | ((bar1 & 0xffff_ffff) << 32)
                    } else {
                        bar0 & 0xffff_fff0
                    };
                    (bar0_phys, header.interrupt_line() as u32)
                };
                if phys_base != 0 {
                    // Only program MSI-X when the kernel's MSI mechanism (the
                    // GIC ITS/v2m on AArch64, the LAPIC on x86_64) is actually
                    // available.
                    if crate::device::msi_available()
                        && let Some(message) = crate::device::allocate_msi(requester_id)
                        && crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msix::program_vector0(
                            cfg.as_ptr(),
                            message,
                        )
                        .is_ok()
                        {
                            logln!(
                                "[nvme] MSI-X vector 0: address={:#x} data={} intid={}",
                                message.address,
                                message.data,
                                message.intid
                            );
                            return Some((
                                phys_base,
                                message.intid,
                                requester_id,
                                Some(message.address),
                            ));
                        }
                    return Some((phys_base, legacy_irq, requester_id, None));
                }
            }
            push_bridge_buses(&bus.devices, &mut stack);
        }
    }
    None
}

/// Find the first AHCI SATA controller and return its HBA (ABAR) MMIO base,
/// legacy interrupt line, and requester id. AHCI completion is polled by the
/// driver, so no MSI/MSI-X vector is configured here.
pub fn lookup_first_ahci(topology: &PcieTopology) -> Option<(u64, u32, u32, Option<u64>)> {
    for group in &topology.segments {
        let mut stack = alloc::vec![&*group.root_bus];
        while let Some(bus) = stack.pop() {
            for dev in &bus.devices {
                let functions: alloc::vec::Vec<(&PcieEndpoint, u8, u8)> = match dev {
                    PcieDevice::SingleFunc(sfd) => {
                        let mut v = alloc::vec::Vec::new();
                        if let PcieFunction::Endpoint(ep) = &sfd.function {
                            v.push((ep.as_ref(), sfd.number.get_inner(), ep.number.get_inner()));
                        }
                        v
                    }
                    PcieDevice::MultiFunc(mfd) => mfd
                        .functions
                        .iter()
                        .enumerate()
                        .filter_map(|(func, function)| match function {
                            PcieFunction::Endpoint(ep) => {
                                Some((ep.as_ref(), mfd.number.get_inner(), func as u8))
                            }
                            _ => None,
                        })
                        .collect(),
                    _ => alloc::vec::Vec::new(),
                };
                for (ep, device, function) in functions {
                    let requester_id =
                        ((bus.number as u32) << 8) | ((device as u32) << 3) | function as u32;
                    if (ep.identifier.class_code, ep.identifier.subclass, ep.identifier.prog_if)
                        != (0x01, 0x06, 0x01)
                    {
                        continue;
                    }
                    let cfg = ep.cfg_ptr.lock();
                    let header = unsafe { &(*cfg.as_ptr()).header.endpoint };
                    // The HBA register block (ABAR) is memory BAR 5.
                    let bar5 = header.bar(5) as u64;
                    let abar = if bar5 & 0x4 != 0 {
                        let bar6 = header.bar(6) as u64;
                        (bar5 & 0xffff_fff0) | ((bar6 & 0xffff_ffff) << 32)
                    } else {
                        bar5 & 0xffff_fff0
                    };
                    let legacy_irq = header.interrupt_line() as u32;
                    if abar != 0 {
                        return Some((abar, legacy_irq, requester_id, None));
                    }
                }
            }
            push_bridge_buses(&bus.devices, &mut stack);
        }
    }
    None
}

/// Find the first virtio-blk device and return its modern (BAR 4) MMIO base,
/// legacy interrupt line, and requester id. Completion is polled through the
/// used ring, so no MSI/MSI-X vector is configured.
pub fn lookup_first_virtio_blk(topology: &PcieTopology) -> Option<(u64, u32, u32, Option<u64>)> {
    for group in &topology.segments {
        let mut stack = alloc::vec![&*group.root_bus];
        while let Some(bus) = stack.pop() {
            for dev in &bus.devices {
                let functions: alloc::vec::Vec<(&PcieEndpoint, u8, u8)> = match dev {
                    PcieDevice::SingleFunc(sfd) => {
                        let mut v = alloc::vec::Vec::new();
                        if let PcieFunction::Endpoint(ep) = &sfd.function {
                            v.push((ep.as_ref(), sfd.number.get_inner(), ep.number.get_inner()));
                        }
                        v
                    }
                    PcieDevice::MultiFunc(mfd) => mfd
                        .functions
                        .iter()
                        .enumerate()
                        .filter_map(|(func, function)| match function {
                            PcieFunction::Endpoint(ep) => {
                                Some((ep.as_ref(), mfd.number.get_inner(), func as u8))
                            }
                            _ => None,
                        })
                        .collect(),
                    _ => alloc::vec::Vec::new(),
                };
                for (ep, device, function) in functions {
                    if ep.identifier.vendor_id != 0x1af4 {
                        continue;
                    }
                    if (ep.identifier.class_code, ep.identifier.subclass) != (0x01, 0x00) {
                        continue;
                    }
                    let requester_id =
                        ((bus.number as u32) << 8) | ((device as u32) << 3) | function as u32;
                    let cfg = ep.cfg_ptr.lock();
                    let header = unsafe { &(*cfg.as_ptr()).header.endpoint };
                    // Locate the modern transport BAR via the common-config
                    // vendor capability (type 1).
                    let cfg_bytes = cfg.as_ptr().cast::<u8>();
                    let mut capability =
                        header.get_capabilities_offset().map(|offset| offset as usize).unwrap_or(0);
                    let mut modern_bar = None;
                    for _ in 0..48 {
                        if capability < 0x40 || capability + 16 > 0x100 {
                            break;
                        }
                        let id = unsafe { core::ptr::read_volatile(cfg_bytes.add(capability)) };
                        let next =
                            unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 1)) }
                                as usize;
                        let len = unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 2)) }
                            as usize;
                        let cfg_type =
                            unsafe { core::ptr::read_volatile(cfg_bytes.add(capability + 3)) };
                        if id == 0x09 && len >= 16 && cfg_type == 1 {
                            modern_bar = Some(unsafe {
                                core::ptr::read_volatile(cfg_bytes.add(capability + 4))
                            });
                        }
                        if next == 0 || next == capability {
                            break;
                        }
                        capability = next;
                    }
                    let Some(bar_index) = modern_bar else {
                        continue;
                    };
                    let bar_index = bar_index as usize;
                    if bar_index >= 6 {
                        continue;
                    }
                    let bar = header.bar(bar_index) as u64;
                    if bar & 1 != 0 {
                        continue;
                    }
                    let phys_base = if bar & 0x4 != 0 {
                        if bar_index + 1 >= 6 {
                            continue;
                        }
                        (bar & 0xffff_fff0)
                            | ((header.bar(bar_index + 1) as u64 & 0xffff_ffff) << 32)
                    } else {
                        bar & 0xffff_fff0
                    };
                    let legacy_irq = header.interrupt_line() as u32;
                    if phys_base != 0 {
                        return Some((phys_base, legacy_irq, requester_id, None));
                    }
                }
            }
            push_bridge_buses(&bus.devices, &mut stack);
        }
    }
    None
}
