/// The software operating interface for a device or more properly, a device function. This is what
/// devices present to the kernel and what drivers use to interact with the device. Userspace does
/// not ever interact with this directly but can query it for debugging and informational purposes.
pub enum HwDeviceIfce {
    Unknown = 0,
    Unsupported = 1,
    // Generic
    Ns16550Uart,
    Ns16650Uart,
    Ns16750Uart,
    Ns16850Uart,
    Ns16950Uart,
    Ns16550MultiPortUart,
    Ns16650MultiPortUart,
    Ns16750MultiPortUart,
    Ns16850MultiPortUart,
    Ns16950MultiPortUart,
    I2CHostController,
    SpiHostController,
    AhciSataController,
    SdHostController,
    ScsiSasController,
    // PCI Express
    PcieHostBridge,
    PciToPciBridgeNormalDecode,
    PciToPciBridgeSubtractiveDecode,
    NvmExpressController,
    // USB
    Usb4Router,
    XhciUsbHostController,
    EhciUsbHostController,
    UsbHidClass,
    CdcAcmVirtualSerial,
    CdcNcmVirtualEthernet,
    //IPMI
    IpmiKcs,
    // Graphics and Display
    AmdGpu,
    IntelGpu,
    NvidiaGpu,
    UefiGopFramebuffer,
    UsbBulkDisplayClass,
    VirtioGpu,
    // x86-64 platform components
    #[cfg(target_arch = "x86_64")]
    I8042InputController,
    #[cfg(target_arch = "x86_64")]
    IoApic,
    #[cfg(target_arch = "x86_64")]
    IoXapic,
    #[cfg(target_arch = "x86_64")]
    SmBusController,
    #[cfg(target_arch = "x86_64")]
    IntelVtdIommu,
    #[cfg(target_arch = "x86_64")]
    AmdViIommu,
    #[cfg(target_arch = "x86_64")]
    HighPrecisionEventTimer,
    // Arm platform components
    #[cfg(target_arch = "aarch64")]
    ArmPl011Uart,
    #[cfg(target_arch = "aarch64")]
    ArmGic,
    #[cfg(target_arch = "aarch64")]
    ArmSmmu,
}

/// This trait is implemented to provide human readable names for device interfaces generally for
/// user queries and logging.
impl core::fmt::Display for HwDeviceIfce {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match self {
            HwDeviceIfce::Unknown => "Unrecognized Device Interface",
            HwDeviceIfce::Unsupported => "Unsupported Device Interface",
            HwDeviceIfce::Ns16550Uart => "National Semiconductor 16550A compatible UART",
            HwDeviceIfce::Ns16650Uart => "National Semiconductor 16650 compatible UART",
            HwDeviceIfce::Ns16750Uart => "National Semiconductor 16750 compatible UART",
            HwDeviceIfce::Ns16850Uart => "National Semiconductor 16850 compatible UART",
            HwDeviceIfce::Ns16950Uart => "National Semiconductor 16950 compatible UART",
            HwDeviceIfce::Ns16550MultiPortUart => {
                "National Semiconductor 16550A compatible Multi-Port UART"
            }
            HwDeviceIfce::Ns16650MultiPortUart => {
                "National Semiconductor 16650 compatible Multi-Port UART"
            }
            HwDeviceIfce::Ns16750MultiPortUart => {
                "National Semiconductor 16750 compatible Multi-Port UART"
            }
            HwDeviceIfce::Ns16850MultiPortUart => {
                "National Semiconductor 16850 compatible Multi-Port UART"
            }
            HwDeviceIfce::Ns16950MultiPortUart => {
                "National Semiconductor 16950 compatible Multi-Port UART"
            }
            HwDeviceIfce::I2CHostController => {
                "Inter-Integrated Circuit (I2C) Host Controller"
            }
            HwDeviceIfce::SpiHostController => {
                "Serial Peripheral Interface (SPI) Host Controller"
            }
            HwDeviceIfce::AhciSataController => "AHCI SATA Storage Controller",
            HwDeviceIfce::SdHostController => "SD Host Controller",
            HwDeviceIfce::ScsiSasController => "Serial Attached SCSI Controller",
            HwDeviceIfce::PcieHostBridge => "PCI Express Host Bridge",
            HwDeviceIfce::PciToPciBridgeNormalDecode => {
                "PCI (Express) to PCI (Express) Bridge (Normal Decode)"
            }
            HwDeviceIfce::PciToPciBridgeSubtractiveDecode => {
                "PCI (Express) to PCI (Express) Bridge (Subtractive Decode)"
            }
            HwDeviceIfce::NvmExpressController => {
                "Non-Volatile Memory Express (NVMe) Storage Controller"
            }
            HwDeviceIfce::Usb4Router => "USB4 Router",
            HwDeviceIfce::XhciUsbHostController => {
                "eXtensible Host Controller Interface (xHCI) compatible USB Host Controller"
            }
            HwDeviceIfce::EhciUsbHostController => {
                "Enhanced Host Controller Interface (EHCI) compatible USB Host Controller"
            }
            HwDeviceIfce::UsbHidClass => "USB Human Interface Device (HID) Class Device",
            HwDeviceIfce::CdcAcmVirtualSerial => {
                "USB Communications Device Class (CDC) Abstract Control Model (ACM) Serial Device"
            }
            HwDeviceIfce::CdcNcmVirtualEthernet => {
                "USB Communications Device Class (CDC) Network Control Model (NCM) Ethernet Device"
            }
            HwDeviceIfce::IpmiKcs => {
                "Intelligent Platform Management Interface (IPMI) Keyboard Controller Style (KCS) \
                 Interface"
            }
            HwDeviceIfce::AmdGpu => {
                "Advanced Micro Devices (AMD) VGA Compatible Device, Model Unknown"
            }
            HwDeviceIfce::IntelGpu => "Intel Corporation VGA Compatible Device, Model Unknown",
            HwDeviceIfce::NvidiaGpu => {
                "Nvidia Corporation VGA Compatible Device, Model Unknown"
            }
            HwDeviceIfce::UefiGopFramebuffer => {
                "UEFI Graphics Output Protocol (GOP) Framebuffer"
            }
            HwDeviceIfce::UsbBulkDisplayClass => "USB Bulk Display Class Device",
            HwDeviceIfce::VirtioGpu => "Virtio Virtual Graphics Processing Unit (GPU)",
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::I8042InputController => "i8042 (PS/2) compatible Input Controller",
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::IoApic => {
                "Input/Output Advanced Programmable Interrupt Controller (I/O APIC)"
            }
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::IoXapic => {
                "Extended Input/Output Advanced Programmable Interrupt Controller (IOxAPIC)"
            }
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::SmBusController => "System Management Bus (SMBus) Controller",
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::IntelVtdIommu => {
                "Intel Virtualization Technology for Directed I/O (VT-d) Input/Output Memory \
                 Management Unit (IOMMU)"
            }
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::AmdViIommu => {
                "Advanced Micro Devices Virtualization (AMD-V) Input/Output Memory Management Unit \
                 (IOMMU)"
            }
            #[cfg(target_arch = "x86_64")]
            HwDeviceIfce::HighPrecisionEventTimer => "High Precision Event Timer",
            #[cfg(target_arch = "aarch64")]
            HwDeviceIfce::ArmPl011Uart => "ARM PL011 UART",
            #[cfg(target_arch = "aarch64")]
            HwDeviceIfce::ArmGic => "ARM Generic Interrupt Controller",
            #[cfg(target_arch = "aarch64")]
            HwDeviceIfce::ArmSmmu => "ARM System Memory Management Unit",
        };
        write!(f, "{}", name)
    }
}
