//! Primitive syscall inputs share allocation mechanics, not launch authority.
use alloc::vec::Vec;

use crate::{
    klib::sensitive_buffer::SensitiveBuffer,
    memory::{
        self,
        AddressSpaceHandle,
        object::{
            self,
            MemoryObjectCap,
            MemoryObjectError,
        },
    },
};

type Allocator = memory::kernel_buffers::SnapshotAllocator;
type Bytes = Vec<u8, Allocator>;

/// Borrow the executing caller's exact authorized generation. The syscall
/// entry/current thread retains that root; this class token grants no root or
/// teardown ownership. Capture trusted state once, before any buffer allocation.
pub(super) struct Class {
    origin: AddressSpaceHandle,
    allocator: Allocator,
}

pub(super) struct ScopedInputs {
    pub(super) image: Bytes,
    pub(super) descriptor: Bytes,
}
pub(super) struct OperationalInputs {
    pub(super) package: Bytes,
    pub(super) plaintext: SensitiveBuffer<Allocator>,
}

impl Class {
    pub(super) fn capture(origin: AddressSpaceHandle) -> Result<Self, MemoryObjectError> {
        if !memory::budget::accepting(origin) {
            return Err(MemoryObjectError::AddressSpaceClosing);
        }
        let storage = memory::kernel_buffers::capture_storage(origin)
            .map_err(|_| MemoryObjectError::AddressSpaceMissing)?;
        Ok(Self {
            origin,
            allocator: storage.allocator(),
        })
    }

    pub(super) fn snapshot(
        &self,
        cap: MemoryObjectCap,
        len: usize,
    ) -> Result<Bytes, MemoryObjectError> {
        if !memory::address_space_handle_is_current(self.origin) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        object::snapshot_bytes_in(self.origin.id(), cap, len, self.allocator.clone())
    }

    pub(super) fn scoped(
        &self,
        image: (MemoryObjectCap, usize),
        descriptor: (MemoryObjectCap, usize),
    ) -> Result<ScopedInputs, MemoryObjectError> {
        Self::scoped_with(image, descriptor, |(cap, len)| self.snapshot(cap, len))
    }

    fn scoped_with(
        image: (MemoryObjectCap, usize),
        descriptor: (MemoryObjectCap, usize),
        mut snapshot: impl FnMut((MemoryObjectCap, usize)) -> Result<Bytes, MemoryObjectError>,
    ) -> Result<ScopedInputs, MemoryObjectError> {
        let image = snapshot(image)?;
        let descriptor = snapshot(descriptor)?;
        Ok(ScopedInputs {
            image,
            descriptor,
        })
    }

    pub(super) fn operational(
        &self,
        package: MemoryObjectCap,
        len: usize,
    ) -> Result<OperationalInputs, MemoryObjectError> {
        Ok(OperationalInputs {
            package: self.snapshot(package, len)?,
            plaintext: SensitiveBuffer::new_in(self.allocator.clone()),
        })
    }
}

#[cfg(feature = "kernel_tests")]
pub(crate) mod tests;
