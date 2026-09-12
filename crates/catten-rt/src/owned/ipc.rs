//! Endpoints, connections, calls, and messages.
//!
//! Child module of [`crate::owned`].

use super::*;

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

/// Result of transferring a signed whole-node shutdown request to the
/// kernel-owned coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeShutdownRequestError {
    InvalidEnvelope,
    AlreadyInProgress,
    Denied,
}

/// Transfer the exact signed shutdown envelope into the kernel launch gate.
/// The kernel consumes the memory capability on every submitted outcome and
/// independently verifies signature, target node, and bounded duration.
pub fn request_node_shutdown(
    mut envelope: OwnedMemory,
    envelope_len: usize,
) -> Result<(), NodeShutdownRequestError> {
    if envelope_len != charlotte_launch::shutdown::ENCODED_LEN || envelope_len > envelope.len() {
        return Err(NodeShutdownRequestError::InvalidEnvelope);
    }
    let cap = envelope.cap.take().expect("shutdown envelope capability already consumed");
    match kernel::request_node_shutdown(cap, envelope_len) {
        0 => Ok(()),
        1 => Err(NodeShutdownRequestError::AlreadyInProgress),
        _ => Err(NodeShutdownRequestError::Denied),
    }
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
        || descriptor_len < charlotte_launch::deployment::MIN_HEADER_LEN
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
#[must_use = "dropping a deployed artifact requests best-effort forced retirement"]
pub struct DeployedArtifact {
    pub(super) principal: u64,
    pub(super) asid: u64,
    pub(super) retired: bool,
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
        let principal = self.principal;
        self.poll_retire_with(|| kernel::retire_artifact_named(principal))
    }

    /// Propagate an enclosing node-shutdown request to this domain.
    ///
    /// The kernel uses the earlier of the artifact's signed grace period and
    /// `deadline_ms`. Repeated calls cannot extend a retirement already in
    /// progress. This remains an explicit poll because the owner must be kept
    /// alive until kernel-side reclamation completes.
    pub fn poll_node_shutdown(&mut self, deadline_ms: u64) -> Result<bool, ArtifactLaunchError> {
        let principal = self.principal;
        self.poll_retire_with(|| kernel::retire_artifact_for_node_shutdown(principal, deadline_ms))
    }

    fn poll_retire_with(
        &mut self,
        request: impl FnOnce() -> u64,
    ) -> Result<bool, ArtifactLaunchError> {
        if self.retired {
            return Ok(true);
        }
        match request() {
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
            let _ = kernel::force_retire_artifact_named(self.principal);
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

    /// Temporarily expose the handle for a low-level service reactor that
    /// adopts every received attachment itself. The returned integer is a
    /// borrow: it must never be closed, transferred, or adopted.
    pub const fn as_raw(&self) -> u64 {
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

    /// Resize this endpoint's admission bound, preserving queued messages.
    ///
    /// The returned capacity is the platform-clamped value actually applied.
    /// A bound below the current depth rejects new sends until the queue
    /// drains; nothing is lost.
    pub fn resize(&self, capacity: usize) -> Result<usize, IpcError> {
        let applied = kernel::ipc_endpoint_resize(self.raw_handle(), capacity);
        if applied == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok(applied as usize)
    }

    /// Read `(capacity, queued depth, depth high-water)` for this endpoint.
    ///
    /// The high-water mark is the deepest the queue has ever been, which is
    /// the evidence a service uses to raise `capacity`.
    pub fn status(&self) -> Result<(usize, usize, usize), IpcError> {
        let (capacity, depth, high_water) = kernel::ipc_endpoint_status(self.raw_handle());
        if capacity == 0 {
            return Err(IpcError::CreationFailed);
        }
        Ok((capacity as usize, depth as usize, high_water as usize))
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
