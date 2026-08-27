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
            PollingDriverGrant,
            ServiceDomain,
            ServiceLimits,
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

/// The single-node network appliance: DHCP-configured TCP/IP, UTC time, and
/// the HTTP keyhole that serves node state.
#[derive(Copy, Clone)]
pub struct NetworkAppliance {
    pub tcpip: ServiceDomain,
    pub time: ServiceDomain,
    pub httpd: ServiceDomain,
}

/// A capability profile for one S3 client-service instance. The service never
/// publishes these credentials; callers receive only its restricted endpoint.
pub struct S3Profile<'a> {
    pub endpoint_ipv4: [u8; 4],
    pub host: &'a [u8],
    pub port: u16,
    pub tls: bool,
    /// DER-encoded X.509 trust anchor. Required when `tls` is true and omitted
    /// from plaintext profiles.
    pub ca_certificate_der: Option<&'a [u8]>,
    pub region: &'a [u8],
    pub bucket: &'a [u8],
    pub prefix: &'a [u8],
    pub access_key: &'a [u8],
    pub secret_key: &'a [u8],
    pub namespace: Option<&'a [u8]>,
    pub rights: u64,
}

/// An allow-listed Kafka produce route within a profile.
pub struct KafkaProduceRoute<'a> {
    pub topic: &'a [u8],
    pub partition: u32,
}

/// An additional broker destination authorized for metadata-driven routing.
/// Kafka metadata must advertise the exact `host` and `port`; it cannot choose
/// the provisioned address.
pub struct KafkaBrokerEndpoint<'a> {
    pub endpoint_ipv4: [u8; 4],
    pub host: &'a [u8],
    pub port: u16,
}

/// A capability profile for one Kafka data-plane service. The endpoint grants
/// access only to this broker, fixed consume topic/partition, allow-listed
/// produce routes, consumer group, and transactional identity.
pub struct KafkaProfile<'a> {
    pub endpoint_ipv4: [u8; 4],
    pub host: &'a [u8],
    pub port: u16,
    pub broker_endpoints: &'a [KafkaBrokerEndpoint<'a>],
    pub tls: bool,
    /// DER-encoded X.509 trust anchor required when `tls` is set.
    pub ca_certificate_der: Option<&'a [u8]>,
    pub topic: &'a [u8],
    pub partition: u32,
    pub produce_routes: &'a [KafkaProduceRoute<'a>],
    /// Operator-selected admission limit, bounded by the implementation hard
    /// maximum. Keeping this in the signed profile lets deployments choose a
    /// lower ceiling without rebuilding the OS.
    pub max_produce_routes: u16,
    pub group: &'a [u8],
    pub transactional_id: &'a [u8],
    pub rights: u64,
    pub transaction_timeout_ms: u32,
}

/// The full steady-state service set, with each optional group present only
/// when the hardware that backs it was discovered.
#[derive(Copy, Clone)]
pub struct SteadyState {
    pub storage: Option<StorageStack>,
    pub entropy: Option<ServiceDomain>,
    pub network: Option<NetworkStack>,
    pub cluster: Option<Cluster>,
    pub appliance: Option<NetworkAppliance>,
}

/// Launch a VirtIO RNG adapter when the platform exposes one with protected
/// DMA. Architectures with RNDR/RDRAND can still serve cryptographic callers
/// through the kernel syscall when no paravirtualized device is present.
pub fn launch_entropy(ns: &NameServiceHandle) -> Option<ServiceDomain> {
    let topology = &crate::device_management::topology::DEVICE_TOPOLOGY;
    let (bar, _irq, requester_id, _) =
        crate::device_management::drivers::busses::pci_express::topology::lookup_first_virtio_rng(
            &topology.pcie,
        )?;
    if crate::device::stream_id(requester_id).is_err() {
        logln!("[launch] SKIP virtio-rng: protected DMA unavailable.");
        return None;
    }
    logln!("[launch] virtio-rng at BAR4={:#x} requester={:#x}", bar, requester_id);
    Some(crate::service::supervisor::spawn_polling_driver_with_name_service(
        crate::service::store::service_elf(b"rng").expect("[launch] rng.elf"),
        ns,
        ConnectionRights::CALL,
        PollingDriverGrant {
            mmio_phys_base: bar as usize,
            mmio_pages: 4,
            dma_requester_id: requester_id,
        },
    ))
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

/// Spawn `tcpip` in DHCP mode, the NTP-backed time service, and `httpd`.
pub fn launch_network_appliance(ns: &NameServiceHandle, persist_time: bool) -> NetworkAppliance {
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
    const NTP_IP_KEY: u64 = charlotte_launch::manifest_key(b"ntp_ip");
    const PERSIST_KEY: u64 = charlotte_launch::manifest_key(b"persist");
    let time_manifest = [
        ManifestEntry {
            key: NTP_IP_KEY,
            flags: 0,
            value: ManifestValue::Bytes(&[162, 159, 200, 1]),
        },
        ManifestEntry {
            key: PERSIST_KEY,
            flags: 0,
            value: ManifestValue::Bytes(b"1"),
        },
    ];
    let time = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"time").expect("[launch] time.elf"),
        ns,
        ConnectionRights::CALL,
        if persist_time {
            &time_manifest
        } else {
            &time_manifest[..1]
        },
    );
    NetworkAppliance {
        tcpip,
        time,
        httpd,
    }
}

/// Spawn a separately configured S3 data-plane service.
///
/// This is intentionally not part of unconditional steady-state launch:
/// credentials and bucket policy must come from the machine's trusted
/// provisioning path. Multiple instances may eventually publish distinct
/// policy-selected names; the current protocol name supports one instance.
pub fn launch_s3_profile(ns: &NameServiceHandle, profile: &S3Profile<'_>) -> ServiceDomain {
    use charlotte_protocol_s3::manifest;

    let mut entries = alloc::vec::Vec::with_capacity(12);
    entries.extend_from_slice(&[
        ManifestEntry {
            key: manifest::IP,
            flags: 0,
            value: ManifestValue::Bytes(&profile.endpoint_ipv4),
        },
        ManifestEntry {
            key: manifest::HOST,
            flags: 0,
            value: ManifestValue::Bytes(profile.host),
        },
        ManifestEntry {
            key: manifest::PORT,
            flags: 0,
            value: ManifestValue::Unsigned(profile.port as u64),
        },
        ManifestEntry {
            key: manifest::TLS,
            flags: 0,
            value: ManifestValue::Unsigned(profile.tls as u64),
        },
        ManifestEntry {
            key: manifest::REGION,
            flags: 0,
            value: ManifestValue::Bytes(profile.region),
        },
        ManifestEntry {
            key: manifest::BUCKET,
            flags: 0,
            value: ManifestValue::Bytes(profile.bucket),
        },
        ManifestEntry {
            key: manifest::PREFIX,
            flags: 0,
            value: ManifestValue::Bytes(profile.prefix),
        },
        ManifestEntry {
            key: manifest::ACCESS_KEY,
            flags: 0,
            value: ManifestValue::Bytes(profile.access_key),
        },
        ManifestEntry {
            key: manifest::SECRET_KEY,
            flags: 0,
            value: ManifestValue::Bytes(profile.secret_key),
        },
        ManifestEntry {
            key: manifest::RIGHTS,
            flags: 0,
            value: ManifestValue::Unsigned(profile.rights),
        },
    ]);
    if let Some(namespace) = profile.namespace {
        entries.push(ManifestEntry {
            key: manifest::NAMESPACE,
            flags: 0,
            value: ManifestValue::Bytes(namespace),
        });
    }
    if let Some(ca_der) = profile.ca_certificate_der {
        entries.push(ManifestEntry {
            key: manifest::CA_DER,
            flags: 0,
            value: ManifestValue::Bytes(ca_der),
        });
    }
    crate::service::supervisor::spawn_with_manifest_and_limits(
        crate::service::store::service_elf(b"s3").expect("[launch] s3.elf"),
        ns,
        ConnectionRights::CALL,
        &entries,
        // TLS certificate parsing and record processing need more than the
        // normal 16 KiB EL0 stack. Record buffers themselves live on the heap.
        ServiceLimits::default().with_user_stack_size(128 * 1024),
    )
}

/// Spawn a separately provisioned Kafka producer/consumer service.
///
/// Broker topology and authority stay behind the returned endpoint. Fetch is
/// fixed to one topic/partition; production may select only the bounded routes
/// admitted by the profile. Route selection therefore does not turn topic
/// names into ambient application authority.
pub fn launch_kafka_profile(ns: &NameServiceHandle, profile: &KafkaProfile<'_>) -> ServiceDomain {
    let routes: alloc::vec::Vec<charlotte_protocol_kafka::ProduceRoute<'_>> = profile
        .produce_routes
        .iter()
        .map(|route| charlotte_protocol_kafka::ProduceRoute {
            topic: route.topic,
            partition: i32::try_from(route.partition).expect("Kafka partition exceeds i32"),
        })
        .collect();
    let brokers: alloc::vec::Vec<charlotte_protocol_kafka::BrokerEndpoint<'_>> = profile
        .broker_endpoints
        .iter()
        .map(|broker| charlotte_protocol_kafka::BrokerEndpoint {
            endpoint_ipv4: broker.endpoint_ipv4,
            host: broker.host,
            port: broker.port,
        })
        .collect();
    let encoded = charlotte_protocol_kafka::Profile {
        endpoint_ipv4: profile.endpoint_ipv4,
        host: profile.host,
        port: profile.port,
        broker_endpoints: brokers,
        tls: profile.tls,
        ca_certificate_der: profile.ca_certificate_der.unwrap_or(&[]),
        topic: profile.topic,
        partition: i32::try_from(profile.partition).expect("Kafka partition exceeds i32"),
        produce_routes: routes,
        max_produce_routes: profile.max_produce_routes,
        group: profile.group,
        transactional_id: profile.transactional_id,
        rights: profile.rights,
        transaction_timeout_ms: profile.transaction_timeout_ms,
    }
    .encode()
    .expect("invalid Kafka profile");
    crate::service::supervisor::spawn_with_read_only_profile_and_limits(
        crate::service::store::service_elf(b"kafka").expect("[launch] kafka.elf"),
        ns,
        ConnectionRights::CALL,
        &encoded,
        ServiceLimits::default().with_user_stack_size(128 * 1024),
    )
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
    let entropy = launch_entropy(&ns);
    let network = launch_network_stack(&ns);
    let (cluster, appliance) = match network {
        Some(_) => (
            Some(launch_node_cluster(&ns, b"charlotte")),
            Some(launch_network_appliance(&ns, storage.is_some())),
        ),
        None => (None, None),
    };
    *STEADY_STATE.lock() = Some(SteadyState {
        storage,
        entropy,
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
