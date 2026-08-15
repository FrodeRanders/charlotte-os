//! Ownership-aware wrappers for memory and asynchronous kernel operations.
//!
//! The raw syscall crate intentionally mirrors the register ABI and therefore
//! uses copyable integers and addresses. This module is the safe application
//! layer: capability ownership is linear, mappings own their capability, DMA
//! consumes a CPU mapping, and an asynchronous read retains its mutable borrow
//! until the kernel has reached a terminal state.

use core::{
    marker::PhantomData,
    slice,
};

use catten_syscall::{
    self,
    DmaDirection,
};

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
pub struct OwnedMemory {
    cap: Option<u64>,
    len: usize,
}

impl OwnedMemory {
    pub fn allocate(pages: usize) -> Result<Self, MemoryError> {
        let cap = catten_syscall::memory_alloc(pages);
        if cap == 0 {
            return Err(MemoryError::AllocationFailed);
        }
        let len = catten_syscall::memory_size(cap);
        if len == 0 {
            let _ = catten_syscall::memory_close(cap);
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
        let len = catten_syscall::memory_size(cap);
        if len == 0 {
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

    fn raw_handle(&self) -> u64 {
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
        let iova =
            catten_syscall::dma_map_exclusive(domain.raw_handle(), self.raw_handle(), direction);
        if iova == 0 {
            return Err((self, MemoryError::DmaMapFailed));
        }
        Ok(DmaTransfer {
            domain,
            memory: Some(self),
            iova,
        })
    }

    fn map<Access>(self, writable: bool) -> Result<MappedMemory<Access>, (Self, MemoryError)> {
        let (status, base) = catten_syscall::memory_map_any(self.raw_handle(), writable);
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

impl Drop for OwnedMemory {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = catten_syscall::memory_close(cap);
        }
    }
}

pub enum ReadOnly {}
pub enum Writable {}

/// A CPU mapping which owns the underlying memory capability.
#[must_use = "dropping mapped memory unmaps and closes it"]
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

    pub fn unmap(mut self) -> Result<OwnedMemory, MemoryError> {
        let cap = self.memory.as_ref().expect("mapped memory already consumed").raw_handle();
        let status = catten_syscall::memory_unmap(cap);
        if status != catten_syscall::memory_status::OK {
            return Err(MemoryError::MemoryStatus(status));
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
            let _ = catten_syscall::memory_unmap(memory.raw_handle());
            drop(memory);
        }
    }
}

/// A borrowed DMA-domain capability. The grant remains owned by the launch
/// environment; this wrapper only prevents mixing it with memory handles.
pub struct DmaDomain(u64);

impl DmaDomain {
    /// # Safety
    /// `cap` must name a DMA domain granted to the current address space and
    /// must remain valid for every transfer created from this wrapper.
    pub const unsafe fn from_raw(cap: u64) -> Self {
        Self(cap)
    }

    const fn raw_handle(&self) -> u64 {
        self.0
    }
}

/// Memory exclusively owned by a device until [`finish`](Self::finish).
#[must_use = "a DMA transfer must be finished before CPU access resumes"]
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
        let status = catten_syscall::dma_unmap(self.domain.raw_handle(), self.iova);
        if status != catten_syscall::device_status::OK {
            return Err((self, MemoryError::DmaStatus(status)));
        }
        Ok(self.memory.take().expect("DMA memory already consumed"))
    }
}

impl Drop for DmaTransfer<'_> {
    fn drop(&mut self) {
        if self.memory.is_some() {
            let _ = catten_syscall::dma_unmap(self.domain.raw_handle(), self.iova);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    SubmissionFailed,
}

/// An in-flight read which retains exclusive ownership of its destination.
#[must_use = "dropping a read operation cancels and waits for it"]
pub struct ReadOperation<'buffer> {
    cap: Option<u64>,
    buffer: Option<&'buffer mut [u8]>,
}

impl<'buffer> ReadOperation<'buffer> {
    pub fn submit(buffer: &'buffer mut [u8]) -> Result<Self, ReadError> {
        let cap =
            unsafe { catten_syscall::submit_read(buffer.as_mut_ptr() as usize, buffer.len()) };
        if cap == catten_syscall::COMPLETION_SUBMIT_FAILED {
            return Err(ReadError::SubmissionFailed);
        }
        Ok(Self {
            cap: Some(cap),
            buffer: Some(buffer),
        })
    }

    /// Wait for the terminal completion and return the destination borrow.
    pub fn wait(mut self) -> &'buffer mut [u8] {
        let cap = self.cap.take().expect("read operation already completed");
        catten_syscall::wait(cap);
        catten_syscall::close(cap);
        self.buffer.take().expect("read buffer already returned")
    }
}

impl Drop for ReadOperation<'_> {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            // Cancellation is only a request. Waiting for terminal state is
            // what makes it sound to release the exclusive buffer borrow.
            catten_syscall::cancel(cap);
            catten_syscall::wait(cap);
            catten_syscall::close(cap);
        }
    }
}
