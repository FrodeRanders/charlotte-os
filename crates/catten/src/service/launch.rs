//! Steady-state service composition.
//!
//! The boot path composes the node's *operational* service set here, decoupled
//! from the self-test harness. Each `launch_*` function spawns a service (or a
//! small group of interdependent services) and returns its [`ServiceDomain`]
//! handles so callers — including the optional validation layer — can observe
//! stage words and status frames without re-spawning anything.
//!
//! The services depend on one another only through the name service's deferred
//! lookups, so launch order is not a correctness requirement: a service whose
//! dependency has not registered yet simply blocks on the lookup. Spawning is
//! therefore pure launch; readiness waits and assertions belong to the test
//! (or boot-progress) layer.

use spin::LazyLock;

use crate::{
    ipc::ConnectionRights,
    logln,
    service::{
        bootstrap::{
            ManifestEntry,
            ManifestValue,
        },
        supervisor::{
            DriverGrant,
            NameServiceHandle,
            ServiceDomain,
        },
    },
};

/// The storage stack: a block driver domain plus the object store on top.
#[derive(Copy, Clone)]
pub struct StorageStack {
    pub driver: ServiceDomain,
    pub objstore: ServiceDomain,
    /// The driver's embedded ELF name (`b"nvme"`, `b"ahci"`, or `b"virtio_blk"`).
    pub driver_elf: &'static [u8],
}

/// The network stack: the NIC driver and the frame demultiplexer.
#[derive(Copy, Clone)]
pub struct NetworkStack {
    pub driver: ServiceDomain,
    pub frouter: ServiceDomain,
}

/// The node's cluster services: discovery, reliable messages, and DNS, which
/// owns the cluster's single durable Raft log.
#[derive(Copy, Clone)]
pub struct Cluster {
    pub disco: ServiceDomain,
    pub relmsg: ServiceDomain,
    pub dns: ServiceDomain,
}

/// The single-node network appliance: a DHCP-configured `tcpip` service plus
/// the `httpd` keyhole that serves node state over it.
#[derive(Copy, Clone)]
pub struct NetworkAppliance {
    pub tcpip: ServiceDomain,
    pub httpd: ServiceDomain,
}

/// The full steady-state service set, with each optional group present only
/// when the hardware that backs it was discovered.
#[derive(Copy, Clone)]
pub struct SteadyState {
    pub storage: Option<StorageStack>,
    pub network: Option<NetworkStack>,
    pub cluster: Option<Cluster>,
    pub appliance: Option<NetworkAppliance>,
}

static STEADY_STATE: LazyLock<crate::cpu::multiprocessor::spin::mutex::Mutex<Option<SteadyState>>> =
    LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

/// Spawn the block driver for the first discovered storage controller and the
/// object store on top of it.
///
/// Returns `None` when the platform cannot back a driver (no MSI mechanism or
/// no protected-DMA stream for the controller), so a boot plan can degrade to
/// a storage-less node instead of faulting.
pub fn launch_storage(ns: &NameServiceHandle) -> Option<StorageStack> {
    if !crate::device::msi_available() {
        logln!("[launch] SKIP storage: no supported MSI mechanism.");
        return None;
    }
    let (driver_elf, mmio_base, mmio_pages, intid, requester_id, msi_address) =
        discover_block_device();
    if crate::device::stream_id(requester_id).is_err() {
        logln!("[launch] SKIP storage: protected DMA unavailable.");
        return None;
    }
    let driver = crate::service::supervisor::spawn_driver_with_name_service(
        crate::service::store::service_elf(driver_elf).expect("[launch] block driver elf"),
        ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: mmio_base,
            mmio_pages,
            intid,
            dma_requester_id: Some(requester_id),
            dma_msi_address: msi_address,
        },
    );
    let objstore = crate::service::supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"objstore").expect("[launch] objstore.elf"),
        ns,
        ConnectionRights::CALL,
    );
    Some(StorageStack {
        driver,
        objstore,
        driver_elf,
    })
}

/// Spawn the NIC driver for the first discovered Ethernet controller and the
/// frame demultiplexer in front of it. Returns `None` when no NIC is present.
pub fn launch_network_stack(ns: &NameServiceHandle) -> Option<NetworkStack> {
    let (driver_elf, mmio_base, mmio_pages, intid, requester_id, msi_address) =
        discover_network_controller()?;
    let driver = crate::service::supervisor::spawn_driver_with_name_service(
        crate::service::store::service_elf(driver_elf).expect("[launch] network driver elf"),
        ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: mmio_base,
            mmio_pages,
            intid,
            dma_requester_id: Some(requester_id),
            dma_msi_address: msi_address,
        },
    );
    let frouter = crate::service::supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"frouter").expect("[launch] frouter.elf"),
        ns,
        ConnectionRights::CALL,
    );
    Some(NetworkStack {
        driver,
        frouter,
    })
}

/// Spawn this node's cluster services: `disco` (Ethernet-broadcast
/// discovery), `relmsg` (reliable messages), and `dns`.
///
/// DNS owns the node's one durable Raft member. Membership, names, deployment
/// state, and cluster events therefore share one ordered log.
pub fn launch_node_cluster(ns: &NameServiceHandle, cluster: &[u8]) -> Cluster {
    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");

    let disco = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"disco").expect("[launch] disco.elf"),
        ns,
        ConnectionRights::CALL,
        &[ManifestEntry {
            key: CLUSTER_KEY,
            flags: 0,
            value: ManifestValue::Bytes(cluster),
        }],
    );
    let relmsg = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"relmsg").expect("[launch] relmsg.elf"),
        ns,
        ConnectionRights::CALL,
        &[],
    );
    let dns = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"dns").expect("[launch] dns.elf"),
        ns,
        ConnectionRights::CALL,
        &[
            ManifestEntry {
                key: CLUSTER_KEY,
                flags: 0,
                value: ManifestValue::Bytes(cluster),
            },
            ManifestEntry {
                key: ELECTION_KEY,
                flags: 0,
                value: ManifestValue::Unsigned(2_000),
            },
        ],
    );
    logln!(
        "[launch] cluster services spawned: disco={} relmsg={} dns={} (single Raft owner)",
        disco.asid,
        relmsg.asid,
        dns.asid
    );
    Cluster {
        disco,
        relmsg,
        dns,
    }
}

/// Spawn `tcpip` in DHCP mode and `httpd` on top of it.
pub fn launch_network_appliance(ns: &NameServiceHandle) -> NetworkAppliance {
    const DHCP_KEY: u64 = charlotte_launch::manifest_key(b"dhcp");
    let tcpip = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"tcpip").expect("[launch] tcpip.elf"),
        ns,
        ConnectionRights::CALL,
        &[ManifestEntry {
            key: DHCP_KEY,
            flags: 0,
            value: ManifestValue::Bytes(b"1"),
        }],
    );
    let httpd = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"httpd").expect("[launch] httpd.elf"),
        ns,
        ConnectionRights::CALL,
        &[],
    );
    NetworkAppliance {
        tcpip,
        httpd,
    }
}

/// Launch the complete steady-state service set and publish it for observers.
///
/// Runs as a boot thread: storage launches whenever a supported controller is
/// present; the network stack launches only when a NIC is present, and the
/// cluster + appliance follow the network. The self-test suite verifies the
/// launched services instead of spawning them.
pub extern "C" fn launch_steady_state() {
    let ns = crate::service::supervisor::node_name_service();
    let storage = launch_storage(&ns);
    let network = launch_network_stack(&ns);
    let (cluster, appliance) = match network {
        Some(_) => {
            (Some(launch_node_cluster(&ns, b"charlotte")), Some(launch_network_appliance(&ns)))
        }
        None => (None, None),
    };
    *STEADY_STATE.lock() = Some(SteadyState {
        storage,
        network,
        cluster,
        appliance,
    });
    logln!("[launch] steady-state service set published.");
}

/// Read the published steady-state service set, blocking (cooperatively)
/// until the launch thread has published it.
pub fn steady_state() -> SteadyState {
    loop {
        let guard = STEADY_STATE.lock();
        if let Some(state) = guard.as_ref() {
            return *state;
        }
        drop(guard);
        crate::cpu::scheduler::yield_lp();
    }
}

/// A discovered PCI function descriptor:
/// `(driver_elf, mmio_base, mmio_pages, intid, requester_id, msi_address)`.
type DeviceDescriptor = (&'static [u8], usize, usize, u32, u32, Option<u64>);

/// Locate the first storage controller in the published PCI topology and
/// return its descriptor.
fn discover_block_device() -> DeviceDescriptor {
    #[cfg(not(feature = "hvf_compat"))]
    {
        let topo = &crate::device_management::topology::DEVICE_TOPOLOGY;
        if let Some((bar0, irq, requester_id, msi_address)) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_nvme(
                &topo.pcie,
            )
        {
            logln!("[launch] NVMe controller at BAR0={:#x} intid={}", bar0, irq);
            return (b"nvme", bar0 as usize, 2, irq, requester_id, msi_address);
        }
        if let Some((abar, irq, requester_id, msi_address)) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_virtio_blk(
                &topo.pcie,
            )
        {
            logln!("[launch] virtio-blk at BAR4={:#x} intid={}", abar, irq);
            return (b"virtio_blk", abar as usize, 4, irq, requester_id, msi_address);
        }
        if let Some((abar, irq, requester_id, msi_address)) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_ahci(
                &topo.pcie,
            )
        {
            logln!("[launch] AHCI at ABAR={:#x} intid={}", abar, irq);
            return (b"ahci", abar as usize, 2, irq, requester_id, msi_address);
        }
        panic!("[launch] no NVMe, AHCI, or virtio-blk controller in the published PCI topology");
    }
    #[cfg(feature = "hvf_compat")]
    {
        // HVF cannot safely map the QEMU ECAM window, so this development mode
        // retains the known fixed test-device placement.
        let bar0: usize = 0x1000_0000;
        let intid: u32 = 44;
        logln!("[launch] HVF fallback: BAR0={:#x} intid={}", bar0, intid);
        (b"nvme", bar0, 2, intid, 0x10, None)
    }
}

/// Locate the first Ethernet controller in the published PCI topology and
/// return its descriptor.
fn discover_network_controller() -> Option<DeviceDescriptor> {
    let topo = &crate::device_management::topology::DEVICE_TOPOLOGY;
    let found =
        crate::device_management::drivers::busses::pci_express::topology::lookup_first_virtio_net(
            &topo.pcie,
        )
        .map(|device| (&b"net"[..], device))
        .or_else(|| {
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_e1000e(
                &topo.pcie,
            )
            .map(|device| (&b"e1000e"[..], device))
        })?;
    let (driver_elf, (bar0, pages, intid, requester_id, msi_address)) = found;
    logln!(
        "[launch] {} controller at BAR0={:#x} (interrupt {}, requester {:#x})",
        core::str::from_utf8(driver_elf).unwrap_or("net"),
        bar0,
        intid,
        requester_id
    );
    Some((driver_elf, bar0 as usize & !0xfff, pages, intid, requester_id, msi_address))
}
