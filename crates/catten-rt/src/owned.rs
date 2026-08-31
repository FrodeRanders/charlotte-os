//! Ownership-aware wrappers for memory and asynchronous kernel operations.
//!
//! The raw syscall crate intentionally mirrors the register ABI and therefore
//! uses copyable integers and addresses. This module is the safe application
//! layer: capability ownership is linear, mappings own their capability, DMA
//! consumes a CPU mapping, and an asynchronous read retains its mutable borrow
//! until the kernel has reached a terminal state.

extern crate alloc;

use alloc::vec::Vec;
use core::{
    marker::PhantomData,
    slice,
};

use catten_syscall::{
    self,
    DmaDirection,
    IpcRights,
    OpCode,
};
use charlotte_lifecycle::ThreadIdentity;

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
    cap: Option<u64>,
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

    fn from_kernel(cap: u64) -> Result<Self, MemoryError> {
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

    const fn raw_handle(&self) -> u64 {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceError {
    MappingFailed(u64),
    Status(u64),
}

/// An owned MMIO capability before it is mapped into the process.
#[must_use = "dropping an MMIO capability closes it"]
#[derive(Debug)]
pub struct MmioRegion {
    cap: Option<u64>,
}

impl MmioRegion {
    /// Adopt a uniquely owned MMIO capability supplied at launch.
    ///
    /// # Safety
    /// `cap` must identify an MMIO grant owned by this address space and must
    /// not be used through the raw syscall API after adoption.
    pub const unsafe fn from_raw(cap: u64) -> Self {
        Self {
            cap: Some(cap),
        }
    }

    fn raw_handle(&self) -> u64 {
        self.cap.expect("MMIO capability already consumed")
    }

    pub fn map(mut self, writable: bool) -> Result<MappedMmio, (Self, DeviceError)> {
        let (status, base) = kernel::device_mmio_map_any(self.raw_handle(), writable);
        if status != catten_syscall::device_status::OK {
            return Err((self, DeviceError::MappingFailed(status)));
        }
        let cap = self.cap.take().expect("MMIO capability already consumed");
        Ok(MappedMmio {
            cap: Some(cap),
            base,
        })
    }
}

impl Drop for MmioRegion {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::device_close(cap);
        }
    }
}

/// An active MMIO mapping. Device-register access remains `unsafe` because
/// register width, alignment, volatility, and ordering are device-specific.
#[must_use = "dropping an MMIO mapping unmaps and closes it"]
#[derive(Debug)]
pub struct MappedMmio {
    cap: Option<u64>,
    base: usize,
}

impl MappedMmio {
    pub const fn as_ptr(&self) -> *mut u8 {
        self.base as *mut u8
    }

    pub fn unmap(mut self) -> Result<MmioRegion, (Self, DeviceError)> {
        let cap = self.cap.expect("MMIO capability already consumed");
        let status = kernel::device_mmio_unmap(cap);
        if status != catten_syscall::device_status::OK {
            return Err((self, DeviceError::Status(status)));
        }
        let _ = self.cap.take();
        Ok(MmioRegion {
            cap: Some(cap),
        })
    }
}

impl Drop for MappedMmio {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::device_mmio_unmap(cap);
            let _ = kernel::device_close(cap);
        }
    }
}

/// An owned interrupt capability.
#[must_use = "dropping an interrupt capability masks and closes it"]
#[derive(Debug)]
pub struct Interrupt {
    cap: Option<u64>,
}

impl Interrupt {
    /// Adopt a uniquely owned interrupt capability supplied at launch.
    ///
    /// # Safety
    /// `cap` must identify an interrupt grant owned by this address space and
    /// must not be used through the raw syscall API after adoption.
    pub const unsafe fn from_raw(cap: u64) -> Self {
        Self {
            cap: Some(cap),
        }
    }

    fn raw_handle(&self) -> u64 {
        self.cap.expect("interrupt capability already consumed")
    }

    pub fn bind_completion_queue(&self, cq: u32) -> Result<(), DeviceError> {
        let status = kernel::device_irq_bind_cq(self.raw_handle(), cq);
        if status == catten_syscall::device_status::OK {
            Ok(())
        } else {
            Err(DeviceError::Status(status))
        }
    }

    pub fn acknowledge(&self) -> Result<u64, DeviceError> {
        let (status, consumed) = kernel::device_irq_ack(self.raw_handle());
        if status == catten_syscall::device_status::OK {
            Ok(consumed)
        } else {
            Err(DeviceError::Status(status))
        }
    }
}

impl Drop for Interrupt {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::device_close(cap);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionError {
    SubmissionFailed,
    Status(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadError {
    ObserverRegistrationFailed,
    Completion(CompletionError),
}

/// A generation-bound handle to an EL0 thread.
///
/// Dropping the handle detaches the thread. Joining consumes the handle and
/// uses the spawn-time generation, so a recycled TID cannot be mistaken for
/// the original thread.
#[must_use = "dropping a thread handle detaches it"]
#[derive(Debug)]
pub struct ThreadHandle {
    identity: ThreadIdentity,
}

impl ThreadHandle {
    /// Spawn an EL0 thread pinned to `target_lp`.
    ///
    /// # Safety
    /// `entry_vaddr` must identify a valid `extern "C" fn()` entry point in
    /// the current address space and `target_lp` must identify a logical CPU.
    pub unsafe fn spawn(entry_vaddr: usize, target_lp: u32) -> Self {
        let (tid, generation) =
            unsafe { catten_syscall::spawn_thread_with_generation(entry_vaddr, target_lp) };
        Self {
            identity: ThreadIdentity::new(tid, generation),
        }
    }

    pub const fn id(&self) -> u64 {
        self.identity.tid()
    }

    pub fn join(self) -> Result<i64, ThreadError> {
        let cap = catten_syscall::observe_thread_exit_generation(
            self.identity.tid(),
            self.identity.generation(),
        );
        if cap == catten_syscall::COMPLETION_SUBMIT_FAILED {
            return Err(ThreadError::ObserverRegistrationFailed);
        }
        Completion::from_kernel(cap).and_then(Completion::wait).map_err(ThreadError::Completion)
    }
}

/// An owned completion capability.
///
/// Dropping a pending completion requests cancellation, waits for the terminal
/// state, and only then closes the capability. This is the appropriate wrapper
/// for timers, connection-close watches, and other buffer-free operations.
#[must_use = "dropping a completion cancels and closes it"]
#[derive(Debug)]
pub struct Completion {
    cap: Option<u64>,
}

impl Completion {
    pub fn submit(op: OpCode) -> Result<Self, CompletionError> {
        Self::from_kernel(kernel::submit(op))
    }

    pub fn timer(timeout_ms: u64) -> Result<Self, CompletionError> {
        Self::from_kernel(kernel::submit_timer(timeout_ms))
    }

    /// Adopt a uniquely owned completion capability.
    ///
    /// # Safety
    /// `cap` must be a live completion capability owned by the caller and no
    /// other value may close, poll, wait for, or cancel it after adoption.
    pub unsafe fn from_raw(cap: u64) -> Result<Self, CompletionError> {
        Self::from_kernel(cap)
    }

    fn from_kernel(cap: u64) -> Result<Self, CompletionError> {
        if cap == catten_syscall::COMPLETION_SUBMIT_FAILED {
            return Err(CompletionError::SubmissionFailed);
        }
        Ok(Self {
            cap: Some(cap),
        })
    }

    fn raw_handle(&self) -> u64 {
        self.cap.expect("completion capability already consumed")
    }

    fn finish_result(&mut self, status: u64, result: u64) -> Result<Option<i64>, CompletionError> {
        match status {
            catten_syscall::completion_status::READY => {
                let cap = self.cap.take().expect("completion capability already consumed");
                kernel::close(cap);
                Ok(Some(result as i64))
            }
            catten_syscall::completion_status::PENDING_OR_TIMEOUT => Ok(None),
            other => {
                let cap = self.cap.take().expect("completion capability already consumed");
                kernel::close(cap);
                Err(CompletionError::Status(other))
            }
        }
    }

    pub fn poll(&mut self) -> Result<Option<i64>, CompletionError> {
        let (status, result) = kernel::poll(self.raw_handle());
        self.finish_result(status, result)
    }

    pub fn wait_timeout(&mut self, timeout_ms: u64) -> Result<Option<i64>, CompletionError> {
        let (status, result) = kernel::wait_timeout(self.raw_handle(), timeout_ms);
        self.finish_result(status, result)
    }

    pub fn wait(mut self) -> Result<i64, CompletionError> {
        let cap = self.raw_handle();
        kernel::wait(cap);
        let (status, result) = kernel::poll(cap);
        match self.finish_result(status, result)? {
            Some(result) => Ok(result),
            None => {
                Err(CompletionError::Status(catten_syscall::completion_status::PENDING_OR_TIMEOUT))
            }
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            kernel::cancel(cap);
            kernel::wait(cap);
            kernel::close(cap);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    BorrowRequiresCall,
    CreationFailed,
    DescriptorMemory(MemoryError),
    DuplicateVectorMemory,
    EmptyVector,
    InvalidReturnedMemory,
    Status(u64),
    TooManyVectorEntries,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveError {
    EndpointClosed,
    InvalidReturnedMemory,
    Status(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactLaunchError {
    InvalidLength,
    Rejected,
    RetirementDenied,
}

/// Transfer a signed ELF and deployment descriptor to the privileged scoped
/// deployment gate. The kernel consumes both memory capabilities on every
/// submitted outcome; invalid lengths are rejected locally and normal `Drop`
/// releases both owners.
pub fn spawn_scoped_artifact(
    mut artifact: OwnedMemory,
    artifact_len: usize,
    artifact_name: u64,
    mut descriptor: OwnedMemory,
    descriptor_len: usize,
) -> Result<u64, ArtifactLaunchError> {
    if artifact_len == 0
        || artifact_len > artifact.len()
        || descriptor_len < charlotte_launch::deployment::HEADER_LEN
        || descriptor_len > descriptor.len()
        || descriptor_len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
    {
        return Err(ArtifactLaunchError::InvalidLength);
    }
    let artifact_cap = artifact.cap.take().expect("artifact capability already consumed");
    let descriptor_cap = descriptor.cap.take().expect("descriptor capability already consumed");
    let asid = kernel::spawn_artifact_scoped(
        artifact_cap,
        artifact_len,
        artifact_name,
        descriptor_cap,
        descriptor_len,
    );
    if asid == 0 {
        Err(ArtifactLaunchError::Rejected)
    } else {
        Ok(asid)
    }
}

/// Transfer a signed ELF and descriptor to the scoped deployment gate using
/// the descriptor's full artifact name as the authoritative identity.
///
/// This is the normal API for deployment descriptors. The packed-name form
/// above remains for ABI compatibility with early short-name callers.
pub fn spawn_scoped_artifact_named(
    artifact: OwnedMemory,
    artifact_len: usize,
    descriptor: OwnedMemory,
    descriptor_len: usize,
) -> Result<u64, ArtifactLaunchError> {
    spawn_scoped_artifact(artifact, artifact_len, 0, descriptor, descriptor_len)
}

/// Transfer a complete encrypted connector pickup to the kernel launch gate.
/// The package owner is consumed on every submitted outcome; plaintext is
/// never mapped into this caller. `artifact_name` identifies the resulting
/// retirement owner and must match the package's authenticated target.
pub fn launch_operational_connector(
    mut package: OwnedMemory,
    package_len: usize,
    artifact_name: &[u8],
) -> Result<DeployedArtifact, ArtifactLaunchError> {
    if package_len < charlotte_launch::operations_pickup::PICKUP_HEADER_LEN
        || package_len > package.len()
        || package_len > charlotte_launch::operations_pickup::MAX_PICKUP_LEN
        || !charlotte_launch::deployment::valid_artifact_name(artifact_name)
    {
        return Err(ArtifactLaunchError::InvalidLength);
    }
    let principal = charlotte_launch::artifact_principal_id(artifact_name);
    let package_cap = package.cap.take().expect("pickup capability already consumed");
    let asid = kernel::spawn_operational_connector(package_cap, package_len, principal);
    if asid == 0 {
        return Err(ArtifactLaunchError::Rejected);
    }
    Ok(DeployedArtifact {
        principal,
        asid,
        retired: false,
    })
}

/// An application domain created through the scoped deployment gate.
///
/// The owner remains with the deployment agent until retirement completes.
/// `poll_retire` is explicit because draining all domain threads can block;
/// `Drop` retains a best-effort abort fallback.
#[must_use = "dropping a deployed artifact requests best-effort retirement"]
pub struct DeployedArtifact {
    principal: u64,
    asid: u64,
    retired: bool,
}

impl DeployedArtifact {
    pub fn principal(&self) -> u64 {
        self.principal
    }

    pub fn asid(&self) -> u64 {
        self.asid
    }

    /// Request retirement and report whether reclamation has completed.
    pub fn poll_retire(&mut self) -> Result<bool, ArtifactLaunchError> {
        if self.retired {
            return Ok(true);
        }
        match kernel::retire_artifact_named(self.principal) {
            0 => {
                self.retired = true;
                Ok(true)
            }
            1 => Ok(false),
            _ => Err(ArtifactLaunchError::RetirementDenied),
        }
    }
}

impl Drop for DeployedArtifact {
    fn drop(&mut self) {
        if !self.retired {
            let _ = kernel::retire_artifact_named(self.principal);
        }
    }
}

/// Launch a scoped artifact and retain an owner that fences retirement by the
/// full signed artifact identity.
pub fn launch_scoped_artifact_named(
    artifact: OwnedMemory,
    artifact_len: usize,
    artifact_name: &[u8],
    descriptor: OwnedMemory,
    descriptor_len: usize,
) -> Result<DeployedArtifact, ArtifactLaunchError> {
    if artifact_name.is_empty()
        || artifact_name.len() > charlotte_launch::deployment::MAX_ARTIFACT_NAME_LEN
    {
        return Err(ArtifactLaunchError::InvalidLength);
    }
    let asid = spawn_scoped_artifact_named(artifact, artifact_len, descriptor, descriptor_len)?;
    Ok(DeployedArtifact {
        principal: charlotte_launch::artifact_principal_id(artifact_name),
        asid,
        retired: false,
    })
}

/// Launch a legacy short-name artifact while retaining principal-fenced
/// retirement ownership. New production deployments should use a signed
/// descriptor and [`launch_scoped_artifact_named`].
pub fn launch_artifact(
    mut artifact: OwnedMemory,
    artifact_len: usize,
    artifact_name: &[u8],
) -> Result<DeployedArtifact, ArtifactLaunchError> {
    if artifact_len == 0
        || artifact_len > artifact.len()
        || artifact_name.is_empty()
        || artifact_name.len() > 8
    {
        return Err(ArtifactLaunchError::InvalidLength);
    }
    let mut packed = [0u8; 8];
    packed[..artifact_name.len()].copy_from_slice(artifact_name);
    let artifact_cap = artifact.cap.take().expect("artifact capability already consumed");
    let asid = kernel::spawn_artifact(artifact_cap, artifact_len, u64::from_le_bytes(packed));
    if asid == 0 {
        return Err(ArtifactLaunchError::Rejected);
    }
    Ok(DeployedArtifact {
        principal: charlotte_launch::artifact_principal_id(artifact_name),
        asid,
        retired: false,
    })
}

/// An owned IPC endpoint capability.
#[must_use = "dropping an endpoint closes it"]
#[derive(Debug)]
pub struct Endpoint {
    cap: Option<u64>,
}

impl Endpoint {
    pub fn create(interface: u64, version: u32, capacity: usize) -> Result<Self, IpcError> {
        let cap = kernel::ipc_endpoint_create(interface, version, capacity);
        if cap == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok(Self {
            cap: Some(cap),
        })
    }

    /// Adopt a uniquely owned endpoint capability.
    ///
    /// # Safety
    /// `cap` must be a live endpoint owned by the caller and must not be used
    /// through the raw syscall API after adoption.
    pub const unsafe fn from_raw(cap: u64) -> Result<Self, IpcError> {
        if cap == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok(Self {
            cap: Some(cap),
        })
    }

    fn raw_handle(&self) -> u64 {
        self.cap.expect("endpoint capability already consumed")
    }

    /// Receive one queued request without blocking.
    ///
    /// Every capability attached to a successful message is immediately
    /// adopted by an owning Rust value. Dropping the returned message therefore
    /// releases attachments and cancels an unused reply token.
    pub fn try_receive(&self) -> Result<Option<IncomingMessage>, ReceiveError> {
        IncomingMessage::from_kernel(kernel::ipc_recv(self.raw_handle()))
    }

    /// Wait for and receive one request.
    pub fn receive(&self) -> Result<IncomingMessage, ReceiveError> {
        IncomingMessage::from_kernel(kernel::ipc_recv_block(self.raw_handle()))?
            .ok_or(ReceiveError::Status(catten_syscall::ipc_status::NO_MESSAGE))
    }

    /// Receive one queued request with the kernel-authenticated sender
    /// generation, principal, and supervisor roles populated.
    ///
    /// Authority-mediating services must use this form; the legacy receive
    /// ABI deliberately leaves those fields zero for compatibility.
    pub fn try_receive_authenticated(&self) -> Result<Option<IncomingMessage>, ReceiveError> {
        IncomingMessage::from_kernel(kernel::ipc_recv_authenticated(self.raw_handle()))
    }

    /// Block for a request with a kernel-authenticated sender envelope.
    pub fn receive_authenticated(&self) -> Result<IncomingMessage, ReceiveError> {
        IncomingMessage::from_kernel(kernel::ipc_recv_block_authenticated(self.raw_handle()))?
            .ok_or(ReceiveError::Status(catten_syscall::ipc_status::NO_MESSAGE))
    }

    pub fn connect(&self, rights: IpcRights) -> Result<Connection, IpcError> {
        Connection::from_kernel(kernel::ipc_connect(self.raw_handle(), rights))
            .ok_or(IpcError::CreationFailed)
    }

    pub fn bind_completion_queue(&self, cq: u32) -> Result<(), IpcError> {
        let status = kernel::ipc_endpoint_bind_cq(self.raw_handle(), cq);
        if status == catten_syscall::ipc_status::OK {
            Ok(())
        } else {
            Err(IpcError::Status(status))
        }
    }

    pub fn into_raw(mut self) -> u64 {
        self.cap.take().expect("endpoint capability already consumed")
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::ipc_close(cap);
        }
    }
}

#[derive(Debug)]
enum VectorItem<'memory> {
    Copy(&'memory OwnedMemory),
    Move(OwnedMemory),
    BorrowRead(&'memory OwnedMemory),
    BorrowWrite(&'memory mut OwnedMemory),
}

impl VectorItem<'_> {
    fn cap(&self) -> u64 {
        match self {
            Self::Copy(memory) | Self::BorrowRead(memory) => memory.raw_handle(),
            Self::Move(memory) => memory.raw_handle(),
            Self::BorrowWrite(memory) => memory.raw_handle(),
        }
    }

    const fn mode(&self) -> u32 {
        match self {
            Self::Copy(_) => 0,
            Self::Move(_) => 1,
            Self::BorrowRead(_) => 2,
            Self::BorrowWrite(_) => 3,
        }
    }
}

/// Builder for an IPC call carrying a mixed vector of copied, moved, and
/// borrowed memory objects.
///
/// The builder owns moved objects and retains Rust borrows for loaned objects.
/// On submission failure it is returned intact; on success moved capabilities
/// are consumed and all loans remain tied to the returned [`PendingCall`].
#[must_use = "a capability vector has no effect until submitted"]
#[derive(Debug)]
pub struct CapabilityVector<'memory> {
    items: Vec<VectorItem<'memory>>,
}

impl<'memory> CapabilityVector<'memory> {
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn validate_new_cap(&self, cap: u64) -> Result<(), IpcError> {
        if self.items.len() >= catten_syscall::CAP_VECTOR_MAX {
            return Err(IpcError::TooManyVectorEntries);
        }
        if self.items.iter().any(|item| item.cap() == cap) {
            return Err(IpcError::DuplicateVectorMemory);
        }
        Ok(())
    }

    pub fn push_copy(&mut self, memory: &'memory OwnedMemory) -> Result<(), IpcError> {
        self.validate_new_cap(memory.raw_handle())?;
        self.items.push(VectorItem::Copy(memory));
        Ok(())
    }

    pub fn push_move(&mut self, memory: OwnedMemory) -> Result<(), (OwnedMemory, IpcError)> {
        if let Err(error) = self.validate_new_cap(memory.raw_handle()) {
            return Err((memory, error));
        }
        self.items.push(VectorItem::Move(memory));
        Ok(())
    }

    pub fn push_borrow_read(&mut self, memory: &'memory OwnedMemory) -> Result<(), IpcError> {
        self.validate_new_cap(memory.raw_handle())?;
        self.items.push(VectorItem::BorrowRead(memory));
        Ok(())
    }

    pub fn push_borrow_write(&mut self, memory: &'memory mut OwnedMemory) -> Result<(), IpcError> {
        self.validate_new_cap(memory.raw_handle())?;
        self.items.push(VectorItem::BorrowWrite(memory));
        Ok(())
    }

    fn descriptor(&self) -> Result<OwnedMemory, IpcError> {
        if self.items.is_empty() {
            return Err(IpcError::EmptyVector);
        }
        let descriptor = OwnedMemory::allocate(1).map_err(IpcError::DescriptorMemory)?;
        let mut mapping =
            descriptor.map_writable().map_err(|(_, error)| IpcError::DescriptorMemory(error))?;
        let bytes = mapping.as_mut_slice();
        bytes[..2].copy_from_slice(&(self.items.len() as u16).to_le_bytes());
        let entry_size = core::mem::size_of::<catten_syscall::CapVectorEntry>();
        for (index, item) in self.items.iter().enumerate() {
            let entry = catten_syscall::CapVectorEntry {
                cap: item.cap(),
                mode: item.mode(),
                reserved: 0,
            };
            let entry_bytes = unsafe {
                slice::from_raw_parts(
                    (&entry as *const catten_syscall::CapVectorEntry).cast::<u8>(),
                    entry_size,
                )
            };
            let offset = 2 + index * entry_size;
            bytes[offset..offset + entry_size].copy_from_slice(entry_bytes);
        }
        mapping.unmap().map_err(|(_, error)| IpcError::DescriptorMemory(error))
    }

    fn commit_moves(&mut self) {
        for item in &mut self.items {
            if let VectorItem::Move(memory) = item {
                let _ = memory.cap.take().expect("moved vector memory already consumed");
            }
        }
    }

    fn can_send(&self) -> bool {
        self.items.iter().all(|item| matches!(item, VectorItem::Copy(_) | VectorItem::Move(_)))
    }
}

impl Default for CapabilityVector<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// A uniquely owned IPC connection capability.
#[must_use = "dropping a connection closes its capability"]
#[derive(Debug)]
pub struct Connection {
    cap: Option<u64>,
}

/// A non-owning view of an IPC connection.
///
/// This is used for launch-provided connections whose lifetime is controlled
/// by the process environment. It is `Copy`, but cannot close or transfer the
/// underlying connection capability.
#[derive(Clone, Copy, Debug)]
pub struct ConnectionRef<'connection> {
    cap: u64,
    _connection: PhantomData<&'connection Connection>,
}

impl<'connection> ConnectionRef<'connection> {
    /// Borrow a valid connection capability for `lifetime`.
    ///
    /// # Safety
    /// The capability must remain live for the returned value's lifetime and
    /// must not be closed through the raw syscall API during that time.
    pub const unsafe fn from_raw(cap: u64) -> Result<Self, IpcError> {
        if cap == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok(Self {
            cap,
            _connection: PhantomData,
        })
    }

    pub const fn as_raw(self) -> u64 {
        self.cap
    }

    pub fn send(self, opcode: u32, arg0: u64) -> Result<(), IpcError> {
        let status = kernel::ipc_scalar_send(self.cap, opcode, arg0);
        if status == catten_syscall::ipc_status::OK {
            Ok(())
        } else {
            Err(IpcError::Status(status))
        }
    }

    pub fn call(self, opcode: u32, arg0: u64) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call(self.cap, opcode, arg0))
    }

    pub fn call_move(
        self,
        opcode: u32,
        arg0: u64,
        mut memory: OwnedMemory,
    ) -> Result<PendingCall<'static>, (OwnedMemory, IpcError)> {
        let call = kernel::ipc_scalar_call_move(self.cap, opcode, arg0, memory.raw_handle());
        if call == 0 {
            return Err((memory, IpcError::CreationFailed));
        }
        let _ = memory.cap.take().expect("owned memory capability already consumed");
        Ok(PendingCall::from_valid_cap(call))
    }

    pub fn call_borrow_read<'memory>(
        self,
        opcode: u32,
        arg0: u64,
        memory: &'memory OwnedMemory,
    ) -> Result<PendingCall<'memory>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_borrow_read(
            self.cap,
            opcode,
            arg0,
            memory.raw_handle(),
        ))
    }

    pub fn call_borrow_write<'memory>(
        self,
        opcode: u32,
        arg0: u64,
        memory: &'memory mut OwnedMemory,
    ) -> Result<PendingCall<'memory>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_borrow_write(
            self.cap,
            opcode,
            arg0,
            memory.raw_handle(),
        ))
    }

    pub fn call_copy(
        self,
        opcode: u32,
        arg0: u64,
        memory: &OwnedMemory,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_copy(
            self.cap,
            opcode,
            arg0,
            memory.raw_handle(),
        ))
    }

    pub fn call_connection(
        self,
        opcode: u32,
        arg0: u64,
        endpoint: &Endpoint,
        rights: IpcRights,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_connection(
            self.cap,
            opcode,
            arg0,
            endpoint.raw_handle(),
            rights,
        ))
    }

    pub fn call_connection_copy(
        self,
        opcode: u32,
        arg0: u64,
        endpoint: &Endpoint,
        rights: IpcRights,
        memory: &OwnedMemory,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_connection_copy(
            self.cap,
            opcode,
            arg0,
            endpoint.raw_handle(),
            rights,
            memory.raw_handle(),
        ))
    }

    /// Call while re-delegating from a mintable connection and copying a
    /// memory object. This is used by mediation services that receive an
    /// application's endpoint connection but do not own that endpoint.
    pub fn call_delegated_connection_copy(
        self,
        opcode: u32,
        arg0: u64,
        connection: ConnectionRef<'_>,
        rights: IpcRights,
        memory: &OwnedMemory,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_connection_copy(
            self.cap,
            opcode,
            arg0,
            connection.cap,
            rights,
            memory.raw_handle(),
        ))
    }
}

impl Connection {
    /// Adopt a uniquely owned connection capability.
    ///
    /// # Safety
    /// `cap` must be a live connection capability owned by the caller. It must
    /// not be used through raw syscalls or adopted again after this call.
    pub const unsafe fn from_raw(cap: u64) -> Result<Self, IpcError> {
        if cap == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok(Self {
            cap: Some(cap),
        })
    }

    fn from_kernel(cap: u64) -> Option<Self> {
        if cap == 0 {
            None
        } else {
            Some(Self {
                cap: Some(cap),
            })
        }
    }

    fn raw_handle(&self) -> u64 {
        self.cap.expect("connection capability already consumed")
    }

    /// Temporarily expose the handle for a low-level API that does not take
    /// ownership. Application code should prefer the typed methods on this
    /// value. The returned integer must never be closed or adopted.
    pub const fn as_raw(&self) -> u64 {
        self.cap.expect("connection capability already consumed")
    }

    pub fn as_ref(&self) -> ConnectionRef<'_> {
        ConnectionRef {
            cap: self.raw_handle(),
            _connection: PhantomData,
        }
    }

    pub fn send(&self, opcode: u32, arg0: u64) -> Result<(), IpcError> {
        let status = kernel::ipc_scalar_send(self.raw_handle(), opcode, arg0);
        if status == catten_syscall::ipc_status::OK {
            Ok(())
        } else {
            Err(IpcError::Status(status))
        }
    }

    pub fn call(&self, opcode: u32, arg0: u64) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call(self.raw_handle(), opcode, arg0))
    }

    pub fn send_move(
        &self,
        opcode: u32,
        arg0: u64,
        mut memory: OwnedMemory,
    ) -> Result<(), (OwnedMemory, IpcError)> {
        let status =
            kernel::ipc_scalar_send_move(self.raw_handle(), opcode, arg0, memory.raw_handle());
        if status != catten_syscall::ipc_status::OK {
            return Err((memory, IpcError::Status(status)));
        }
        let _ = memory.cap.take().expect("owned memory capability already consumed");
        Ok(())
    }

    pub fn call_move(
        &self,
        opcode: u32,
        arg0: u64,
        mut memory: OwnedMemory,
    ) -> Result<PendingCall<'static>, (OwnedMemory, IpcError)> {
        let call =
            kernel::ipc_scalar_call_move(self.raw_handle(), opcode, arg0, memory.raw_handle());
        if call == 0 {
            return Err((memory, IpcError::CreationFailed));
        }
        let _ = memory.cap.take().expect("owned memory capability already consumed");
        Ok(PendingCall::from_valid_cap(call))
    }

    pub fn call_borrow_read<'memory>(
        &self,
        opcode: u32,
        arg0: u64,
        memory: &'memory OwnedMemory,
    ) -> Result<PendingCall<'memory>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_borrow_read(
            self.raw_handle(),
            opcode,
            arg0,
            memory.raw_handle(),
        ))
    }

    pub fn call_borrow_write<'memory>(
        &self,
        opcode: u32,
        arg0: u64,
        memory: &'memory mut OwnedMemory,
    ) -> Result<PendingCall<'memory>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_borrow_write(
            self.raw_handle(),
            opcode,
            arg0,
            memory.raw_handle(),
        ))
    }

    pub fn call_copy(
        &self,
        opcode: u32,
        arg0: u64,
        memory: &OwnedMemory,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_copy(
            self.raw_handle(),
            opcode,
            arg0,
            memory.raw_handle(),
        ))
    }

    /// Call while delegating a connection minted from `endpoint`.
    ///
    /// Neither `endpoint` nor this connection is transferred by the call.
    pub fn call_connection(
        &self,
        opcode: u32,
        arg0: u64,
        endpoint: &Endpoint,
        rights: IpcRights,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_connection(
            self.raw_handle(),
            opcode,
            arg0,
            endpoint.raw_handle(),
            rights,
        ))
    }

    /// Call while delegating a connection and copying a memory object.
    pub fn call_connection_copy(
        &self,
        opcode: u32,
        arg0: u64,
        endpoint: &Endpoint,
        rights: IpcRights,
        memory: &OwnedMemory,
    ) -> Result<PendingCall<'static>, IpcError> {
        PendingCall::from_kernel(kernel::ipc_scalar_call_connection_copy(
            self.raw_handle(),
            opcode,
            arg0,
            endpoint.raw_handle(),
            rights,
            memory.raw_handle(),
        ))
    }

    pub fn send_vector<'memory>(
        &self,
        opcode: u32,
        arg0: u64,
        mut vector: CapabilityVector<'memory>,
    ) -> Result<(), (CapabilityVector<'memory>, IpcError)> {
        if !vector.can_send() {
            return Err((vector, IpcError::BorrowRequiresCall));
        }
        let mut descriptor = match vector.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return Err((vector, error)),
        };
        let status =
            kernel::ipc_vector_send(self.raw_handle(), opcode, arg0, descriptor.raw_handle());
        if status != catten_syscall::ipc_status::OK {
            return Err((vector, IpcError::Status(status)));
        }
        let _ = descriptor.cap.take().expect("vector descriptor already consumed");
        vector.commit_moves();
        Ok(())
    }

    pub fn call_vector<'memory>(
        &self,
        opcode: u32,
        arg0: u64,
        mut vector: CapabilityVector<'memory>,
    ) -> Result<PendingCall<'memory>, (CapabilityVector<'memory>, IpcError)> {
        let mut descriptor = match vector.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return Err((vector, error)),
        };
        let call =
            kernel::ipc_vector_call(self.raw_handle(), opcode, arg0, descriptor.raw_handle());
        if call == 0 {
            return Err((vector, IpcError::CreationFailed));
        }
        let _ = descriptor.cap.take().expect("vector descriptor already consumed");
        vector.commit_moves();
        Ok(PendingCall::from_valid_cap(call))
    }

    pub fn watch_closed(&self) -> Result<Completion, CompletionError> {
        Completion::from_kernel(kernel::ipc_connection_watch_closed(self.raw_handle()))
    }

    pub fn into_raw(mut self) -> u64 {
        self.cap.take().expect("connection capability already consumed")
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::ipc_close(cap);
        }
    }
}

/// A received call's reply authority.
///
/// Reply operations consume this value, so a request cannot be replied to
/// twice. Dropping an unused token closes it and wakes/cancels the caller
/// according to the IPC contract.
#[must_use = "a reply token must be consumed by a reply or explicitly dropped"]
#[derive(Debug)]
pub struct ReplyToken {
    cap: Option<u64>,
}

impl ReplyToken {
    fn from_kernel(cap: u64) -> Option<Self> {
        if cap == 0 {
            None
        } else {
            Some(Self {
                cap: Some(cap),
            })
        }
    }

    pub fn reply(mut self, result: i64) -> Result<(), IpcError> {
        let cap = self.cap.take().expect("reply token already consumed");
        let status = kernel::ipc_reply(cap, result);
        if status == catten_syscall::ipc_status::OK {
            Ok(())
        } else {
            let _ = kernel::ipc_close(cap);
            Err(IpcError::Status(status))
        }
    }

    pub fn reply_move(
        mut self,
        mut memory: OwnedMemory,
        result: i64,
    ) -> Result<(), (OwnedMemory, IpcError)> {
        let reply = self.cap.take().expect("reply token already consumed");
        let status = kernel::ipc_reply_move(reply, memory.raw_handle(), result);
        if status == catten_syscall::ipc_status::OK {
            let _ = memory.cap.take().expect("owned memory capability already consumed");
            Ok(())
        } else {
            let _ = kernel::ipc_close(reply);
            Err((memory, IpcError::Status(status)))
        }
    }

    pub fn reply_connection(
        mut self,
        endpoint: &Endpoint,
        rights: IpcRights,
        result: i64,
    ) -> Result<(), IpcError> {
        let reply = self.cap.take().expect("reply token already consumed");
        let status = kernel::ipc_reply_connection(reply, endpoint.raw_handle(), rights, result);
        if status == catten_syscall::ipc_status::OK {
            Ok(())
        } else {
            let _ = kernel::ipc_close(reply);
            Err(IpcError::Status(status))
        }
    }

    /// Reply with an attenuated connection minted from a re-delegable
    /// connection. This is the mediation counterpart to `reply_connection`:
    /// a controller need not own the target service's endpoint.
    pub fn reply_connection_ref(
        mut self,
        connection: ConnectionRef<'_>,
        rights: IpcRights,
        result: i64,
    ) -> Result<(), IpcError> {
        let reply = self.cap.take().expect("reply token already consumed");
        let status = kernel::ipc_reply_connection(reply, connection.cap, rights, result);
        if status == catten_syscall::ipc_status::OK {
            Ok(())
        } else {
            let _ = kernel::ipc_close(reply);
            Err(IpcError::Status(status))
        }
    }
}

impl Drop for ReplyToken {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::ipc_close(cap);
        }
    }
}

/// A received IPC message whose attached capabilities have unique owners.
///
/// Moving fields out of this value transfers their ownership. Any fields left
/// behind are closed automatically.
#[must_use = "dropping an incoming message releases all attached capabilities"]
#[derive(Debug)]
pub struct IncomingMessage {
    pub opcode: u32,
    pub arg0: u64,
    pub sender: u64,
    pub sender_generation: u64,
    pub sender_principal: u64,
    pub sender_roles: u32,
    pub interface: u64,
    pub version: u32,
    pub reply: Option<ReplyToken>,
    pub memory: Option<OwnedMemory>,
    pub connection: Option<Connection>,
}

impl IncomingMessage {
    fn from_kernel(message: catten_syscall::IpcMessage) -> Result<Option<Self>, ReceiveError> {
        if message.status == catten_syscall::ipc_status::NO_MESSAGE {
            return Ok(None);
        }
        if message.status == catten_syscall::ipc_status::ENDPOINT_CLOSED {
            return Err(ReceiveError::EndpointClosed);
        }
        if message.status != catten_syscall::ipc_status::OK {
            close_raw_message_capabilities(message);
            return Err(ReceiveError::Status(message.status));
        }

        let memory = if message.memory == 0 {
            None
        } else {
            match OwnedMemory::from_kernel(message.memory) {
                Ok(memory) => Some(memory),
                Err(_) => {
                    if message.connection != 0 {
                        let _ = kernel::ipc_close(message.connection);
                    }
                    if message.reply != 0 {
                        let _ = kernel::ipc_close(message.reply);
                    }
                    return Err(ReceiveError::InvalidReturnedMemory);
                }
            }
        };

        Ok(Some(Self {
            opcode: message.opcode,
            arg0: message.arg0,
            sender: message.sender,
            sender_generation: message.sender_generation,
            sender_principal: message.sender_principal,
            sender_roles: message.sender_roles,
            interface: message.interface,
            version: message.version,
            reply: ReplyToken::from_kernel(message.reply),
            memory,
            connection: Connection::from_kernel(message.connection),
        }))
    }
}

fn close_raw_message_capabilities(message: catten_syscall::IpcMessage) {
    if message.memory != 0 {
        let _ = kernel::memory_close(message.memory);
    }
    if message.connection != 0 {
        let _ = kernel::ipc_close(message.connection);
    }
    if message.reply != 0 {
        let _ = kernel::ipc_close(message.reply);
    }
}

#[derive(Debug)]
pub struct CallResult {
    pub result: i64,
    pub connection: Option<Connection>,
    pub memory: Option<OwnedMemory>,
}

/// A pending IPC call. Its lifetime retains any memory loan until the reply is
/// observed or the call is closed by `Drop`.
#[must_use = "dropping a pending call cancels it and revokes attached loans"]
#[derive(Debug)]
pub struct PendingCall<'memory> {
    cap: Option<u64>,
    _loan: PhantomData<&'memory mut OwnedMemory>,
}

impl PendingCall<'_> {
    fn from_kernel(cap: u64) -> Result<Self, IpcError> {
        if cap == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok(Self::from_valid_cap(cap))
    }

    fn from_valid_cap(cap: u64) -> Self {
        Self {
            cap: Some(cap),
            _loan: PhantomData,
        }
    }

    fn raw_handle(&self) -> u64 {
        self.cap.expect("pending-call capability already consumed")
    }

    fn finish(
        &mut self,
        status: u64,
        result: u64,
        connection: u64,
        memory: u64,
    ) -> Result<Option<CallResult>, IpcError> {
        if status == 1 {
            return Ok(None);
        }
        let cap = self.cap.take().expect("pending-call capability already consumed");
        let _ = kernel::ipc_close(cap);
        if status != catten_syscall::ipc_status::OK {
            return Err(IpcError::Status(status));
        }
        let connection = Connection::from_kernel(connection);
        let memory = if memory == 0 {
            None
        } else {
            Some(OwnedMemory::from_kernel(memory).map_err(|_| IpcError::InvalidReturnedMemory)?)
        };
        Ok(Some(CallResult {
            result: result as i64,
            connection,
            memory,
        }))
    }

    pub fn poll(&mut self) -> Result<Option<CallResult>, IpcError> {
        let (status, result, connection, memory) =
            kernel::ipc_reply_poll_with_memory(self.raw_handle());
        self.finish(status, result, connection, memory)
    }

    pub fn wait(mut self) -> Result<CallResult, IpcError> {
        let (status, result, connection, memory) =
            kernel::ipc_reply_wait_with_memory(self.raw_handle());
        self.finish(status, result, connection, memory)?.ok_or(IpcError::Status(1))
    }
}

impl Drop for PendingCall<'_> {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::ipc_close(cap);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    SubmissionFailed,
}

/// An in-flight read which retains exclusive ownership of its destination.
#[must_use = "dropping a read operation cancels and waits for it"]
#[derive(Debug)]
pub struct ReadOperation<'buffer> {
    cap: Option<u64>,
    buffer: Option<&'buffer mut [u8]>,
}

impl<'buffer> ReadOperation<'buffer> {
    pub fn submit(buffer: &'buffer mut [u8]) -> Result<Self, ReadError> {
        let cap = unsafe { kernel::submit_read(buffer.as_mut_ptr() as usize, buffer.len()) };
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
        kernel::wait(cap);
        kernel::close(cap);
        self.buffer.take().expect("read buffer already returned")
    }
}

impl Drop for ReadOperation<'_> {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            // Cancellation is only a request. Waiting for terminal state is
            // what makes it sound to release the exclusive buffer borrow.
            kernel::cancel(cap);
            kernel::wait(cap);
            kernel::close(cap);
        }
    }
}

#[cfg(not(test))]
mod kernel {
    use catten_syscall::{
        self,
        DmaDirection,
        IpcRights,
        OpCode,
    };

    pub fn memory_alloc(pages: usize) -> u64 {
        catten_syscall::memory_alloc(pages)
    }

    pub fn memory_size(cap: u64) -> usize {
        catten_syscall::memory_size(cap)
    }

    pub fn memory_map_any(cap: u64, writable: bool) -> (u64, usize) {
        catten_syscall::memory_map_any(cap, writable)
    }

    pub fn memory_unmap(cap: u64) -> u64 {
        catten_syscall::memory_unmap(cap)
    }

    pub fn memory_close(cap: u64) -> u64 {
        catten_syscall::memory_close(cap)
    }

    pub fn dma_map_exclusive(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
        catten_syscall::dma_map_exclusive(domain, memory, direction)
    }

    pub fn dma_map(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
        // SAFETY: the owning caller keeps both capabilities live and pairs a
        // successful mapping with `dma_unmap` before releasing either one.
        unsafe { catten_syscall::dma_map(domain, memory, direction) }
    }

    pub fn dma_unmap(domain: u64, iova: u64) -> u64 {
        catten_syscall::dma_unmap(domain, iova)
    }

    pub fn device_mmio_map_any(cap: u64, writable: bool) -> (u64, usize) {
        catten_syscall::device_mmio_map_any(cap, writable)
    }

    pub fn device_mmio_unmap(cap: u64) -> u64 {
        catten_syscall::device_mmio_unmap(cap)
    }

    pub fn device_irq_bind_cq(cap: u64, cq: u32) -> u64 {
        catten_syscall::device_irq_bind_cq(cap, cq)
    }

    pub fn device_irq_ack(cap: u64) -> (u64, u64) {
        catten_syscall::device_irq_ack(cap)
    }

    pub fn device_close(cap: u64) -> u64 {
        catten_syscall::device_close(cap)
    }

    pub fn submit(op: OpCode) -> u64 {
        catten_syscall::submit(op)
    }

    pub fn submit_timer(timeout_ms: u64) -> u64 {
        catten_syscall::submit_timer(timeout_ms)
    }

    pub unsafe fn submit_read(buf_ptr: usize, buf_len: usize) -> u64 {
        unsafe { catten_syscall::submit_read(buf_ptr, buf_len) }
    }

    pub fn poll(cap: u64) -> (u64, u64) {
        catten_syscall::poll(cap)
    }

    pub fn wait(cap: u64) {
        catten_syscall::wait(cap);
    }

    pub fn wait_timeout(cap: u64, timeout_ms: u64) -> (u64, u64) {
        catten_syscall::wait_timeout(cap, timeout_ms)
    }

    pub fn cancel(cap: u64) {
        catten_syscall::cancel(cap);
    }

    pub fn close(cap: u64) {
        catten_syscall::close(cap);
    }

    pub fn ipc_scalar_send(connection: u64, opcode: u32, arg0: u64) -> u64 {
        catten_syscall::ipc_scalar_send(connection, opcode, arg0)
    }

    pub fn ipc_endpoint_create(interface: u64, version: u32, capacity: usize) -> u64 {
        catten_syscall::ipc_endpoint_create(interface, version, capacity)
    }

    pub fn ipc_connect(endpoint: u64, rights: IpcRights) -> u64 {
        catten_syscall::ipc_connect(endpoint, rights)
    }

    pub fn ipc_endpoint_bind_cq(endpoint: u64, cq: u32) -> u64 {
        catten_syscall::ipc_endpoint_bind_cq(endpoint, cq)
    }

    pub fn ipc_recv(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv(endpoint)
    }

    pub fn ipc_recv_block(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv_block(endpoint)
    }

    pub fn ipc_recv_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv_authenticated(endpoint)
    }

    pub fn ipc_recv_block_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv_block_authenticated(endpoint)
    }

    pub fn ipc_scalar_call(connection: u64, opcode: u32, arg0: u64) -> u64 {
        catten_syscall::ipc_scalar_call(connection, opcode, arg0)
    }

    pub fn ipc_scalar_send_move(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
        catten_syscall::ipc_scalar_send_move(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_move(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
        catten_syscall::ipc_scalar_call_move(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_borrow_read(
        connection: u64,
        opcode: u32,
        arg0: u64,
        memory: u64,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_borrow_read(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_borrow_write(
        connection: u64,
        opcode: u32,
        arg0: u64,
        memory: u64,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_borrow_write(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_copy(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
        catten_syscall::ipc_scalar_call_copy(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_connection(
        connection: u64,
        opcode: u32,
        arg0: u64,
        endpoint: u64,
        rights: IpcRights,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_connection(connection, opcode, arg0, endpoint, rights)
    }

    pub fn ipc_scalar_call_connection_copy(
        connection: u64,
        opcode: u32,
        arg0: u64,
        endpoint: u64,
        rights: IpcRights,
        memory: u64,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_connection_copy(
            connection, opcode, arg0, endpoint, rights, memory,
        )
    }

    pub fn ipc_vector_send(connection: u64, opcode: u32, arg0: u64, descriptor: u64) -> u64 {
        catten_syscall::ipc_vector_send(connection, opcode, arg0, descriptor)
    }

    pub fn ipc_vector_call(connection: u64, opcode: u32, arg0: u64, descriptor: u64) -> u64 {
        catten_syscall::ipc_vector_call(connection, opcode, arg0, descriptor)
    }

    pub fn ipc_connection_watch_closed(connection: u64) -> u64 {
        catten_syscall::ipc_connection_watch_closed(connection)
    }

    pub fn ipc_reply_poll_with_memory(call: u64) -> (u64, u64, u64, u64) {
        catten_syscall::ipc_reply_poll_with_memory(call)
    }

    pub fn ipc_reply_wait_with_memory(call: u64) -> (u64, u64, u64, u64) {
        catten_syscall::ipc_reply_wait_with_memory(call)
    }

    pub fn ipc_reply(reply: u64, result: i64) -> u64 {
        catten_syscall::ipc_reply(reply, result)
    }

    pub fn ipc_reply_move(reply: u64, memory: u64, result: i64) -> u64 {
        catten_syscall::ipc_reply_move(reply, memory, result)
    }

    pub fn ipc_reply_connection(reply: u64, endpoint: u64, rights: IpcRights, result: i64) -> u64 {
        catten_syscall::ipc_reply_connection(reply, endpoint, rights, result)
    }

    pub fn ipc_close(cap: u64) -> u64 {
        catten_syscall::ipc_close(cap)
    }

    pub fn spawn_artifact_scoped(
        artifact: u64,
        artifact_len: usize,
        artifact_name: u64,
        descriptor: u64,
        descriptor_len: usize,
    ) -> u64 {
        catten_syscall::spawn_artifact_scoped(
            artifact,
            artifact_len,
            artifact_name,
            descriptor,
            descriptor_len,
        )
    }

    pub fn spawn_operational_connector(package: u64, package_len: usize, principal: u64) -> u64 {
        catten_syscall::spawn_operational_connector(package, package_len, principal)
    }

    pub fn spawn_artifact(artifact: u64, artifact_len: usize, artifact_name: u64) -> u64 {
        catten_syscall::spawn_artifact(artifact, artifact_len, artifact_name)
    }

    pub fn retire_artifact_named(principal: u64) -> u64 {
        catten_syscall::retire_artifact_named(principal)
    }
}

#[cfg(test)]
mod kernel {
    extern crate std;

    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            MutexGuard,
        },
        vec::Vec,
    };

    use catten_syscall::{
        self,
        DmaDirection,
        IpcRights,
        OpCode,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Event {
        MemoryClose(u64),
        MemoryUnmap(u64),
        DmaUnmap(u64, u64),
        DeviceUnmap(u64),
        DeviceClose(u64),
        CompletionCancel(u64),
        CompletionWait(u64),
        CompletionClose(u64),
        IpcClose(u64),
    }

    pub struct State {
        pub memory_alloc: u64,
        pub memory_size: usize,
        pub memory_map_any: (u64, usize),
        pub memory_unmap: VecDeque<u64>,
        pub memory_close: VecDeque<u64>,
        pub dma_map: u64,
        pub dma_unmap: VecDeque<u64>,
        pub device_map: (u64, usize),
        pub device_unmap: VecDeque<u64>,
        pub device_bind: u64,
        pub device_ack: (u64, u64),
        pub submit: u64,
        pub poll: VecDeque<(u64, u64)>,
        pub wait_timeout: VecDeque<(u64, u64)>,
        pub ipc_send: u64,
        pub ipc_call: u64,
        pub ipc_endpoint: u64,
        pub ipc_connection: u64,
        pub ipc_bind: u64,
        pub ipc_receive: VecDeque<catten_syscall::IpcMessage>,
        pub ipc_send_move: u64,
        pub ipc_call_move: u64,
        pub ipc_vector_send: u64,
        pub ipc_vector_call: u64,
        pub ipc_reply: VecDeque<(u64, u64, u64, u64)>,
        pub ipc_reply_status: u64,
        pub connection_watch: u64,
        pub scoped_spawn: u64,
        pub events: Vec<Event>,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                memory_alloc: 10,
                memory_size: 4096,
                memory_map_any: (catten_syscall::memory_status::OK, 0x1000),
                memory_unmap: VecDeque::new(),
                memory_close: VecDeque::new(),
                dma_map: 0x2000,
                dma_unmap: VecDeque::new(),
                device_map: (catten_syscall::device_status::OK, 0x3000),
                device_unmap: VecDeque::new(),
                device_bind: catten_syscall::device_status::OK,
                device_ack: (catten_syscall::device_status::OK, 0),
                submit: 20,
                poll: VecDeque::new(),
                wait_timeout: VecDeque::new(),
                ipc_send: catten_syscall::ipc_status::OK,
                ipc_call: 30,
                ipc_endpoint: 40,
                ipc_connection: 41,
                ipc_bind: catten_syscall::ipc_status::OK,
                ipc_receive: VecDeque::new(),
                ipc_send_move: catten_syscall::ipc_status::OK,
                ipc_call_move: 30,
                ipc_vector_send: catten_syscall::ipc_status::OK,
                ipc_vector_call: 30,
                ipc_reply: VecDeque::new(),
                ipc_reply_status: catten_syscall::ipc_status::OK,
                connection_watch: 20,
                scoped_spawn: 2,
                events: Vec::new(),
            }
        }
    }

    static SERIAL: Mutex<()> = Mutex::new(());
    static STATE: Mutex<Option<State>> = Mutex::new(None);

    pub fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().expect("test serialization mutex poisoned")
    }

    pub fn reset() {
        *STATE.lock().expect("test kernel mutex poisoned") = Some(State::default());
    }

    pub fn update(f: impl FnOnce(&mut State)) {
        f(STATE
            .lock()
            .expect("test kernel mutex poisoned")
            .as_mut()
            .expect("test kernel is not initialized"));
    }

    pub fn events() -> Vec<Event> {
        STATE
            .lock()
            .expect("test kernel mutex poisoned")
            .as_ref()
            .expect("test kernel is not initialized")
            .events
            .clone()
    }

    fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
        f(STATE
            .lock()
            .expect("test kernel mutex poisoned")
            .as_mut()
            .expect("test kernel is not initialized"))
    }

    pub fn memory_alloc(_pages: usize) -> u64 {
        with_state(|state| state.memory_alloc)
    }

    pub fn memory_size(_cap: u64) -> usize {
        with_state(|state| state.memory_size)
    }

    pub fn memory_map_any(_cap: u64, _writable: bool) -> (u64, usize) {
        with_state(|state| state.memory_map_any)
    }

    pub fn memory_unmap(cap: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::MemoryUnmap(cap));
            state.memory_unmap.pop_front().unwrap_or(catten_syscall::memory_status::OK)
        })
    }

    pub fn memory_close(cap: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::MemoryClose(cap));
            state.memory_close.pop_front().unwrap_or(catten_syscall::memory_status::OK)
        })
    }

    pub fn dma_map_exclusive(_domain: u64, _memory: u64, _direction: DmaDirection) -> u64 {
        with_state(|state| state.dma_map)
    }

    pub fn dma_map(_domain: u64, _memory: u64, _direction: DmaDirection) -> u64 {
        with_state(|state| state.dma_map)
    }

    pub fn dma_unmap(domain: u64, iova: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::DmaUnmap(domain, iova));
            state.dma_unmap.pop_front().unwrap_or(catten_syscall::device_status::OK)
        })
    }

    pub fn device_mmio_map_any(_cap: u64, _writable: bool) -> (u64, usize) {
        with_state(|state| state.device_map)
    }

    pub fn device_mmio_unmap(cap: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::DeviceUnmap(cap));
            state.device_unmap.pop_front().unwrap_or(catten_syscall::device_status::OK)
        })
    }

    pub fn device_irq_bind_cq(_cap: u64, _cq: u32) -> u64 {
        with_state(|state| state.device_bind)
    }

    pub fn device_irq_ack(_cap: u64) -> (u64, u64) {
        with_state(|state| state.device_ack)
    }

    pub fn device_close(cap: u64) -> u64 {
        with_state(|state| state.events.push(Event::DeviceClose(cap)));
        catten_syscall::device_status::OK
    }

    pub fn submit(_op: OpCode) -> u64 {
        with_state(|state| state.submit)
    }

    pub fn submit_timer(_timeout_ms: u64) -> u64 {
        with_state(|state| state.submit)
    }

    pub unsafe fn submit_read(_buf_ptr: usize, _buf_len: usize) -> u64 {
        with_state(|state| state.submit)
    }

    pub fn poll(_cap: u64) -> (u64, u64) {
        with_state(|state| {
            state.poll.pop_front().unwrap_or((catten_syscall::completion_status::READY, 0))
        })
    }

    pub fn wait(cap: u64) {
        with_state(|state| state.events.push(Event::CompletionWait(cap)));
    }

    pub fn wait_timeout(_cap: u64, _timeout_ms: u64) -> (u64, u64) {
        with_state(|state| {
            state
                .wait_timeout
                .pop_front()
                .unwrap_or((catten_syscall::completion_status::PENDING_OR_TIMEOUT, 0))
        })
    }

    pub fn cancel(cap: u64) {
        with_state(|state| state.events.push(Event::CompletionCancel(cap)));
    }

    pub fn close(cap: u64) {
        with_state(|state| state.events.push(Event::CompletionClose(cap)));
    }

    pub fn ipc_scalar_send(_connection: u64, _opcode: u32, _arg0: u64) -> u64 {
        with_state(|state| state.ipc_send)
    }

    pub fn ipc_endpoint_create(_interface: u64, _version: u32, _capacity: usize) -> u64 {
        with_state(|state| state.ipc_endpoint)
    }

    pub fn ipc_connect(_endpoint: u64, _rights: IpcRights) -> u64 {
        with_state(|state| state.ipc_connection)
    }

    pub fn ipc_endpoint_bind_cq(_endpoint: u64, _cq: u32) -> u64 {
        with_state(|state| state.ipc_bind)
    }

    pub fn ipc_recv(_endpoint: u64) -> catten_syscall::IpcMessage {
        with_state(|state| {
            state.ipc_receive.pop_front().unwrap_or(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::NO_MESSAGE,
                opcode: 0,
                arg0: 0,
                reply: 0,
                sender: 0,
                sender_generation: 0,
                sender_principal: 0,
                sender_roles: 0,
                interface: 0,
                version: 0,
                memory: 0,
                connection: 0,
            })
        })
    }

    pub fn ipc_recv_block(endpoint: u64) -> catten_syscall::IpcMessage {
        ipc_recv(endpoint)
    }

    pub fn ipc_recv_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        ipc_recv(endpoint)
    }

    pub fn ipc_recv_block_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        ipc_recv(endpoint)
    }

    pub fn ipc_scalar_call(_connection: u64, _opcode: u32, _arg0: u64) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_send_move(_connection: u64, _opcode: u32, _arg0: u64, _memory: u64) -> u64 {
        with_state(|state| state.ipc_send_move)
    }

    pub fn ipc_scalar_call_move(_connection: u64, _opcode: u32, _arg0: u64, _memory: u64) -> u64 {
        with_state(|state| state.ipc_call_move)
    }

    pub fn ipc_scalar_call_borrow_read(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _memory: u64,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_borrow_write(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _memory: u64,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_copy(_connection: u64, _opcode: u32, _arg0: u64, _memory: u64) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_connection(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _endpoint: u64,
        _rights: IpcRights,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_connection_copy(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _endpoint: u64,
        _rights: IpcRights,
        _memory: u64,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_vector_send(_connection: u64, _opcode: u32, _arg0: u64, _descriptor: u64) -> u64 {
        with_state(|state| state.ipc_vector_send)
    }

    pub fn ipc_vector_call(_connection: u64, _opcode: u32, _arg0: u64, _descriptor: u64) -> u64 {
        with_state(|state| state.ipc_vector_call)
    }

    pub fn ipc_connection_watch_closed(_connection: u64) -> u64 {
        with_state(|state| state.connection_watch)
    }

    pub fn ipc_reply_poll_with_memory(_call: u64) -> (u64, u64, u64, u64) {
        with_state(|state| state.ipc_reply.pop_front().unwrap_or((1, 0, 0, 0)))
    }

    pub fn ipc_reply_wait_with_memory(_call: u64) -> (u64, u64, u64, u64) {
        with_state(|state| {
            state.ipc_reply.pop_front().unwrap_or((catten_syscall::ipc_status::OK, 0, 0, 0))
        })
    }

    pub fn ipc_reply(_reply: u64, _result: i64) -> u64 {
        with_state(|state| state.ipc_reply_status)
    }

    pub fn ipc_reply_move(_reply: u64, _memory: u64, _result: i64) -> u64 {
        with_state(|state| state.ipc_reply_status)
    }

    pub fn ipc_reply_connection(
        _reply: u64,
        _endpoint: u64,
        _rights: IpcRights,
        _result: i64,
    ) -> u64 {
        with_state(|state| state.ipc_reply_status)
    }

    pub fn ipc_close(cap: u64) -> u64 {
        with_state(|state| state.events.push(Event::IpcClose(cap)));
        catten_syscall::ipc_status::OK
    }

    pub fn spawn_artifact_scoped(
        _artifact: u64,
        _artifact_len: usize,
        _artifact_name: u64,
        _descriptor: u64,
        _descriptor_len: usize,
    ) -> u64 {
        with_state(|state| state.scoped_spawn)
    }

    pub fn spawn_operational_connector(_package: u64, _package_len: usize, _principal: u64) -> u64 {
        with_state(|state| state.scoped_spawn)
    }

    pub fn spawn_artifact(_artifact: u64, _artifact_len: usize, _artifact_name: u64) -> u64 {
        with_state(|state| state.scoped_spawn)
    }

    pub fn retire_artifact_named(_principal: u64) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::collections::VecDeque;

    use catten_syscall::{
        self,
        DmaDirection,
        IpcRights,
        OpCode,
    };

    use super::{
        ArtifactLaunchError,
        CapabilityVector,
        Completion,
        Connection,
        DmaDomain,
        Endpoint,
        IncomingMessage,
        MemoryError,
        MmioRegion,
        OwnedMemory,
        ReadOperation,
        kernel,
        launch_operational_connector,
        spawn_scoped_artifact,
    };

    fn setup() -> std::sync::MutexGuard<'static, ()> {
        let guard = kernel::serial();
        kernel::reset();
        guard
    }

    #[test]
    fn mapping_unmap_failure_preserves_retryable_owner() {
        let _guard = setup();
        kernel::update(|state| {
            state.memory_unmap = VecDeque::from([
                catten_syscall::memory_status::UNMAP_FAILED,
                catten_syscall::memory_status::OK,
            ]);
        });

        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let mapping = memory.map_writable().expect("memory mapping");
        let (mapping, _) = mapping.unmap().expect_err("first unmap must fail");
        let memory = mapping.unmap().expect("retry must retain the owner");
        drop(memory);

        assert_eq!(
            kernel::events(),
            [
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn dma_finish_failure_preserves_exclusive_transfer() {
        let _guard = setup();
        kernel::update(|state| {
            state.dma_unmap = VecDeque::from([
                catten_syscall::device_status::MAP_FAILED,
                catten_syscall::device_status::OK,
            ]);
        });

        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let transfer = memory.begin_dma(&domain, DmaDirection::DeviceRead).expect("DMA transfer");
        let (transfer, _) = transfer.finish().expect_err("first DMA unmap must fail");
        let memory = transfer.finish().expect("retry must retain DMA ownership");
        drop(memory);

        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn shared_dma_drop_unmaps_device_before_cpu_and_close() {
        let _guard = setup();
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let shared = memory
            .map_shared_dma(&domain, DmaDirection::Bidirectional)
            .expect("shared DMA mapping");
        drop(shared);

        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn shared_dma_mapping_failure_restores_memory_owner() {
        let _guard = setup();
        kernel::update(|state| state.dma_map = 0);
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let error = memory
            .map_shared_dma(&domain, DmaDirection::DeviceWrite)
            .expect_err("DMA mapping must fail");
        assert_eq!(error.error(), MemoryError::DmaMapFailed);
        drop(error);
        assert_eq!(
            kernel::events(),
            [kernel::Event::MemoryUnmap(10), kernel::Event::MemoryClose(10)]
        );
    }

    #[test]
    fn shared_dma_cpu_unmap_failure_does_not_repeat_dma_unmap() {
        let _guard = setup();
        kernel::update(|state| {
            state.memory_unmap = VecDeque::from([
                catten_syscall::memory_status::UNMAP_FAILED,
                catten_syscall::memory_status::OK,
            ]);
        });
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let shared = memory
            .map_shared_dma(&domain, DmaDirection::Bidirectional)
            .expect("shared DMA mapping");
        let (shared, _) = shared.finish().expect_err("first CPU unmap must fail");
        let memory = shared.finish().expect("retry must preserve ownership");
        drop(memory);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn dropped_read_cancels_waits_then_closes() {
        let _guard = setup();
        let mut bytes = [0_u8; 4];
        drop(ReadOperation::submit(&mut bytes).expect("read submission"));
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::CompletionCancel(20),
                kernel::Event::CompletionWait(20),
                kernel::Event::CompletionClose(20),
            ]
        );
    }

    #[test]
    fn completion_timeout_keeps_capability_until_terminal() {
        let _guard = setup();
        kernel::update(|state| {
            state.wait_timeout = VecDeque::from([
                (catten_syscall::completion_status::PENDING_OR_TIMEOUT, 0),
                (catten_syscall::completion_status::READY, 42),
            ]);
        });
        let mut completion = Completion::submit(OpCode::Nop).expect("completion submission");
        assert_eq!(completion.wait_timeout(1), Ok(None));
        assert_eq!(completion.wait_timeout(1), Ok(Some(42)));
        drop(completion);
        assert_eq!(kernel::events(), [kernel::Event::CompletionClose(20)]);
    }

    #[test]
    fn failed_move_returns_memory_to_the_caller() {
        let _guard = setup();
        kernel::update(|state| state.ipc_send_move = catten_syscall::ipc_status::QUEUE_FULL);
        let connection = unsafe { Connection::from_raw(8) }.expect("connection");
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let (memory, _) = connection.send_move(1, 2, memory).expect_err("move must fail");
        drop(memory);
        drop(connection);
        assert_eq!(kernel::events(), [kernel::Event::MemoryClose(10), kernel::Event::IpcClose(8)]);
    }

    #[test]
    fn scoped_launch_rejects_lengths_without_leaking_inputs() {
        let _guard = setup();
        let artifact = unsafe { OwnedMemory::from_raw(11) }.expect("artifact memory");
        let descriptor = unsafe { OwnedMemory::from_raw(12) }.expect("descriptor memory");

        assert_eq!(
            spawn_scoped_artifact(artifact, 1, 1, descriptor, 0),
            Err(ArtifactLaunchError::InvalidLength)
        );
        assert_eq!(
            kernel::events(),
            [kernel::Event::MemoryClose(12), kernel::Event::MemoryClose(11)]
        );
    }

    #[test]
    fn scoped_launch_transfers_both_inputs_on_submission() {
        let _guard = setup();
        let artifact = unsafe { OwnedMemory::from_raw(11) }.expect("artifact memory");
        let descriptor = unsafe { OwnedMemory::from_raw(12) }.expect("descriptor memory");

        assert_eq!(
            spawn_scoped_artifact(
                artifact,
                1,
                1,
                descriptor,
                charlotte_launch::deployment::HEADER_LEN,
            ),
            Ok(2)
        );
        assert!(kernel::events().is_empty());
    }

    #[test]
    fn scoped_launch_kernel_rejection_still_consumes_both_inputs() {
        let _guard = setup();
        kernel::update(|state| state.scoped_spawn = 0);
        let artifact = unsafe { OwnedMemory::from_raw(11) }.expect("artifact memory");
        let descriptor = unsafe { OwnedMemory::from_raw(12) }.expect("descriptor memory");

        assert_eq!(
            spawn_scoped_artifact(
                artifact,
                1,
                1,
                descriptor,
                charlotte_launch::deployment::HEADER_LEN,
            ),
            Err(ArtifactLaunchError::Rejected)
        );
        assert!(kernel::events().is_empty());
    }

    #[test]
    fn operational_launch_rejects_lengths_without_leaking_package() {
        let _guard = setup();
        let package = unsafe { OwnedMemory::from_raw(11) }.expect("pickup memory");

        assert!(matches!(
            launch_operational_connector(package, 1, b"kafka"),
            Err(ArtifactLaunchError::InvalidLength)
        ));
        assert_eq!(kernel::events(), [kernel::Event::MemoryClose(11)]);
    }

    #[test]
    fn operational_launch_kernel_rejection_consumes_package() {
        let _guard = setup();
        kernel::update(|state| state.scoped_spawn = 0);
        let package = unsafe { OwnedMemory::from_raw(11) }.expect("pickup memory");

        assert!(matches!(
            launch_operational_connector(
                package,
                charlotte_launch::operations_pickup::PICKUP_HEADER_LEN,
                b"kafka",
            ),
            Err(ArtifactLaunchError::Rejected)
        ));
        assert!(kernel::events().is_empty());
    }

    #[test]
    fn pending_borrow_is_revoked_when_call_is_dropped() {
        let _guard = setup();
        let connection = unsafe { Connection::from_raw(8) }.expect("connection");
        let mut memory = OwnedMemory::allocate(1).expect("memory allocation");
        let call = connection.call_borrow_write(1, 2, &mut memory).expect("borrowed call");
        drop(call);
        drop(memory);
        drop(connection);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::IpcClose(30),
                kernel::Event::MemoryClose(10),
                kernel::Event::IpcClose(8),
            ]
        );
    }

    #[test]
    fn mmio_unmap_failure_preserves_retryable_mapping() {
        let _guard = setup();
        kernel::update(|state| {
            state.device_unmap = VecDeque::from([
                catten_syscall::device_status::MAP_FAILED,
                catten_syscall::device_status::OK,
            ]);
        });
        let mmio = unsafe { MmioRegion::from_raw(50) };
        let mapping = mmio.map(true).expect("MMIO mapping");
        let (mapping, _) = mapping.unmap().expect_err("first MMIO unmap must fail");
        let mmio = mapping.unmap().expect("retry must retain MMIO ownership");
        drop(mmio);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DeviceUnmap(50),
                kernel::Event::DeviceUnmap(50),
                kernel::Event::DeviceClose(50),
            ]
        );
    }

    #[test]
    fn endpoint_and_connection_close_exactly_once() {
        let _guard = setup();
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let connection = endpoint.connect(IpcRights::CALL).expect("connection creation");
        drop(connection);
        drop(endpoint);
        assert_eq!(kernel::events(), [kernel::Event::IpcClose(41), kernel::Event::IpcClose(40)]);
    }

    #[test]
    fn incoming_message_owns_every_attachment() {
        let _guard = setup();
        kernel::update(|state| {
            state.ipc_receive.push_back(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::OK,
                opcode: 7,
                arg0: 9,
                reply: 43,
                sender: 1,
                sender_generation: 2,
                sender_principal: 3,
                sender_roles: 4,
                interface: 5,
                version: 6,
                memory: 10,
                connection: 42,
            });
        });
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let message: IncomingMessage =
            endpoint.try_receive().expect("receive status").expect("queued message");
        assert_eq!(message.opcode, 7);
        drop(message);
        drop(endpoint);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::IpcClose(43),
                kernel::Event::MemoryClose(10),
                kernel::Event::IpcClose(42),
                kernel::Event::IpcClose(40),
            ]
        );
    }

    #[test]
    fn failed_reply_move_returns_memory_owner() {
        let _guard = setup();
        kernel::update(|state| {
            state.ipc_reply_status = catten_syscall::ipc_status::QUEUE_FULL;
            state.ipc_receive.push_back(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::OK,
                opcode: 1,
                arg0: 0,
                reply: 43,
                sender: 0,
                sender_generation: 0,
                sender_principal: 0,
                sender_roles: 0,
                interface: 1,
                version: 1,
                memory: 0,
                connection: 0,
            });
        });
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let message = endpoint.try_receive().expect("receive status").expect("queued message");
        let reply = message.reply.expect("reply token");
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let (memory, _) = reply.reply_move(memory, 4).expect_err("reply must fail");
        drop(memory);
        drop(endpoint);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::IpcClose(43),
                kernel::Event::MemoryClose(10),
                kernel::Event::IpcClose(40),
            ]
        );
    }

    #[test]
    fn reply_from_connection_borrow_preserves_its_owner() {
        let _guard = setup();
        kernel::update(|state| {
            state.ipc_receive.push_back(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::OK,
                opcode: 1,
                arg0: 0,
                reply: 43,
                sender: 0,
                sender_generation: 0,
                sender_principal: 0,
                sender_roles: 0,
                interface: 1,
                version: 1,
                memory: 0,
                connection: 0,
            });
        });
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let message = endpoint.try_receive().expect("receive status").expect("queued message");
        let reply = message.reply.expect("reply token");
        let connection = unsafe { Connection::from_raw(42) }.expect("connection");
        reply
            .reply_connection_ref(connection.as_ref(), IpcRights::CALL, 7)
            .expect("reply delegation");
        drop(connection);
        drop(endpoint);
        assert_eq!(kernel::events(), [kernel::Event::IpcClose(42), kernel::Event::IpcClose(40)]);
    }

    #[test]
    fn failed_vector_call_rolls_descriptor_back_and_returns_moves() {
        let _guard = setup();
        let mut descriptor_page = [0_u8; 4096];
        kernel::update(|state| {
            state.memory_map_any =
                (catten_syscall::memory_status::OK, descriptor_page.as_mut_ptr() as usize);
            state.ipc_vector_call = 0;
        });
        let connection = unsafe { Connection::from_raw(8) }.expect("connection");
        let memory = unsafe { OwnedMemory::from_raw(11) }.expect("memory");
        let mut vector = CapabilityVector::new();
        vector.push_move(memory).expect("vector entry");
        let (vector, _) = connection.call_vector(1, 2, vector).expect_err("vector call must fail");
        drop(vector);
        drop(connection);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
                kernel::Event::MemoryClose(11),
                kernel::Event::IpcClose(8),
            ]
        );
    }
}
