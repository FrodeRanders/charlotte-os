use hashbrown::HashMap;

use super::DeviceId;
use crate::cpu::isa::io::IoReg8;

pub struct IoRange {
    pub start: IoReg8,
    pub len: usize,
}

pub struct IoMap {
    entries: HashMap<DeviceId, IoRange>,
}

impl IoMap {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, device_id: DeviceId, range: IoRange) {
        self.entries.insert(device_id, range);
    }

    pub fn get(&self, device_id: &DeviceId) -> Option<&IoRange> {
        self.entries.get(device_id)
    }
}
