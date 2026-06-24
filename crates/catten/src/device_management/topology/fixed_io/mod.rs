//! # Tracking structures for fixed I/O devices
//!
//! This module defines the `IoMap` struct, which manages the mapping of fixed I/O devices to their
//! corresponding I/O ranges. Enumeration of fixed I/O devices is performed using either the ACPI
//! static description tables or a Flattened Device Tree (FDT) at boot time, and the resulting
//! mapping is stored in an `IoMap` instance for later use by the device manager and device drivers.
//! On PC like platforms there are generally only a few fixed I/O devices, most of which are legacy
//! devices that are not used, however there is the occasional anomalous machine that has fixed I/O
//! devices that need to be used for crucial functionality so this module is designed to be flexible
//! enough to support a wide range of fixed I/O devices and configurations. Additionally, embedded
//! and SoC style machines often have a large number of fixed I/O devices which must also be
//! properly accommodated.
//!
//! Note however that this is intended for strictly fixed I/O devices that are not on any enumerable
//! bus or accessed through another similar mechanism such as the ACPI namespace which behaves less
//! like a static description and more like a firmware based bus. As such namespace devices should
//! appear in the device topology under the ACPI namespace pseudo-bus and not be tracked here. This
//! is intended for devices that are accessed through fixed I/O ports or memory addresses and are
//! not enumerable or accessible by other dynamic means.

use hashbrown::HashMap;

pub use crate::cpu::isa::io::IoRegion;
use crate::device_management::DeviceId;

pub struct IoMap {
    entries: HashMap<DeviceId, IoRegion>,
}

impl IoMap {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, device_id: DeviceId, range: IoRegion) {
        self.entries.insert(device_id, range);
    }

    pub fn get(&self, device_id: &DeviceId) -> Option<&IoRegion> {
        self.entries.get(device_id)
    }
}
