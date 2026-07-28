//! Service domain supervision: spawn, observe exit, and tear down.
//!
//! This is deliberately mechanism-only. Naming, lookup policy, and restart
//! generations belong to the userspace name service; the supervisor's job is
//! to create protection domains, deliver exactly one bootstrap capability to
//! each (architecture doc Phase 3), and reclaim domains after they stop.
#![cfg(target_arch = "aarch64")]

#[cfg(target_arch = "aarch64")]
const ECHO_UPGRADE_ELF: &[u8] = include_bytes!("../self_test/echo.elf");
#[cfg(target_arch = "aarch64")]
const NODE_NAME_SERVICE_ELF: &[u8] = include_bytes!("../self_test/ns.elf");
const NODE_NAME_SERVICE_INTERFACE: u64 = u64::from_le_bytes(*b"NAME\0\0\0\0");
const NODE_NAME_SERVICE_VERSION: u32 = 1;
const NODE_NAME_SERVICE_QUEUE_CAPACITY: usize = 64;

use alloc::vec::Vec;

use crate::{
    cpu::scheduler::{
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
    *node = Some(handle);
    handle
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
        .expect("[supervisor] MMIO region grant failed");
    let irq = crate::device::grant_interrupt(loaded.asid, grant.intid)
        .expect("[supervisor] interrupt grant failed");
    bootstrap::write_mmio_cap(loaded.config_frame, mmio);
    bootstrap::write_irq_cap(loaded.config_frame, irq);
    start_domain(loaded)
}

/// Returns true once the domain's initial thread has exited and been reaped
/// from the master thread table.
pub fn domain_exited(domain: &ServiceDomain) -> bool {
    if let Ok(thread) = MASTER_THREAD_TABLE.read().get(domain.tid)
        && thread.generation == domain.generation
    {
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
    let state_count = grant.state_caps.len() as u32;
    let first_state = if state_count > 0 {
        grant.state_caps[0]
    } else {
        0
    };
    for cap in &grant.state_caps {
        let _ = crate::memory::object::move_to(crate::memory::KERNEL_ASID, *cap, loaded.asid);
    }
    // Delegate a connection from the old endpoint to the new domain while
    // the old domain is still alive.
    let delegated_ep = if grant.endpoint_cap != 0 {
        ipc::connection_delegate(
            old_asid,
            grant.endpoint_cap,
            loaded.asid,
            ConnectionRights::SEND | ConnectionRights::CALL,
        )
        .ok()
    } else {
        None
    };
    bootstrap::write_handoff_state(
        loaded.config_frame,
        state_count,
        first_state,
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
