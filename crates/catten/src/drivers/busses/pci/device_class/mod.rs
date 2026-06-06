use hashbrown::HashMap;

static DEVICE_CLASS_MAP: spin::LazyLock<HashMap<PciFunctionCode, DeviceClass>> =
    spin::LazyLock::new(|| {
        let mut dcm = HashMap::new();
        // Initialize the map with known PCI function codes and their corresponding device classes
        dcm.insert(
            PciFunctionCode {
                class_code: 0x06, // Bridge Device class code
                subclass: 0x04,   // PCI-to-PCI bridge subclass code
                prog_if: 0x00,    // Normal decode programming interface code
            },
            DeviceClass::PciToPciBridgeNormalDecode,
        );
        dcm.insert(
            PciFunctionCode {
                class_code: 0x06, // Bridge Device class code
                subclass: 0x04,   // PCI-to-PCI bridge subclass code
                prog_if: 0x01,    // Subtractive decode programming interface code
            },
            DeviceClass::PciToPciBridgeSubtractiveDecode,
        );

        dcm
    });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceClass {
    Unknown,
    HostBridge,
    PciToPciBridgeNormalDecode,
    PciToPciBridgeSubtractiveDecode,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PciFunctionCode {
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
}
