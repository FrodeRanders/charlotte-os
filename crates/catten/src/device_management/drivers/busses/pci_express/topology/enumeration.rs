//! Fixed boot worklist: every in-range bus identity is accepted at most once.
//!
//! Range/revisit rules adapted from upstream CharlotteOS 72cc1417b8ddf09013caab1d80dfc389ccfbdf18.
pub(crate) const MAX_FUNCTIONS: usize = 16_384;
pub(crate) const MAX_REGIONS: usize = 32;
pub(crate) const ECAM_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    FunctionCapacity,
    DeviceCapacity,
    StorageUnavailable,
}

#[cfg(test)]
#[path = "enumeration/tests.rs"]
mod tests;

pub(crate) struct Budget {
    remaining: usize,
    devices: usize,
}

impl Budget {
    pub const fn new() -> Self {
        Self {
            remaining: MAX_FUNCTIONS,
            devices: MAX_FUNCTIONS,
        }
    }

    pub fn accept_device(&mut self) -> Result<(), Error> {
        self.devices = self.devices.checked_sub(1).ok_or(Error::DeviceCapacity)?;
        Ok(())
    }

    pub fn accept_function(&mut self) -> Result<(), Error> {
        self.remaining = self.remaining.checked_sub(1).ok_or(Error::FunctionCapacity)?;
        Ok(())
    }
}

pub(crate) struct Buses {
    start: u8,
    end: u8,
    seen: [bool; 256],
    pending: [u8; 256],
    tail: usize,
}

impl Buses {
    pub fn new(start: u8, end: u8) -> Self {
        assert!(start <= end, "validated MCFG bus range");
        let mut buses = Self {
            start,
            end,
            seen: [false; 256],
            pending: [0; 256],
            tail: 0,
        };
        buses.discover(start);
        buses
    }

    pub fn discover(&mut self, bus: u8) {
        if bus < self.start || bus > self.end || self.seen[bus as usize] {
            return;
        }
        self.seen[bus as usize] = true;
        // There are only 256 distinct u8 bus identities; mark at discovery,
        // including empty buses, before appending. Duplicates never use space.
        self.pending[self.tail] = bus;
        self.tail += 1;
    }

    pub fn next(&mut self) -> Option<u8> {
        if self.tail == 0 {
            return None;
        }
        // Match the existing lookup order: last discovered sibling first,
        // depth first, without recursively constructing or searching buses.
        self.tail -= 1;
        Some(self.pending[self.tail])
    }
}
