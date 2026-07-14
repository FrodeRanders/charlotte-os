//! Service domain supervision: spawn, observe exit, and tear down.
//!
//! This is deliberately mechanism-only. Naming, lookup policy, and restart
//! generations belong to the userspace name service; the supervisor's job is
//! to create protection domains, deliver exactly one bootstrap capability to
//! each (architecture doc Phase 3), and reclaim domains after they stop.
#![cfg(target_arch = "aarch64")]

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
