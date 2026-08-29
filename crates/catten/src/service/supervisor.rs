//! Service domain supervision: spawn, observe exit, and tear down.
//!
//! This is deliberately mechanism-only. Naming, lookup policy, and restart
//! generations belong to the userspace name service; the supervisor's job is
//! to create protection domains, deliver exactly one bootstrap capability to
//! each (architecture doc Phase 3), and reclaim domains after they stop.

const NODE_NAME_SERVICE_INTERFACE: u64 = u64::from_le_bytes(*b"NAME\0\0\0\0");
const NODE_NAME_SERVICE_VERSION: u32 = 1;
const NODE_NAME_SERVICE_QUEUE_CAPACITY: usize = 64;
const CAPABILITY_GRANT_INTERFACE: u64 = u64::from_le_bytes(*b"GRANT\0\0\0");
const CAPABILITY_GRANT_VERSION: u32 = 1;

use alloc::vec::Vec;

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        spawn_thread_on_lp,
        threads::{
            MASTER_THREAD_TABLE,
            ThreadGeneration,
            ThreadId,
        },
        yield_lp,
    },
    ipc::{
        self,
        CapabilityId,
        ConnectionRights,
    },
    memory::{
        AddressSpaceHandle,
        AddressSpaceId,
        KERNEL_ASID,
        close_user_address_space_handle,
        physical::PAddr,
    },
    service::{
        bootstrap,
        loader,
    },
};

#[derive(Debug)]
pub enum ProfileLaunchError {
    EmptyProfile,
    ProfileTooLarge,
    Load(crate::memory::AddressSpaceRegistrationError),
    BootstrapConnection(ipc::IpcError),
    ProfileAllocation(crate::memory::object::MemoryObjectError),
    ProfileTransfer(crate::memory::object::MemoryObjectError),
    RollbackMemory(crate::memory::object::MemoryObjectError),
    RollbackDomain(crate::memory::AddressSpaceCloseError),
    InvalidDeploymentDescriptor,
    DescriptorArtifactMismatch,
}

/// Own every resource acquired while preparing a profile-backed domain.
/// Nothing starts until the bootstrap connection and immutable profile have
/// both reached the new address space. Explicit failure paths call `abort`;
/// `Drop` is the best-effort fallback for a future early return.
struct ProfileLaunchTransaction {
    loaded: Option<loader::LoadedDomain>,
    kernel_profile: Option<crate::memory::object::MemoryObjectCap>,
}

impl ProfileLaunchTransaction {
    fn new(loaded: loader::LoadedDomain) -> Self {
        Self {
            loaded: Some(loaded),
            kernel_profile: None,
        }
    }

    fn loaded(&self) -> &loader::LoadedDomain {
        self.loaded.as_ref().expect("profile launch transaction already consumed")
    }

    fn abort<T>(mut self, error: ProfileLaunchError) -> Result<T, ProfileLaunchError> {
        if let Some(profile) = self.kernel_profile.take()
            && let Err(close_error) = crate::memory::object::close_cap(KERNEL_ASID, profile)
        {
            self.kernel_profile = Some(profile);
            self.rollback_best_effort();
            return Err(ProfileLaunchError::RollbackMemory(close_error));
        }
        if let Some(loaded) = self.loaded.take()
            && let Err(close_error) = close_user_address_space_handle(loaded.address_space)
        {
            self.loaded = Some(loaded);
            self.rollback_best_effort();
            return Err(ProfileLaunchError::RollbackDomain(close_error));
        }
        Err(error)
    }

    fn finish(mut self) -> loader::LoadedDomain {
        debug_assert!(self.kernel_profile.is_none());
        self.loaded.take().expect("profile launch transaction already consumed")
    }

    fn rollback_best_effort(&mut self) {
        if let Some(profile) = self.kernel_profile.take() {
            let _ = crate::memory::object::close_cap(KERNEL_ASID, profile);
        }
        if let Some(loaded) = self.loaded.take() {
            let _ = close_user_address_space_handle(loaded.address_space);
        }
    }
}

impl Drop for ProfileLaunchTransaction {
    fn drop(&mut self) {
        self.rollback_best_effort();
    }
}

/// A running EL0 service protection domain.
#[derive(Copy, Clone)]
pub struct ServiceDomain {
    pub asid: AddressSpaceId,
    /// Reuse-safe identity used by delayed authority and teardown paths.
    pub address_space: AddressSpaceHandle,
    pub tid: ThreadId,
    pub generation: ThreadGeneration,
    pub config_frame: PAddr,
    /// Mutable userspace status/output page observed by supervisors and tests.
    pub status_frame: PAddr,
}

/// Per-domain resource limits selected by the trusted launch path.
///
/// These are kernel-enforced limits, not mutable manifest data. Every thread
/// subsequently created inside the domain inherits the selected stack size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceLimits {
    pub user_stack_size: usize,
}

impl ServiceLimits {
    pub const fn with_user_stack_size(mut self, bytes: usize) -> Self {
        self.user_stack_size = bytes;
        self
    }
}

impl Default for ServiceLimits {
    fn default() -> Self {
        Self {
            user_stack_size: charlotte_launch::DEFAULT_USER_STACK_PAGES * loader::PAGE_SIZE,
        }
    }
}

/// A running name-service domain plus the supervisor's handle to its
/// registry endpoint, used to delegate bootstrap connections to other
/// domains.
#[derive(Copy, Clone)]
pub struct NameServiceHandle {
    pub domain: ServiceDomain,
    /// The registry endpoint capability *in the name service's table*.
    /// The supervisor created it, so it may delegate connections from it.
    pub endpoint_cap: CapabilityId,
}

/// Trusted capability-grant controller and the endpoint from which the
/// supervisor can mint application bootstrap connections.
#[derive(Copy, Clone)]
pub struct CapabilityGrantControllerHandle {
    pub domain: ServiceDomain,
    pub endpoint_cap: CapabilityId,
}

/// The node-local name service shared by ordinary service domains.
///
/// Applications receive delegated connections to this registry; they do not
/// receive the endpoint capability itself.  A node has exactly one such
/// registry.  Tests or future namespace managers that deliberately need an
/// isolated registry must use [`spawn_private_name_service`].
static NODE_NAME_SERVICE: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<NameServiceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

static CAPABILITY_GRANT_CONTROLLER: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<CapabilityGrantControllerHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// The only domain to which the boot supervisor delegates system-wide
/// telemetry inspection authority.
static SYSTEM_OBSERVER_ASID: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<AddressSpaceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// Name-service handle bound to the authorized live-upgrade manager.
///
/// Multiple independent registries may exist. The upgrade syscall must use
/// the manager's registry rather than whichever registry happened to be
/// spawned most recently.
pub(crate) static LIVE_UPGRADE_NS: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<NameServiceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// ASID authorized to invoke the privileged live-upgrade syscall.
///
/// This is assigned only by `spawn_service_manager`; merely registering the
/// name "svcmgr" does not grant process-creation authority.
pub(crate) static LIVE_UPGRADE_MANAGER_ASID: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<AddressSpaceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// Reuse-safe identity of the one node agent allowed to create deployed
/// protection domains. Registration under the name `agent` does not grant
/// this authority; the kernel supervisor delegates it explicitly.
pub(crate) static DEPLOYMENT_AGENT_ASID: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<AddressSpaceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// A domain owned by the deployment agent on this node. The stable principal
/// is derived from the signed artifact name; it fences retirement from ASID
/// reuse and permits several independently deployed applications to coexist.
#[derive(Copy, Clone)]
pub(crate) struct DeployedDomain {
    pub principal: u64,
    pub domain: ServiceDomain,
}

/// Domains currently owned by the deployment agent on this node.
///
/// The limit is an admission bound, not a wire-format limitation. It keeps a
/// compromised deployment agent from growing kernel bookkeeping without
/// bound while allowing a useful set of co-located application components.
pub(crate) const MAX_DEPLOYED_DOMAINS: usize = 64;
pub(crate) static DEPLOYED_DOMAINS: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Vec<DeployedDomain>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(Vec::new()));

/// Kernel-private connection to the node name service, minted when the node
/// registry starts, used by the supervisor to publish the local node ready-marker.
static KERNEL_NS_CONN: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<CapabilityId>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// Interface id of the marker endpoint registered under the local node ready name.
/// The endpoint is never called; it only exists so the name service has
/// something to hand out on lookup.
const LOCAL_READY_INTERFACE: u64 = u64::from_le_bytes(*b"BOOTDONE");
/// `ns::OP_REGISTER` opcode (the name-service protocol lives in userspace).
const NS_OP_REGISTER: u32 = 1;
/// How long the local node ready publisher waits, after the boot threads are
/// admitted, for the boot storm (deferred verifiers spawning services) to
/// settle before declaring the node ready for cluster communication.
const BOOT_SETTLE_MS: u64 = 3_000;

pub(crate) fn start_domain_with_limits(
    loaded: loader::LoadedDomain,
    limits: ServiceLimits,
) -> ServiceDomain {
    assert!(
        limits.user_stack_size != 0 && limits.user_stack_size.is_multiple_of(loader::PAGE_SIZE),
        "[supervisor] user stack limit must be a non-zero whole number of pages"
    );
    let user_stack_pages = limits.user_stack_size / loader::PAGE_SIZE;
    crate::memory::set_domain_limits(
        loaded.address_space,
        crate::memory::DomainLimits {
            user_stack_pages,
        },
    )
    .unwrap_or_else(|error| panic!("[supervisor] invalid service limits: {error:?}"));

    let entry: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(loaded.entry_vaddr) };
    // Bootstrap on the caller's active LP. Admission to an idle remote LP
    // depends on an SGI edge; if that edge is delayed or lost, the service
    // cannot register and every blocking lookup behind it deadlocks. Normal
    // scheduler policy may still migrate explicitly migration-safe work.
    let tid = spawn_thread_on_lp(loaded.asid, entry, crate::cpu::isa::lp::ops::get_lp_id());
    let generation = MASTER_THREAD_TABLE
        .read()
        .get(tid)
        .unwrap_or_else(|error| {
            panic!(
                "[supervisor] spawned thread missing from table: {error:?} (tid={tid}, asid={})",
                loaded.asid
            )
        })
        .generation;
    ServiceDomain {
        asid: loaded.asid,
        address_space: loaded.address_space,
        tid,
        generation,
        config_frame: loaded.config_frame,
        status_frame: loaded.status_frame,
    }
}

pub(crate) fn start_domain(loaded: loader::LoadedDomain) -> ServiceDomain {
    start_domain_with_limits(loaded, ServiceLimits::default())
}

/// Load and start the name service.
///
/// The supervisor creates the registry endpoint *inside the name service's
/// address space* before it runs, and delivers the endpoint capability
/// through the bootstrap slot. This keeps bootstrap authority flowing
/// strictly downward: the name service never learns kernel identifiers, and
/// no other domain can mint registry connections.
fn spawn_name_service(
    image: &[u8],
    interface: u64,
    version: u32,
    capacity: usize,
) -> NameServiceHandle {
    let loaded = loader::load_domain(image);
    let endpoint_cap = ipc::endpoint_create(loaded.asid, interface, version, capacity)
        .expect("[supervisor] name-service endpoint_create failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, endpoint_cap);
    bootstrap::write_manifest(loaded.config_frame, &[]);
    let domain = start_domain(loaded);

    NameServiceHandle {
        domain,
        endpoint_cap,
    }
}

/// Start the single node-local name service.
///
/// This is a boot operation and intentionally fails if a second node
/// registry is requested. Adding a name or another service domain must use
/// the existing handle returned by [`node_name_service`].
pub fn start_node_name_service() -> NameServiceHandle {
    let mut node = NODE_NAME_SERVICE.lock();
    assert!(node.is_none(), "[supervisor] node name service already started");
    let handle = spawn_name_service(
        crate::service::store::service_elf(b"ns").expect("[supervisor] ns service elf"),
        NODE_NAME_SERVICE_INTERFACE,
        NODE_NAME_SERVICE_VERSION,
        NODE_NAME_SERVICE_QUEUE_CAPACITY,
    );
    // Retain a kernel-side connection so the supervisor can publish the
    // local-ready marker once the node's disk stack is serving.
    let kernel_conn = ipc::connection_delegate(
        handle.domain.asid,
        handle.endpoint_cap,
        KERNEL_ASID,
        ConnectionRights::CALL,
    )
    .expect("[supervisor] kernel name-service connection delegation failed");
    *KERNEL_NS_CONN.lock() = Some(kernel_conn);
    *node = Some(handle);
    handle
}

/// Start the node's trusted capability-grant controller.
///
/// Its own endpoint is the bootstrap capability; a separately typed private
/// name-service connection lets it mediate lookups without exposing ambient
/// naming authority to applications.
pub fn start_capability_grant_controller(
    name_service: &NameServiceHandle,
) -> CapabilityGrantControllerHandle {
    let mut controller = CAPABILITY_GRANT_CONTROLLER.lock();
    assert!(controller.is_none(), "[supervisor] capability grant controller already started");
    let loaded = loader::load_domain(
        crate::service::store::service_elf(b"grantctl").expect("[supervisor] grantctl service elf"),
    );
    let endpoint_cap =
        ipc::endpoint_create(loaded.asid, CAPABILITY_GRANT_INTERFACE, CAPABILITY_GRANT_VERSION, 64)
            .expect("[supervisor] grant-controller endpoint creation failed");
    let private_name_service = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        ConnectionRights::CALL,
    )
    .expect("[supervisor] grant-controller name-service delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, endpoint_cap);
    bootstrap::write_name_service_cap(loaded.config_frame, private_name_service);
    bootstrap::write_manifest(loaded.config_frame, &[]);
    let handle = CapabilityGrantControllerHandle {
        domain: start_domain(loaded),
        endpoint_cap,
    };
    *controller = Some(handle);
    handle
}

pub fn capability_grant_controller() -> CapabilityGrantControllerHandle {
    CAPABILITY_GRANT_CONTROLLER
        .lock()
        .expect("[supervisor] capability grant controller has not been started")
}

/// Spawn the thread that registers the well-known local node ready marker once the
/// boot storm has settled.
///
/// Network-initiating services (cluster discovery, reliable-message/Raft
/// membership clients) block on a name-service lookup of the marker before
/// starting to communicate, so a freshly booted node never joins a cluster
/// mid-boot.
pub fn start_local_ready_publisher() {
    spawn_thread_on_lp(KERNEL_ASID, local_ready_publisher, crate::cpu::isa::lp::ops::get_lp_id());
}

extern "C" fn local_ready_publisher() {
    // The boot storm is the burst of deferred verifiers spawning EL0 services
    // right after the scheduler starts. Yield for a bounded settling window so
    // the NIC driver, the frame demultiplexer, and the socket transport have
    // all quiesced before any node initiates cluster communication.
    let settle_until = monotonic_millis().saturating_add(BOOT_SETTLE_MS);
    while monotonic_millis() < settle_until {
        yield_lp();
    }

    // Local business first: the marker is published only once the local disk
    // stack (NVMe driver + object store) is actually serving — the store is
    // the foundation node identity, the replicated log, and every
    // store-loaded service depend on. Cluster-facing work (discovery probes,
    // the replicated name service, membership admission) starts only after
    // this marker, so boot ordering is defined by local readiness rather
    // than by wall-clock luck. The disk stack comes up as part of the NVMe
    // deferred verifier, which passes only once the object store registers.
    assert!(
        crate::self_test::results::wait_until_resolved(
            crate::self_test::results::TestId::Nvme,
            120_000,
        ) && crate::self_test::results::has_passed(crate::self_test::results::TestId::Nvme),
        "local disk stack failed before node readiness"
    );

    #[cfg(feature = "virtio_net_test")]
    assert!(
        crate::self_test::results::wait_until_resolved(
            crate::self_test::results::TestId::Net,
            120_000,
        ) && crate::self_test::results::has_passed(crate::self_test::results::TestId::Net),
        "local network stack failed before node readiness"
    );

    publish_local_ready();
}

/// Register `charlotte_launch::LOCAL_READY_NAME` in the node name service.
///
/// The marker points at a kernel-owned endpoint that is never called; its
/// only purpose is to let a blocking `ns::OP_LOOKUP` resolve. Called from the
/// local node ready publisher thread.
pub fn publish_local_ready() {
    let endpoint = ipc::endpoint_create(KERNEL_ASID, LOCAL_READY_INTERFACE, 1, 1)
        .expect("[supervisor] boot-done endpoint creation failed");
    let conn = ipc::connection_mint(KERNEL_ASID, endpoint, ConnectionRights::ALL)
        .expect("[supervisor] boot-done connection mint failed");
    let ns_conn =
        KERNEL_NS_CONN.lock().expect("[supervisor] kernel name-service connection missing");
    let call = ipc::scalar_call_with_connection(
        KERNEL_ASID,
        ns_conn,
        NS_OP_REGISTER,
        charlotte_launch::LOCAL_READY_NAME,
        conn,
        ConnectionRights::SEND | ConnectionRights::CALL | ConnectionRights::MINT_CONNECTION,
    )
    .expect("[supervisor] boot-done registration call failed");
    ipc::wait_reply(KERNEL_ASID, call).expect("[supervisor] boot-done registration wait failed");
    let result = ipc::poll_reply(KERNEL_ASID, call)
        .expect("[supervisor] boot-done registration result missing");
    let generation = result.map(|reply| reply.result).unwrap_or(0);
    crate::logln!("[node] boot-done marker registered (generation {generation}).");
}

/// Obtain the running node-local name service.
///
/// The handle remains kernel-private. Callers use it only to delegate a
/// suitably attenuated connection into a newly loaded protection domain.
pub fn node_name_service() -> NameServiceHandle {
    NODE_NAME_SERVICE.lock().expect("[supervisor] node name service has not been started")
}

/// Start an intentionally isolated registry.
///
/// Normal node services must not use this function. It exists for namespace
/// isolation tests and, eventually, explicitly managed tenant namespaces.
pub fn spawn_private_name_service(
    image: &[u8],
    interface: u64,
    version: u32,
    capacity: usize,
) -> NameServiceHandle {
    spawn_name_service(image, interface, version, capacity)
}

/// Load and start a service or client domain, delivering a connection to
/// the name service as its bootstrap capability.
pub fn spawn_with_name_service(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
) -> ServiceDomain {
    spawn_with_manifest(image, name_service, rights, &[])
}

/// Like [`spawn_with_name_service`], but writes a typed launch manifest into
/// the domain's config page before it starts. This is the general spawn entry
/// point; the manifest-less helpers delegate to it.
pub fn spawn_with_manifest(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    manifest: &[bootstrap::ManifestEntry<'_>],
) -> ServiceDomain {
    spawn_with_manifest_and_limits(image, name_service, rights, manifest, ServiceLimits::default())
}

/// Spawn a service with an explicit kernel-enforced resource policy.
pub fn spawn_with_manifest_and_limits(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    manifest: &[bootstrap::ManifestEntry<'_>],
    limits: ServiceLimits,
) -> ServiceDomain {
    let loaded = loader::load_domain(image);
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        rights,
    )
    .expect("[supervisor] bootstrap connection delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    bootstrap::write_manifest(loaded.config_frame, manifest);
    start_domain_with_limits(loaded, limits)
}

/// Spawn a service with one immutable profile object. The memory capability
/// is transferred with `MAP_READ` only; typed launch metadata carries the
/// exact meaningful byte length rather than exposing page padding.
pub fn spawn_with_read_only_profile_and_limits(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    profile: &[u8],
    limits: ServiceLimits,
) -> ServiceDomain {
    try_spawn_with_read_only_profile_and_limits(image, name_service, rights, profile, limits)
        .unwrap_or_else(|error| panic!("[supervisor] profile-backed launch failed: {error:?}"))
}

/// Fallible, transactionally prepared variant of
/// [`spawn_with_read_only_profile_and_limits`]. A failure before the initial
/// thread starts closes the kernel-side profile object, every capability
/// already delegated to the target, and the target address space.
pub fn try_spawn_with_read_only_profile_and_limits(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    profile: &[u8],
    limits: ServiceLimits,
) -> Result<ServiceDomain, ProfileLaunchError> {
    if profile.is_empty() {
        return Err(ProfileLaunchError::EmptyProfile);
    }
    let profile_len =
        u32::try_from(profile.len()).map_err(|_| ProfileLaunchError::ProfileTooLarge)?;
    let metadata = charlotte_launch::ProfileCapabilityMetadata::new(profile_len)
        .ok_or(ProfileLaunchError::EmptyProfile)?;
    let loaded = loader::try_load_domain(image).map_err(ProfileLaunchError::Load)?;
    let mut transaction = ProfileLaunchTransaction::new(loaded);

    let connection = match ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        transaction.loaded().asid,
        rights,
    ) {
        Ok(connection) => connection,
        Err(error) => return transaction.abort(ProfileLaunchError::BootstrapConnection(error)),
    };
    let source = match crate::memory::object::allocate_with_bytes(KERNEL_ASID, profile) {
        Ok(source) => source,
        Err(error) => return transaction.abort(ProfileLaunchError::ProfileAllocation(error)),
    };
    transaction.kernel_profile = Some(source);
    let target = match crate::memory::object::move_read_only_to(
        KERNEL_ASID,
        source,
        transaction.loaded().asid,
    ) {
        Ok(target) => {
            transaction.kernel_profile = None;
            target
        }
        Err(error) => return transaction.abort(ProfileLaunchError::ProfileTransfer(error)),
    };
    bootstrap::write_bootstrap_cap(transaction.loaded().config_frame, connection);
    bootstrap::write_profile_cap(transaction.loaded().config_frame, target, metadata);
    bootstrap::write_manifest(transaction.loaded().config_frame, &[]);
    Ok(start_domain_with_limits(transaction.finish(), limits))
}

/// Launch a signed application with no name-service capability. Its bootstrap
/// connection targets the capability-grant controller and its immutable
/// profile is the signed deployment descriptor used for every acquisition.
pub fn try_spawn_with_deployment_descriptor(
    image: &[u8],
    descriptor_bytes: &[u8],
    limits: ServiceLimits,
) -> Result<ServiceDomain, ProfileLaunchError> {
    let descriptor = charlotte_launch::deployment::decode(descriptor_bytes)
        .ok_or(ProfileLaunchError::InvalidDeploymentDescriptor)?;
    if charlotte_launch::deployment::verify(descriptor_bytes, &charlotte_launch::CLUSTER_PUBLIC_KEY)
        != charlotte_launch::deployment::VerifyOutcome::Valid
    {
        return Err(ProfileLaunchError::InvalidDeploymentDescriptor);
    }
    let metadata = charlotte_launch::signature_note::artifact_metadata(image)
        .ok_or(ProfileLaunchError::DescriptorArtifactMismatch)?;
    if metadata.name() != descriptor.artifact_name
        || charlotte_launch::sha256::digest(image) != descriptor.artifact_digest
    {
        return Err(ProfileLaunchError::DescriptorArtifactMismatch);
    }
    let descriptor_len =
        u32::try_from(descriptor_bytes.len()).map_err(|_| ProfileLaunchError::ProfileTooLarge)?;
    let profile_metadata = charlotte_launch::ProfileCapabilityMetadata::new(descriptor_len)
        .ok_or(ProfileLaunchError::EmptyProfile)?;
    let loaded = loader::try_load_domain(image).map_err(ProfileLaunchError::Load)?;
    let mut transaction = ProfileLaunchTransaction::new(loaded);
    let controller = capability_grant_controller();
    let connection = match ipc::connection_delegate(
        controller.domain.asid,
        controller.endpoint_cap,
        transaction.loaded().asid,
        ConnectionRights::CALL,
    ) {
        Ok(connection) => connection,
        Err(error) => return transaction.abort(ProfileLaunchError::BootstrapConnection(error)),
    };
    let source = match crate::memory::object::allocate_with_bytes(KERNEL_ASID, descriptor_bytes) {
        Ok(source) => source,
        Err(error) => return transaction.abort(ProfileLaunchError::ProfileAllocation(error)),
    };
    transaction.kernel_profile = Some(source);
    let target = match crate::memory::object::move_read_only_to(
        KERNEL_ASID,
        source,
        transaction.loaded().asid,
    ) {
        Ok(target) => {
            transaction.kernel_profile = None;
            target
        }
        Err(error) => return transaction.abort(ProfileLaunchError::ProfileTransfer(error)),
    };
    bootstrap::write_bootstrap_cap(transaction.loaded().config_frame, connection);
    bootstrap::write_profile_cap(transaction.loaded().config_frame, target, profile_metadata);
    bootstrap::write_manifest(transaction.loaded().config_frame, &[]);
    Ok(start_domain_with_limits(transaction.finish(), limits))
}

/// Start the node observability service and delegate the unique
/// system-observer capability to it.
pub fn start_observability_service(name_service: &NameServiceHandle) -> ServiceDomain {
    let mut observer = SYSTEM_OBSERVER_ASID.lock();
    assert!(observer.is_none(), "[supervisor] system observer already started");

    let loaded = loader::load_domain(
        crate::service::store::service_elf(b"observe").expect("[supervisor] observe service elf"),
    );
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        ConnectionRights::CALL,
    )
    .expect("[supervisor] observer name-service delegation failed");
    let observer_cap =
        crate::capability::allocate(loaded.asid, crate::capability::ObjectKind::SystemObserver);
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    bootstrap::write_system_observer_cap(loaded.config_frame, observer_cap);
    bootstrap::write_manifest(loaded.config_frame, &[]);
    let domain = start_domain(loaded);
    *observer = Some(domain.address_space);
    domain
}

/// Spawn a service with a bootstrap name-service connection and one
/// kernel-provided read-only data object.
pub fn spawn_with_name_service_and_data(
    image: &[u8],
    name_service: &NameServiceHandle,
    data: &[u8],
    size_key: u64,
) -> ServiceDomain {
    let loaded = loader::load_domain(image);
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        ConnectionRights::CALL,
    )
    .expect("[supervisor] bootstrap connection delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    let source = crate::memory::object::allocate_with_bytes(crate::memory::KERNEL_ASID, data)
        .expect("[supervisor] bootstrap data allocation failed");
    let moved = crate::memory::object::move_to(crate::memory::KERNEL_ASID, source, loaded.asid)
        .expect("[supervisor] bootstrap data move failed");
    bootstrap::write_handoff_state(loaded.config_frame, 1, moved, 0);
    bootstrap::write_manifest(
        loaded.config_frame,
        &[bootstrap::ManifestEntry {
            key: size_key,
            flags: 0,
            value: bootstrap::ManifestValue::Unsigned(data.len() as u64),
        }],
    );
    start_domain(loaded)
}

/// Spawn the single userspace service manager and grant it upgrade authority.
pub fn spawn_service_manager(image: &[u8], name_service: &NameServiceHandle) -> ServiceDomain {
    let domain = spawn_with_name_service(image, name_service, ConnectionRights::CALL);
    *LIVE_UPGRADE_NS.lock() = Some(*name_service);
    *LIVE_UPGRADE_MANAGER_ASID.lock() = Some(domain.address_space);
    domain
}

/// Delegate node deployment authority to an already-created agent domain.
/// Kept separate from ordinary spawning because the cluster test supplies a
/// manifest before the agent starts.
pub fn authorize_deployment_agent(domain: &ServiceDomain) {
    let mut authorized = DEPLOYMENT_AGENT_ASID.lock();
    assert!(authorized.is_none(), "[supervisor] deployment agent already authorized");
    *authorized = Some(domain.address_space);
}

/// The device authority a driver manager grants to a driver protection
/// domain (architecture doc §10.1). Deliberately narrow: exactly the MMIO
/// window and interrupt the driver needs, nothing more.
pub struct DriverGrant {
    /// Physical base of the device register window (page-aligned).
    pub mmio_phys_base: usize,
    /// Number of pages in the register window.
    pub mmio_pages: usize,
    /// The device interrupt id (a GIC SPI, INTID >= 32).
    pub intid: u32,
    /// PCI requester ID (bus << 8 | device << 3 | function) for a DMA-capable
    /// device. When present, the supervisor creates a private SMMU domain.
    pub dma_requester_id: Option<u32>,
    /// MSI/MSI-X doorbell address the requester is allowed to write.
    pub dma_msi_address: Option<u64>,
}

/// Device authority for a polling driver that needs MMIO and protected DMA
/// but deliberately has no interrupt capability.
pub struct PollingDriverGrant {
    pub mmio_phys_base: usize,
    pub mmio_pages: usize,
    pub dma_requester_id: u32,
}

/// The state and authority a supervisor passes from an old service instance
/// to its replacement during a live upgrade (live-service-upgrade design
/// doc). The old service serialised its state into memory objects and
/// handed its endpoint to the supervisor; the supervisor delivers both to
/// the new domain via the config-page contract.
pub struct UpgradeGrant {
    /// Memory objects the old service moved to the supervisor's AS,
    /// holding the serialised state the new service should resume from.
    pub state_caps: Vec<crate::memory::object::MemoryObjectCap>,
    /// The old service's endpoint capability, so the new service can
    /// re-register it under the same name. 0 if the new service should
    /// create its own endpoint.
    pub endpoint_cap: CapabilityId,
}

/// Load and start a userspace driver domain (architecture doc Phase 8).
///
/// Like [`spawn_with_name_service`] the driver receives a bootstrap
/// connection to the name service, but it additionally receives delegated
/// device capabilities — an MMIO region and an interrupt — minted kernel-side
/// and delivered through the config-page contract. The driver never names a
/// physical address or interrupt vector; it only maps and binds the
/// capabilities it is handed.
pub fn spawn_driver_with_name_service(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    grant: DriverGrant,
) -> ServiceDomain {
    let loaded = loader::load_domain(image);
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        rights,
    )
    .expect("[supervisor] driver bootstrap connection delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    bootstrap::write_manifest(loaded.config_frame, &[]);

    let mmio = crate::device::grant_mmio(loaded.asid, grant.mmio_phys_base, grant.mmio_pages)
        .unwrap_or_else(|error| {
            panic!(
                "[supervisor] MMIO region grant failed: {:?} (owner={}, base={:#x}, pages={})",
                error, loaded.asid, grant.mmio_phys_base, grant.mmio_pages
            )
        });
    let irq = crate::device::grant_interrupt(loaded.asid, grant.intid)
        .expect("[supervisor] interrupt grant failed");
    bootstrap::write_mmio_cap(loaded.config_frame, mmio);
    bootstrap::write_irq_cap(loaded.config_frame, irq);
    if let Some(requester_id) = grant.dma_requester_id {
        let dma = crate::device::grant_dma_domain(loaded.asid, requester_id, grant.dma_msi_address)
            .expect("[supervisor] DMA-domain grant failed");
        bootstrap::write_dma_domain_cap(loaded.config_frame, dma);
    }
    start_domain(loaded)
}

/// Launch a polling device adapter with only MMIO and DMA authority.
pub fn spawn_polling_driver_with_name_service(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    grant: PollingDriverGrant,
) -> ServiceDomain {
    let loaded = loader::load_domain(image);
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        rights,
    )
    .expect("[supervisor] polling-driver bootstrap delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    bootstrap::write_manifest(loaded.config_frame, &[]);
    let mmio = crate::device::grant_mmio(loaded.asid, grant.mmio_phys_base, grant.mmio_pages)
        .expect("[supervisor] polling-driver MMIO grant failed");
    bootstrap::write_mmio_cap(loaded.config_frame, mmio);
    let dma = crate::device::grant_dma_domain(loaded.asid, grant.dma_requester_id, None)
        .expect("[supervisor] polling-driver DMA-domain grant failed");
    bootstrap::write_dma_domain_cap(loaded.config_frame, dma);
    start_domain(loaded)
}

/// Returns true once the domain's initial thread has exited and been reaped
/// from the master thread table.
pub fn domain_exited(domain: &ServiceDomain) -> bool {
    // A service may create additional threads after its initial entry thread.
    // Removing only that initial TID is not sufficient evidence that the
    // address space is quiescent: tearing it down while another domain thread
    // is in EL0 (or entering SVC) leaves TTBR0 naming an already-removed AS.
    if MASTER_THREAD_TABLE.read().iter().flatten().any(|thread| thread.asid == domain.asid) {
        return false;
    }
    // A retiring thread is moved between two independently locked tables.
    // Treat the system-wide transition interval conservatively so this
    // observer cannot mistake the remove-before-stage gap for completed
    // domain reaping.
    if crate::cpu::scheduler::threads::retirement_in_flight() {
        return false;
    }
    !crate::cpu::scheduler::threads::DEAD_THREADS
        .read()
        .values()
        .flatten()
        .any(|thread| thread.asid == domain.asid)
}

/// Wait until the domain's threads have all exited.
///
/// The initial thread's exit is observed as a completion event; the domain is
/// considered exited only when the whole address space has drained (sub-
/// threads included), which is re-checked on each wake. Panics after
/// `timeout_millis`, so a wedged service fails tests loudly instead of
/// hanging the boot.
pub fn wait_domain_exit(domain: &ServiceDomain, timeout_millis: u64) {
    let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(timeout_millis);
    // The observer is generation-bound: thread IDs are recycled, so a
    // replacement service can already occupy this domain's tid slot by the
    // time the upgrade completes. Registering on the new occupant would wait
    // for an exit that never comes; the generation check completes the
    // capability immediately instead when the observed thread is gone.
    let exit = crate::completion::observe_thread_exit_with_generation(
        domain.asid,
        domain.tid,
        Some(domain.generation),
    )
    .expect("[supervisor] domain exit observer");
    let remaining = deadline.saturating_sub(crate::cpu::scheduler::monotonic_millis());
    let exited = crate::completion::wait_timeout(domain.asid, exit, remaining)
        .expect("[supervisor] domain exit wait error");
    assert!(exited, "[supervisor] domain did not exit before deadline (asid={})", domain.asid);
    // The initial thread's exit is the wake event; sub-threads may still be
    // draining. A short bounded settle keeps teardown safe without polling
    // for the common single-thread case above.
    while !domain_exited(domain) {
        assert!(
            crate::cpu::scheduler::monotonic_millis() < deadline,
            "[supervisor] domain did not fully exit before deadline (asid={})",
            domain.asid
        );
        crate::cpu::scheduler::sleep_millis(10);
    }
}

/// Tear down an exited domain: close its kernel-side resources (IPC caps,
/// endpoints, memory objects, completion state) and free the address space.
///
/// Closing the domain's endpoints is what makes stale client connections
/// fail deterministically with `EndpointClosed` after a restart.
pub fn teardown_domain(domain: ServiceDomain) {
    assert!(
        domain_exited(&domain),
        "[supervisor] refusing to tear down a domain whose thread still runs"
    );
    close_user_address_space_handle(domain.address_space)
        .expect("[supervisor] address-space close failed");
    let mut manager = LIVE_UPGRADE_MANAGER_ASID.lock();
    if *manager == Some(domain.address_space) {
        *manager = None;
    }
    let mut agent = DEPLOYMENT_AGENT_ASID.lock();
    if *agent == Some(domain.address_space) {
        *agent = None;
    }
}

/// Load and start a replacement service domain, handing it the old
/// instance's state and endpoint (§live-service-upgrade design).
///
/// State memory objects are moved from the supervisor (via KERNEL\_ASID) to
/// the new domain.  `old_asid` is the old service's address space, still
/// live at this point; `old_endpoint_cap` is its endpoint cap.  The
/// supervisor delegates a connection from that endpoint to the new domain
/// before the old domain is torn down, and writes both the state caps and
/// the old endpoint cap to the config page so the replacement service can
/// inspect them.
pub fn spawn_upgrade(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    old_asid: AddressSpaceId,
    grant: UpgradeGrant,
) -> ServiceDomain {
    let handoff_caps = grant.state_caps.len() + usize::from(grant.endpoint_cap != 0);
    assert!(
        handoff_caps < charlotte_launch::CAPABILITY_VECTOR_CAPACITY,
        "[supervisor] upgrade handoff exceeds launch capability vector"
    );
    let loaded = loader::load_domain(image);
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        rights,
    )
    .expect("[supervisor] upgrade bootstrap connection delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    bootstrap::write_manifest(loaded.config_frame, &[]);

    // Move state caps from KERNEL_ASID to the new domain.
    let mut moved_state = Vec::with_capacity(grant.state_caps.len());
    for &source_cap in &grant.state_caps {
        match crate::memory::object::move_to(crate::memory::KERNEL_ASID, source_cap, loaded.asid) {
            Ok(target_cap) => moved_state.push((source_cap, target_cap)),
            Err(error) => {
                for &(original_cap, target_cap) in moved_state.iter().rev() {
                    crate::memory::object::rollback_move_to(
                        loaded.asid,
                        target_cap,
                        crate::memory::KERNEL_ASID,
                        original_cap,
                    )
                    .expect("[supervisor] upgrade state rollback failed");
                }
                close_user_address_space_handle(loaded.address_space)
                    .expect("[supervisor] failed upgrade-domain cleanup");
                panic!("[supervisor] upgrade state move failed: {error:?}");
            }
        }
    }
    // Delegate a connection from the old endpoint to the new domain while
    // the old domain is still alive.
    let delegated_ep = if grant.endpoint_cap != 0 {
        Some(
            ipc::connection_delegate(
                old_asid,
                grant.endpoint_cap,
                loaded.asid,
                ConnectionRights::SEND | ConnectionRights::CALL,
            )
            .unwrap_or_else(|error| {
                for &(original_cap, target_cap) in moved_state.iter().rev() {
                    crate::memory::object::rollback_move_to(
                        loaded.asid,
                        target_cap,
                        crate::memory::KERNEL_ASID,
                        original_cap,
                    )
                    .expect("[supervisor] upgrade state rollback failed");
                }
                close_user_address_space_handle(loaded.address_space)
                    .expect("[supervisor] failed upgrade-domain cleanup");
                panic!("[supervisor] upgrade endpoint delegation failed: {error:?}");
            }),
        )
    } else {
        None
    };
    let target_state_caps =
        moved_state.iter().map(|(_, target_cap)| *target_cap).collect::<Vec<_>>();
    bootstrap::write_handoff_states(
        loaded.config_frame,
        &target_state_caps,
        delegated_ep.unwrap_or(0),
    );
    start_domain(loaded)
}

/// Return the embedded ELF image for a given upgrade selector.
/// 0 = echo service.  Returns None for unknown selectors.
pub fn elf_for_selector(selector: u64) -> Option<&'static [u8]> {
    match selector {
        0 => Some(
            crate::service::store::service_elf(b"echo").expect("[supervisor] echo service elf"),
        ),
        _ => None,
    }
}

/// Persistent service images are trusted by their cluster signature: the EL0
/// loader refuses anything that is not validly signed, so provenance here is
/// the signature note, not a byte-for-byte match against a kernel-embedded
/// copy (the old hash-equality constraint, which the signing work removed).
pub(crate) fn persistent_elf_is_trusted(image: &[u8]) -> bool {
    charlotte_launch::signature_note::verify_elf(image, &charlotte_launch::CLUSTER_PUBLIC_KEY)
        == charlotte_launch::signature_note::VerifyOutcome::Valid
}
