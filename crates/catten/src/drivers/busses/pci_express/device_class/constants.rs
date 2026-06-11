pub mod vendor_id {
    pub type PciVendorId = u16;

    pub const VENDOR_ID_UNKNOWN: PciVendorId = 0xffff;
    pub const VENDOR_ID_INVALID: PciVendorId = 0x0000;

    pub const INTEL: PciVendorId = 0x8086;
    pub const AMD: PciVendorId = 0x1022;
    pub const ARM: PciVendorId = 0x13b5;
    pub const QEMU: PciVendorId = 0x1234;
}

pub mod device_class {
    pub type PciClassCode = u8;
    pub type PciSubclassCode = u8;
    pub type PciProgIf = u8;
    pub type PciClassFull = (PciClassCode, PciSubclassCode, PciProgIf);

    /* Display Controllers */
    pub const VGA_COMPATIBLE: PciClassFull = (0x03, 0x00, 0x00);

    /* Bridges */
    pub const HOST_BRIDGE: PciClassFull = (0x06, 0x00, 0x00);
    pub const PCI_TO_PCI_BRIDGE: PciClassFull = (0x06, 0x04, 0x00);
    pub const PCI_TO_PCI_BRIDE_SUB_DEC: PciClassFull = (0x06, 0x04, 0x01);

    /* NS16x50 UARTs */
    pub const NS16550: PciClassFull = (0x07, 0x00, 0x02);
    pub const NS16650: PciClassFull = (0x07, 0x00, 0x03);
    pub const NS16750: PciClassFull = (0x07, 0x00, 0x04);
    pub const NS16850: PciClassFull = (0x07, 0x00, 0x05);
    pub const NS16950: PciClassFull = (0x07, 0x00, 0x06);
    pub const NS16550_MULTI_PORT: PciClassFull = (0x07, 0x02, 0x02);
    pub const NS16650_MULTI_PORT: PciClassFull = (0x07, 0x02, 0x03);
    pub const NS16750_MULTI_PORT: PciClassFull = (0x07, 0x02, 0x04);
    pub const NS16850_MULTI_PORT: PciClassFull = (0x07, 0x02, 0x05);
    pub const NS16950_MULTI_PORT: PciClassFull = (0x07, 0x02, 0x06);

    /* Base System Peripherals */

    /* USB Host Controllers */
    pub const USB_EHCI: PciClassFull = (0x0c, 0x03, 0x20);
    pub const USB_XHCI: PciClassFull = (0x0c, 0x03, 0x30);
    pub const USB4_ROUTER: PciClassFull = (0x0c, 0x03, 0x40);
}
