//! Service domain supervision: spawn, observe exit, and tear down.
//!
//! This is deliberately mechanism-only. Naming, lookup policy, and restart
//! generations belong to the userspace name service; the supervisor's job is
//! to create protection domains, deliver exactly one bootstrap capability to
//! each (architecture doc Phase 3), and reclaim domains after they stop.
#![cfg(target_arch = "aarch64")]

use alloc::vec::Vec;
use crate::{
    cpu::scheduler::{
        spawn_thread,
        threads::{
            MASTER_THREAD_TABLE,
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
pub struct ServiceDomain {
    pub asid: AddressSpaceId,
    pub tid: ThreadId,
    pub config_frame: PAddr,
}

/// A running name-service domain plus the supervisor's handle to its
/// registry endpoint, used to delegate bootstrap connections to other
/// domains.
pub struct NameServiceHandle {
    pub domain: ServiceDomain,
    /// The registry endpoint capability *in the name service's table*.
    /// The supervisor created it, so it may delegate connections from it.
    pub endpoint_cap: CapabilityId,
}

fn start_domain(loaded: loader::LoadedDomain) -> ServiceDomain {
    let entry: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(loaded.entry_vaddr) };
    let tid = spawn_thread(loaded.asid, entry);
    ServiceDomain {
        asid: loaded.asid,
        tid,
        config_frame: loaded.config_frame,
    }
}

/// Load and start the name service.
///
/// The supervisor creates the registry endpoint *inside the name service's
/// address space* before it runs, and delivers the endpoint capability
/// through the bootstrap slot. This keeps bootstrap authority flowing
/// strictly downward: the name service never learns kernel identifiers, and
/// no other domain can mint registry connections.
pub fn spawn_name_service(
    image: &[u8],
    interface: u64,
    version: u32,
    capacity: usize,
) -> NameServiceHandle {
    let loaded = loader::load_domain(image);
    let endpoint_cap = ipc::endpoint_create(loaded.asid, interface, version, capacity)
        .expect("[supervisor] name-service endpoint_create failed");
    bootstrap::write_bootstrap_cap(loaded.config_frame, endpoint_cap);
    bootstrap::write_argc(loaded.config_frame, 0);
    let domain = start_domain(loaded);
    NameServiceHandle {
        domain,
        endpoint_cap,
    }
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
    bootstrap::write_argc(loaded.config_frame, 0);
    start_domain(loaded)
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
    bootstrap::write_argc(loaded.config_frame, 0);

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
    MASTER_THREAD_TABLE.read().get(domain.tid).is_err()
}

/// Spin (yielding) until the domain's initial thread exits.
///
/// Panics after `max_spins` yields, so a wedged service fails tests loudly
/// instead of hanging the boot.
pub fn wait_domain_exit(domain: &ServiceDomain, max_spins: u64) {
    let mut spins: u64 = 0;
    while !domain_exited(domain) {
        spins += 1;
        assert!(spins < max_spins, "[supervisor] domain did not exit (asid={})", domain.asid);
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
}

/// Load and start a replacement service domain, handing it the old
/// instance's state and endpoint (§live-service-upgrade design).
///
/// State memory objects owned by the supervisor (`grant.state_caps`) are
/// moved to the new domain.  The old `endpoint_cap` (if nonzero) is
/// delegated to the new domain so it can re-register under the same name.
/// The old domain must already be exited and torn down (or about to be).
pub fn spawn_upgrade(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
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
    bootstrap::write_argc(loaded.config_frame, 0);

    // Move state caps from the supervisor's ASID (0) to the new domain.
    // Any nonzero state cap goes to the new service.
    let state_count = grant.state_caps.len() as u32;
    let first_state = if state_count > 0 { grant.state_caps[0] } else { 0 };
    for cap in &grant.state_caps {
        let _ = crate::memory::object::move_to(
            crate::memory::KERNEL_ASID, *cap, loaded.asid,
        );
    }
    // Delegate the old endpoint cap (if one was handed over).
    let ep = if grant.endpoint_cap != 0 {
        // The supervisor created the endpoint in the old service's AS and
        // now delegates it to the new one.  Because the old domain's caps
        // were reclaimed on teardown, the supervisor holds the endpoint
        // through the IPC registry (it minted the endpoint during
        // spawn_name_service).  Delegation from KERNEL_ASID is the direct
        // kernel-API path.
        ipc::connection_mint(crate::memory::KERNEL_ASID, grant.endpoint_cap, ConnectionRights::ALL)
            .ok()
            .or(Some(0))
    } else {
        None
    };
    bootstrap::write_handoff_state(
        loaded.config_frame, state_count, first_state, ep.unwrap_or(0),
    );
    start_domain(loaded)
}
