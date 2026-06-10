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

    pub const UNCLASSIFIED: PciClassCode = 0x00;
    pub const MASS_STORAGE: PciClassCode = 0x01;
    pub const NETWORK: PciClassCode = 0x02;
    pub const DISPLAY: PciClassCode = 0x03;
    pub const MULTIMEDIA: PciClassCode = 0x04;
    pub const MEMORY: PciClassCode = 0x05;
    pub const BRIDGE: PciClassCode = 0x06;
    pub const SIMPLE_COMMUNICATIONS: PciClassCode = 0x07;
    pub const BASE_PERIPHERALS: PciClassCode = 0x08;
    pub const INPUT: PciClassCode = 0x09;
    pub const DOCKING_STATION: PciClassCode = 0x0a;
    pub const PROCESSOR: PciClassCode = 0x0b;
    pub const SERIAL_BUS: PciClassCode = 0x0c;
    pub const WIRELESS: PciClassCode = 0x0d;
    pub const INTELLIGENT_IO: PciClassCode = 0x0e;
    pub const SATELLITE_COMMUNICATIONS: PciClassCode = 0x0f;
    pub const ENCRYPTION: PciClassCode = 0x10;
    pub const DATA_ACQUISITION: PciClassCode = 0x11;
    pub const UNDEFINED: PciClassCode = 0xff; // This is not a valid class code, but is used to represent an undefined class code.

    pub type PciSubclassCode = u8;

    pub const SATA: PciSubclassCode = 0x06;
    pub const 
}
