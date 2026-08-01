//! Service domain supervision: spawn, observe exit, and tear down.
//!
//! This is deliberately mechanism-only. Naming, lookup policy, and restart
//! generations belong to the userspace name service; the supervisor's job is
//! to create protection domains, deliver exactly one bootstrap capability to
//! each (architecture doc Phase 3), and reclaim domains after they stop.
#![cfg(target_arch = "aarch64")]

#[cfg(target_arch = "aarch64")]
const ECHO_UPGRADE_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/echo.elf"));
#[cfg(target_arch = "aarch64")]
const NODE_NAME_SERVICE_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/ns.elf"));
#[cfg(target_arch = "aarch64")]
const OBSERVABILITY_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/observe.elf"));
const NODE_NAME_SERVICE_INTERFACE: u64 = u64::from_le_bytes(*b"NAME\0\0\0\0");
const NODE_NAME_SERVICE_VERSION: u32 = 1;
const NODE_NAME_SERVICE_QUEUE_CAPACITY: usize = 64;

use alloc::vec::Vec;

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        spawn_thread,
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
        AddressSpaceId,
        KERNEL_ASID,
        close_user_address_space,
        physical::PAddr,
    },
    service::{
        bootstrap,
        loader,
    },
};

/// A running EL0 service protection domain.
#[derive(Copy, Clone)]
pub struct ServiceDomain {
    pub asid: AddressSpaceId,
    pub tid: ThreadId,
    pub generation: ThreadGeneration,
    pub config_frame: PAddr,
    /// Mutable userspace status/output page observed by supervisors and tests.
    pub status_frame: PAddr,
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

/// The node-local name service shared by ordinary service domains.
///
/// Applications receive delegated connections to this registry; they do not
/// receive the endpoint capability itself.  A node has exactly one such
/// registry.  Tests or future namespace managers that deliberately need an
/// isolated registry must use [`spawn_private_name_service`].
static NODE_NAME_SERVICE: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<NameServiceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// The only domain to which the boot supervisor delegates system-wide
/// telemetry inspection authority.
static SYSTEM_OBSERVER_ASID: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<AddressSpaceId>>,
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
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<AddressSpaceId>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// Kernel-private connection to the node name service, minted when the node
/// registry starts, used by the supervisor to publish the boot-done marker.
static KERNEL_NS_CONN: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<CapabilityId>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// Interface id of the marker endpoint registered under the boot-done name.
/// The endpoint is never called; it only exists so the name service has
/// something to hand out on lookup.
const BOOT_DONE_INTERFACE: u64 = u64::from_le_bytes(*b"BOOTDONE");
/// `ns::OP_REGISTER` opcode (the name-service protocol lives in userspace).
const NS_OP_REGISTER: u32 = 1;
/// How long the boot-done publisher waits, after the boot threads are
/// admitted, for the boot storm (deferred verifiers spawning services) to
/// settle before declaring the node ready for cluster communication.
const BOOT_SETTLE_MS: u64 = 3_000;

fn start_domain(loaded: loader::LoadedDomain) -> ServiceDomain {
    let entry: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(loaded.entry_vaddr) };
    let tid = spawn_thread(loaded.asid, entry);
    let generation = MASTER_THREAD_TABLE
        .read()
        .get(tid)
        .expect("[supervisor] spawned thread missing from table")
        .generation;
    ServiceDomain {
        asid: loaded.asid,
        tid,
        generation,
        config_frame: loaded.config_frame,
        status_frame: loaded.status_frame,
    }
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
        NODE_NAME_SERVICE_ELF,
        NODE_NAME_SERVICE_INTERFACE,
        NODE_NAME_SERVICE_VERSION,
        NODE_NAME_SERVICE_QUEUE_CAPACITY,
    );
    // Retain a kernel-side connection so the supervisor can publish the
    // boot-done marker after the boot storm settles.
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

/// Spawn the thread that registers the well-known boot-done marker once the
/// boot storm has settled.
///
/// Network-initiating services (cluster discovery, reliable-message/Raft
/// membership clients) block on a name-service lookup of the marker before
/// starting to communicate, so a freshly booted node never joins a cluster
/// mid-boot.
pub fn start_boot_done_publisher() {
    spawn_thread(KERNEL_ASID, boot_done_publisher);
}

extern "C" fn boot_done_publisher() {
    // The boot storm is the burst of deferred verifiers spawning EL0 services
    // right after the scheduler starts. Yield for a bounded settling window so
    // the NIC driver, the frame demultiplexer, and the socket transport have
    // all quiesced before any node initiates cluster communication.
    let settle_until = monotonic_millis().saturating_add(BOOT_SETTLE_MS);
    while monotonic_millis() < settle_until {
        yield_lp();
    }
    publish_boot_done();
}

/// Register `charlotte_launch::BOOT_DONE_NAME` in the node name service.
///
/// The marker points at a kernel-owned endpoint that is never called; its
/// only purpose is to let a blocking `ns::OP_LOOKUP` resolve. Called from the
/// boot-done publisher thread.
pub fn publish_boot_done() {
    let endpoint = ipc::endpoint_create(KERNEL_ASID, BOOT_DONE_INTERFACE, 1, 1)
        .expect("[supervisor] boot-done endpoint creation failed");
    let conn = ipc::connection_mint(KERNEL_ASID, endpoint, ConnectionRights::ALL)
        .expect("[supervisor] boot-done connection mint failed");
    let ns_conn =
        KERNEL_NS_CONN.lock().expect("[supervisor] kernel name-service connection missing");
    let call = ipc::scalar_call_with_connection(
        KERNEL_ASID,
        ns_conn,
        NS_OP_REGISTER,
        charlotte_launch::BOOT_DONE_NAME,
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
    let loaded = loader::load_domain(image);
    let connection = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        loaded.asid,
        rights,
    )
    .expect("[supervisor] bootstrap connection delegation failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, connection);
    bootstrap::write_manifest(loaded.config_frame, &[]);
    start_domain(loaded)
}

/// Start the node observability service and delegate the unique
/// system-observer capability to it.
pub fn start_observability_service(name_service: &NameServiceHandle) -> ServiceDomain {
    let mut observer = SYSTEM_OBSERVER_ASID.lock();
    assert!(observer.is_none(), "[supervisor] system observer already started");

    let loaded = loader::load_domain(OBSERVABILITY_ELF);
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
    *observer = Some(domain.asid);
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
    *LIVE_UPGRADE_MANAGER_ASID.lock() = Some(domain.asid);
    domain
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
    !crate::cpu::scheduler::threads::DEAD_THREADS
        .read()
        .values()
        .flatten()
        .any(|thread| thread.asid == domain.asid)
}

/// Yield until the domain's initial thread exits.
///
/// Panics after `timeout_millis`, so a wedged service fails tests loudly
/// instead of hanging the boot.
pub fn wait_domain_exit(domain: &ServiceDomain, timeout_millis: u64) {
    let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(timeout_millis);
    while !domain_exited(domain) {
        assert!(
            crate::cpu::scheduler::monotonic_millis() < deadline,
            "[supervisor] domain did not exit before deadline (asid={})",
            domain.asid
        );
        yield_lp();
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
    close_user_address_space(domain.asid).expect("[supervisor] address-space close failed");
    let mut manager = LIVE_UPGRADE_MANAGER_ASID.lock();
    if *manager == Some(domain.asid) {
        *manager = None;
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
                close_user_address_space(loaded.asid)
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
                close_user_address_space(loaded.asid)
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
        0 => Some(ECHO_UPGRADE_ELF),
        _ => None,
    }
}

/// Persistent service images are not yet signed, so only accept an exact copy
/// of an image that was authenticated as part of the kernel build.
///
/// This deliberately treats the object-store checksum as corruption detection,
/// not as executable provenance. A signed service-package format can broaden
/// this policy without weakening the current bootstrap trust boundary.
pub(crate) fn persistent_elf_is_trusted(image: &[u8]) -> bool {
    image == ECHO_UPGRADE_ELF
}
