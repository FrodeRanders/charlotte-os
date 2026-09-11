//! Owned memory, mappings, and DMA transfers.
//!
//! Child module of [`crate::owned`].

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    AllocationFailed,
    InvalidCapability,
    MemoryStatus(u64),
    DmaMapFailed,
    DmaStatus(u64),
}

/// An owned memory-object capability.
///
/// This type is deliberately neither `Copy` nor `Clone`. Use [`into_raw`](Self::into_raw)
/// only when transferring the capability through an API that consumes it.
#[must_use = "dropping an owned memory object closes its capability"]
#[derive(Debug)]
pub struct OwnedMemory {
    pub(super) cap: Option<u64>,
    len: usize,
}

/// A launch-environment-owned, read-only memory capability borrowed through
/// [`crate::Context`]. Dropping this view does not close the capability; the
/// kernel reclaims it with the domain.
#[derive(Debug)]
pub struct LaunchMemoryRef<'context> {
    cap: u64,
    len: usize,
    _context: PhantomData<&'context crate::Context>,
}

impl<'context> LaunchMemoryRef<'context> {
    /// Construct at the typed launch-configuration ABI boundary.
    pub(crate) unsafe fn from_raw(cap: u64, len: usize) -> Result<Self, MemoryError> {
        let capacity = kernel::memory_size(cap);
        if cap == 0 || len == 0 || len > capacity {
            return Err(MemoryError::InvalidCapability);
        }
        Ok(Self {
            cap,
            len,
            _context: PhantomData,
        })
    }

    pub fn map_read_only(&self) -> Result<MappedLaunchMemory<'_>, MemoryError> {
        let (status, base) = kernel::memory_map_any(self.cap, false);
        if status != catten_syscall::memory_status::OK {
            return Err(MemoryError::MemoryStatus(status));
        }
        Ok(MappedLaunchMemory {
            cap: self.cap,
            base,
            len: self.len,
            _borrow: PhantomData,
        })
    }
}

/// A temporary mapping of launch-owned immutable data.
#[must_use = "dropping the launch mapping unmaps it"]
#[derive(Debug)]
pub struct MappedLaunchMemory<'memory> {
    cap: u64,
    base: usize,
    len: usize,
    _borrow: PhantomData<&'memory LaunchMemoryRef<'memory>>,
}

impl MappedLaunchMemory<'_> {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.base as *const u8, self.len) }
    }
}

impl Drop for MappedLaunchMemory<'_> {
    fn drop(&mut self) {
        let _ = kernel::memory_unmap(self.cap);
    }
}

impl OwnedMemory {
    pub fn allocate(pages: usize) -> Result<Self, MemoryError> {
        let cap = kernel::memory_alloc(pages);
        if cap == 0 {
            return Err(MemoryError::AllocationFailed);
        }
        let len = kernel::memory_size(cap);
        if len == 0 {
            let _ = kernel::memory_close(cap);
            return Err(MemoryError::AllocationFailed);
        }
        Ok(Self {
            cap: Some(cap),
            len,
        })
    }

    /// Adopt a memory capability received from a trusted typed IPC boundary.
    ///
    /// # Safety
    /// `cap` must be an unmapped, unlent, non-DMA, owned memory-object
    /// capability. No other live value or raw syscall may use the handle after
    /// it is adopted, and it must not also be adopted by another
    /// [`OwnedMemory`].
    pub unsafe fn from_raw(cap: u64) -> Result<Self, MemoryError> {
        let len = kernel::memory_size(cap);
        if len == 0 {
            return Err(MemoryError::InvalidCapability);
        }
        Ok(Self {
            cap: Some(cap),
            len,
        })
    }

    pub(super) fn from_kernel(cap: u64) -> Result<Self, MemoryError> {
        let len = kernel::memory_size(cap);
        if len == 0 {
            let _ = kernel::memory_close(cap);
            return Err(MemoryError::InvalidCapability);
        }
        Ok(Self {
            cap: Some(cap),
            len,
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(super) fn raw_handle(&self) -> u64 {
        self.cap.expect("owned memory capability already consumed")
    }

    /// Relinquish Rust ownership, normally for an IPC move operation.
    pub fn into_raw(mut self) -> u64 {
        self.cap.take().expect("owned memory capability already consumed")
    }

    pub fn map_read_only(self) -> Result<MappedMemory<ReadOnly>, (Self, MemoryError)> {
        self.map(false)
    }

    pub fn map_writable(self) -> Result<MappedMemory<Writable>, (Self, MemoryError)> {
        self.map(true)
    }

    /// Transfer unmapped memory from CPU ownership to a DMA domain.
    ///
    /// A caller must explicitly consume any [`MappedMemory`] with
    /// [`MappedMemory::unmap`] before this method is available. Consequently,
    /// safe Rust references into the object cannot survive into the transfer.
    pub fn begin_dma(
        self,
        domain: &DmaDomain,
        direction: DmaDirection,
    ) -> Result<DmaTransfer<'_>, (Self, MemoryError)> {
        let iova = kernel::dma_map_exclusive(domain.raw_handle(), self.raw_handle(), direction);
        if iova == 0 {
            return Err((self, MemoryError::DmaMapFailed));
        }
        Ok(DmaTransfer {
            domain,
            memory: Some(self),
            iova,
        })
    }

    /// Map coherent memory for simultaneous CPU and device access.
    ///
    /// This is intended for hardware rings and buffers whose protocol defines
    /// ownership through volatile fields and explicit memory fences. It does
    /// not expose Rust references: callers must use the volatile accessors on
    /// [`SharedDmaMemory`].
    pub fn map_shared_dma(
        self,
        domain: &DmaDomain,
        direction: DmaDirection,
    ) -> Result<SharedDmaMemory<'_>, SharedDmaMapError> {
        let mut mapping =
            self.map_writable().map_err(|(memory, error)| SharedDmaMapError::Unmapped {
                memory,
                error,
            })?;
        let memory = mapping.memory.as_ref().expect("mapped memory already consumed");
        let iova = kernel::dma_map(domain.raw_handle(), memory.raw_handle(), direction);
        if iova == 0 {
            return Err(SharedDmaMapError::Mapped {
                mapping,
                error: MemoryError::DmaMapFailed,
            });
        }
        let base = mapping.base;
        let memory = mapping.memory.take().expect("mapped memory already consumed");
        Ok(SharedDmaMemory {
            domain,
            memory: Some(memory),
            base,
            iova,
            dma_active: true,
        })
    }

    fn map<Access>(self, writable: bool) -> Result<MappedMemory<Access>, (Self, MemoryError)> {
        let (status, base) = kernel::memory_map_any(self.raw_handle(), writable);
        if status != catten_syscall::memory_status::OK {
            return Err((self, MemoryError::MemoryStatus(status)));
        }
        Ok(MappedMemory {
            memory: Some(self),
            base,
            _access: PhantomData,
        })
    }
}

/// A shared-DMA setup failure that preserves the memory's exact ownership
/// state, including a still-live CPU mapping when DMA admission failed.
#[derive(Debug)]
pub enum SharedDmaMapError {
    Unmapped {
        memory: OwnedMemory,
        error: MemoryError,
    },
    Mapped {
        mapping: MappedMemory<Writable>,
        error: MemoryError,
    },
}

impl SharedDmaMapError {
    pub const fn error(&self) -> MemoryError {
        match self {
            Self::Unmapped {
                error,
                ..
            }
            | Self::Mapped {
                error,
                ..
            } => *error,
        }
    }
}

impl Drop for OwnedMemory {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::memory_close(cap);
        }
    }
}

#[derive(Debug)]
pub enum ReadOnly {}
#[derive(Debug)]
pub enum Writable {}

/// A CPU mapping which owns the underlying memory capability.
#[must_use = "dropping mapped memory unmaps and closes it"]
#[derive(Debug)]
pub struct MappedMemory<Access> {
    memory: Option<OwnedMemory>,
    base: usize,
    _access: PhantomData<Access>,
}

impl<Access> MappedMemory<Access> {
    pub fn len(&self) -> usize {
        self.memory.as_ref().expect("mapped memory already consumed").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.base as *const u8
    }

    pub fn unmap(mut self) -> Result<OwnedMemory, (Self, MemoryError)> {
        let cap = self.memory.as_ref().expect("mapped memory already consumed").raw_handle();
        let status = kernel::memory_unmap(cap);
        if status != catten_syscall::memory_status::OK {
            return Err((self, MemoryError::MemoryStatus(status)));
        }
        Ok(self.memory.take().expect("mapped memory already consumed"))
    }
}

impl MappedMemory<ReadOnly> {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.base as *const u8, self.len()) }
    }
}

impl MappedMemory<Writable> {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.base as *const u8, self.len()) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.base as *mut u8, self.len()) }
    }
}

impl<Access> Drop for MappedMemory<Access> {
    fn drop(&mut self) {
        if let Some(memory) = self.memory.take() {
            let _ = kernel::memory_unmap(memory.raw_handle());
            drop(memory);
        }
    }
}

/// A borrowed DMA-domain capability. The grant remains owned by the launch
/// environment; this wrapper only prevents mixing it with memory handles.
#[derive(Debug)]
pub struct DmaDomain(u64);

impl DmaDomain {
    /// # Safety
    /// `cap` must name a DMA domain granted to the current address space and
    /// must remain valid for every transfer created from this wrapper.
    pub const unsafe fn from_raw(cap: u64) -> Self {
        Self(cap)
    }

    pub(super) const fn raw_handle(&self) -> u64 {
        self.0
    }
}

/// Memory exclusively owned by a device until [`finish`](Self::finish).
#[must_use = "a DMA transfer must be finished before CPU access resumes"]
#[derive(Debug)]
pub struct DmaTransfer<'domain> {
    domain: &'domain DmaDomain,
    memory: Option<OwnedMemory>,
    iova: u64,
}

impl DmaTransfer<'_> {
    pub fn iova(&self) -> u64 {
        self.iova
    }

    pub fn finish(mut self) -> Result<OwnedMemory, (Self, MemoryError)> {
        let status = kernel::dma_unmap(self.domain.raw_handle(), self.iova);
        if status != catten_syscall::device_status::OK {
            return Err((self, MemoryError::DmaStatus(status)));
        }
        Ok(self.memory.take().expect("DMA memory already consumed"))
    }
}

impl Drop for DmaTransfer<'_> {
    fn drop(&mut self) {
        if self.memory.is_some() {
            let _ = kernel::dma_unmap(self.domain.raw_handle(), self.iova);
        }
    }
}

/// Coherent memory shared by a device and the CPU under a device protocol.
///
/// Unlike [`MappedMemory`], this type deliberately exposes no slices because
/// a device may mutate the bytes asynchronously. Access is volatile and must
/// be ordered with the fences required by the relevant hardware protocol.
#[must_use = "shared DMA memory must remain owned while the device can access it"]
#[derive(Debug)]
pub struct SharedDmaMemory<'domain> {
    domain: &'domain DmaDomain,
    memory: Option<OwnedMemory>,
    base: usize,
    iova: u64,
    dma_active: bool,
}

impl SharedDmaMemory<'_> {
    pub fn len(&self) -> usize {
        self.memory.as_ref().expect("shared DMA memory already consumed").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iova(&self) -> u64 {
        self.iova
    }

    pub fn read_volatile(&self, offset: usize) -> Option<u8> {
        (offset < self.len())
            .then(|| unsafe { core::ptr::read_volatile((self.base as *const u8).add(offset)) })
    }

    pub fn write_volatile(&mut self, offset: usize, value: u8) -> Result<(), MemoryError> {
        if offset >= self.len() {
            return Err(MemoryError::InvalidCapability);
        }
        unsafe { core::ptr::write_volatile((self.base as *mut u8).add(offset), value) };
        Ok(())
    }

    pub fn read_volatile_into(&self, offset: usize, output: &mut [u8]) -> Result<(), MemoryError> {
        let end = offset.checked_add(output.len()).ok_or(MemoryError::InvalidCapability)?;
        if end > self.len() {
            return Err(MemoryError::InvalidCapability);
        }
        for (index, byte) in output.iter_mut().enumerate() {
            *byte =
                unsafe { core::ptr::read_volatile((self.base as *const u8).add(offset + index)) };
        }
        Ok(())
    }

    pub fn write_volatile_from(&mut self, offset: usize, input: &[u8]) -> Result<(), MemoryError> {
        let end = offset.checked_add(input.len()).ok_or(MemoryError::InvalidCapability)?;
        if end > self.len() {
            return Err(MemoryError::InvalidCapability);
        }
        for (index, byte) in input.iter().copied().enumerate() {
            unsafe { core::ptr::write_volatile((self.base as *mut u8).add(offset + index), byte) };
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<OwnedMemory, (Self, MemoryError)> {
        if self.dma_active {
            let status = kernel::dma_unmap(self.domain.raw_handle(), self.iova);
            if status != catten_syscall::device_status::OK {
                return Err((self, MemoryError::DmaStatus(status)));
            }
            self.dma_active = false;
        }
        let memory = self.memory.as_ref().expect("shared DMA memory already consumed");
        let status = kernel::memory_unmap(memory.raw_handle());
        if status != catten_syscall::memory_status::OK {
            return Err((self, MemoryError::MemoryStatus(status)));
        }
        Ok(self.memory.take().expect("shared DMA memory already consumed"))
    }
}

impl Drop for SharedDmaMemory<'_> {
    fn drop(&mut self) {
        if let Some(memory) = self.memory.take() {
            if self.dma_active {
                let _ = kernel::dma_unmap(self.domain.raw_handle(), self.iova);
            }
            let _ = kernel::memory_unmap(memory.raw_handle());
            drop(memory);
        }
    }
}
