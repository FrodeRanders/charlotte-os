//! Bounded request preparation using the existing memory and mapping owners.

use super::{
    MemoryError,
    OwnedMemory,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadError {
    Capacity,
    Allocate(MemoryError),
    Map(MemoryError),
    Encode,
    InvalidLength,
    Unmap(MemoryError),
}

/// An unmapped memory owner paired with its exact initialized byte length.
/// An empty payload still owns a memory object; it is not an absent attachment.
#[must_use = "dropping a payload releases its memory capability"]
#[derive(Debug)]
pub struct OwnedPayload {
    memory: OwnedMemory,
    len: usize,
}

impl OwnedPayload {
    /// Prepare at most `capacity` bytes, subject to the caller's explicit limit.
    /// The encoder sees only that capacity, even when backing rounds to pages.
    /// Preparation failures dispose the private owners through their existing
    /// policies; they grant no permission to reuse an uncertain mapping.
    pub fn encode(
        capacity: usize,
        limit: usize,
        encode: impl FnOnce(&mut [u8]) -> Option<usize>,
    ) -> Result<Self, PayloadError> {
        const PAGE_BYTES: usize = 4096;
        if capacity > limit {
            return Err(PayloadError::Capacity);
        }
        let pages =
            capacity.max(1).checked_add(PAGE_BYTES - 1).ok_or(PayloadError::Capacity)? / PAGE_BYTES;
        let memory = OwnedMemory::allocate(pages).map_err(PayloadError::Allocate)?;
        let mut mapping = memory.map_writable().map_err(|(_, error)| PayloadError::Map(error))?;
        let output = mapping.as_mut_slice().get_mut(..capacity).ok_or(PayloadError::Capacity)?;
        let len = encode(output).ok_or(PayloadError::Encode)?;
        if len > capacity {
            return Err(PayloadError::InvalidLength);
        }
        let memory = mapping.unmap().map_err(|(_, error)| PayloadError::Unmap(error))?;
        Ok(Self {
            memory,
            len,
        })
    }

    pub fn copy_from(bytes: &[u8], limit: usize) -> Result<Self, PayloadError> {
        Self::encode(bytes.len(), limit, |output| {
            output.copy_from_slice(bytes);
            Some(bytes.len())
        })
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn memory(&self) -> &OwnedMemory {
        &self.memory
    }

    pub fn into_parts(self) -> (OwnedMemory, usize) {
        (self.memory, self.len)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec;

    use super::*;
    use crate::owned::kernel::{
        self,
        Event,
    };

    #[test]
    fn encoded_payload_retains_memory_and_unmaps_before_transfer() {
        let _guard = kernel::serial();
        kernel::reset();
        let mut backing = vec![0u8; 4096];
        kernel::update(|s| s.memory_map_any.1 = backing.as_mut_ptr() as usize);
        let payload = OwnedPayload::copy_from(b"hello", 8).unwrap();
        assert_eq!(payload.len(), 5);
        assert_eq!(&backing[..5], b"hello");
        assert_eq!(kernel::events(), [Event::MemoryUnmap(10)]);
        let (memory, len) = payload.into_parts();
        assert_eq!(len, 5);
        drop(memory);
        assert_eq!(kernel::events(), [Event::MemoryUnmap(10), Event::MemoryClose(10)]);
    }

    #[test]
    fn invalid_capacity_allocation_mapping_and_encoding_release_private_owners() {
        let _guard = kernel::serial();
        for case in 0..6 {
            kernel::reset();
            let mut backing = vec![0u8; 4096];
            kernel::update(|s| {
                s.memory_map_any.1 = backing.as_mut_ptr() as usize;
                if case == 1 {
                    s.memory_alloc = 0;
                }
                if case == 2 {
                    s.memory_map_any.0 = 99;
                }
                if case == 5 {
                    s.memory_unmap.push_back(99);
                }
            });
            let result = OwnedPayload::encode(
                if case == 0 {
                    usize::MAX
                } else {
                    4
                },
                4,
                |bytes| {
                    assert_eq!(bytes.len(), 4);
                    if case == 3 {
                        None
                    } else if case == 4 {
                        Some(5)
                    } else {
                        Some(0)
                    }
                },
            );
            assert!(result.is_err());
            let events = kernel::events();
            if case < 2 {
                assert!(events.is_empty());
            } else {
                assert_eq!(events.last(), Some(&Event::MemoryClose(10)));
            }
        }
    }

    #[test]
    fn empty_payload_is_an_owned_attachment() {
        let _guard = kernel::serial();
        kernel::reset();
        let mut backing = vec![0u8; 4096];
        kernel::update(|s| s.memory_map_any.1 = backing.as_mut_ptr() as usize);
        let payload = OwnedPayload::copy_from(b"", 0).unwrap();
        assert!(payload.is_empty());
        drop(payload);
        assert_eq!(kernel::events(), [Event::MemoryUnmap(10), Event::MemoryClose(10)]);
    }
}
