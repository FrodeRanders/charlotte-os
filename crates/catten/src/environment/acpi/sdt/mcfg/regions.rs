//! Allocation-free preparation of complete MCFG regions before ECAM mapping.
//!
//! Validation adapted from upstream CharlotteOS 74def27ce686a07aaec621f15ef786b546c54f20.
pub(crate) use crate::device_management::drivers::busses::pci_express::topology::enumeration::MAX_REGIONS;
const HEADER_BYTES: usize = 44;
const ENTRY_BYTES: usize = 16;
pub(super) const MAX_TABLE_BYTES: usize = 64 * 1024;
const ECAM_BYTES: u64 =
    crate::device_management::drivers::busses::pci_express::topology::enumeration::ECAM_BYTES
        as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub base: u64,
    pub segment: u16,
    pub start: u8,
    pub end: u8,
}

impl Region {
    fn span(self) -> (u64, u64) {
        (self.base + (u64::from(self.start) << 20), self.base + ((u64::from(self.end) + 1) << 20))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    TableGeometry,
    Checksum,
    Reserved,
    Capacity,
    BusRange,
    PhysicalRange,
    Overlap,
}

#[derive(Clone, Copy)]
pub(crate) struct Regions {
    entries: [Option<Region>; MAX_REGIONS],
    len: usize,
}

impl Regions {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_REGIONS],
            len: 0,
        }
    }

    /// Reject the entire table without changing the accepted prefix on error.
    pub fn append_table(&mut self, table: &[u8], physical_mask: u64) -> Result<(), Error> {
        if table.len() < HEADER_BYTES
            || table.len() > MAX_TABLE_BYTES
            || table.get(..4) != Some(b"MCFG")
            || u32::from_le_bytes(table[4..8].try_into().unwrap()) as usize != table.len()
            || !(table.len() - HEADER_BYTES).is_multiple_of(ENTRY_BYTES)
        {
            return Err(Error::TableGeometry);
        }
        if table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
            return Err(Error::Checksum);
        }
        if table[36..HEADER_BYTES].iter().any(|&byte| byte != 0) {
            return Err(Error::Reserved);
        }
        let count = (table.len() - HEADER_BYTES) / ENTRY_BYTES;
        if count > MAX_REGIONS - self.len {
            return Err(Error::Capacity);
        }
        let mut candidate = *self;
        for bytes in table[HEADER_BYTES..].as_chunks::<ENTRY_BYTES>().0 {
            let region = Region {
                base: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                segment: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
                start: bytes[10],
                end: bytes[11],
            };
            if bytes[12..].iter().any(|&byte| byte != 0) {
                return Err(Error::Reserved);
            }
            if region.start > region.end {
                return Err(Error::BusRange);
            }
            // This fork retains a complete 256 MiB boot ECAM mapping, even
            // when firmware advertises only part of its bus-zero-relative window.
            let last = region.base.checked_add(ECAM_BYTES - 1).ok_or(Error::PhysicalRange)?;
            if !region.base.is_multiple_of(1 << 20)
                || last == u64::MAX
                || (region.base | last) & !physical_mask != 0
            {
                return Err(Error::PhysicalRange);
            }
            let (start, end) = region.span();
            for old in candidate.iter() {
                let (old_start, old_end) = old.span();
                if (old.segment == region.segment
                    && old.start <= region.end
                    && region.start <= old.end)
                    || (start < old_end && old_start < end)
                {
                    return Err(Error::Overlap);
                }
            }
            candidate.entries[candidate.len] = Some(region);
            candidate.len += 1;
        }
        *self = candidate;
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Region> {
        self.entries[..self.len].iter().map(|entry| entry.as_ref().unwrap())
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
#[path = "regions/tests.rs"]
mod tests;
