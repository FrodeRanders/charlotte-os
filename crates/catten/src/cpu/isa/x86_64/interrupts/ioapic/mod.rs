struct IoApicOverlay {
    ioregsel: u32,
    ioregwin: u32,
}

impl IoApicOverlay {
    pub fn read32(&self, index: u32) -> u32 {
        unsafe {
            core::ptr::write_volatile(&self.ioregsel, index);
            core::ptr::read_volatile(&self.ioregwin)
        }
    }

    pub fn write32(&mut self, index: u32, value: u32) {
        unsafe {
            core::ptr::write_volatile(&mut self.ioregsel, index);
            core::ptr::write_volatile(&mut self.ioregwin, value);
        }
    }

    pub fn read64(&self, index: u32) -> u64 {
        let low = self.read32(index) as u64;
        let high = self.read32(index + 1) as u64;
        (high << 32) | low
    }

    pub fn write64(&mut self, index: u32, value: u64) {
        self.write32(index, value as u32);
        self.write32(index + 1, (value >> 32) as u32);
    }
}

#[repr(transparent)]
pub struct IoApic(*mut IoApicOverlay);

impl IoApic {
    fn get_id(&self) -> u32 {
        unsafe { (*self.0).read32(0) >> 24 }
    }
}
