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

use alloc::vec::Vec;

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
    /// The NIC driver's embedded ELF name (`b"net"` or `b"e1000e"`).
    pub driver_elf: &'static [u8],
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

/// A TCP service exposed through Charlotte's distributed L2 ingress.
/// Platform launch policy supplies this descriptor to both the frame router
/// (classification/forwarding), TCP/IP service (local VIP acceptance), and
/// DNS (placement/readiness-derived backend eligibility).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ClusterTcpService {
    pub address: [u8; 4],
    pub port: u16,
    /// Deployed artifact/service name whose exact active generation supplies
    /// eligible backends. `None` retains the platform-service compatibility
    /// mode in which every admitted, non-draining member is eligible.
    pub backend_name: Option<&'static [u8]>,
}

fn parse_cluster_tcp_service(value: &'static str) -> Option<ClusterTcpService> {
    let (backend_name, endpoint) = value
        .split_once('=')
        .map_or((None, value), |(name, endpoint)| (Some(name.as_bytes()), endpoint));
    let (address, port) = endpoint.rsplit_once(':')?;
    let address = address.as_bytes();
    let port = port.as_bytes();
    let mut octets = [0u8; 4];
    let mut octet = 0usize;
    let mut value = 0u16;
    let mut digits = 0usize;
    for byte in address.iter().copied().chain(core::iter::once(b'.')) {
        match byte {
            b'0'..=b'9' if digits < 3 => {
                value = value.checked_mul(10)?.checked_add(u16::from(byte - b'0'))?;
                digits += 1;
            }
            b'.' if digits != 0 && octet < octets.len() && value <= u16::from(u8::MAX) => {
                octets[octet] = value as u8;
                octet += 1;
                value = 0;
                digits = 0;
            }
            _ => return None,
        }
    }
    if octet != octets.len() || octets == [0; 4] {
        return None;
    }
    let port = port.iter().copied().try_fold(0u16, |value, byte| {
        byte.is_ascii_digit()
            .then_some(())
            .and_then(|()| value.checked_mul(10))
            .and_then(|value| value.checked_add(u16::from(byte - b'0')))
    })?;
    if backend_name.is_some_and(|name| !charlotte_launch::deployment::valid_artifact_name(name)) {
        return None;
    }
    (port != 0).then_some(ClusterTcpService {
        address: octets,
        port,
        backend_name,
    })
}

pub(crate) fn configured_cluster_tcp_services() -> Vec<ClusterTcpService> {
    if let Some(configured) = option_env!("CATTEN_CLUSTER_SERVICES") {
        return configured
            .split(';')
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                parse_cluster_tcp_service(entry)
                    .unwrap_or_else(|| panic!("invalid compiled cluster service {entry:?}"))
            })
            .collect();
    }
    // Parse the historical fields directly. They remain compile-time strings,
    // so the optional backend name keeps its launch-policy lifetime.
    let endpoint = option_env!("CATTEN_CLUSTER_VIP").zip(option_env!("CATTEN_CLUSTER_TCP_PORT"));
    let Some((address, port)) = endpoint else {
        return Vec::new();
    };
    let legacy = ClusterTcpService {
        address: parse_ipv4(address).unwrap_or_else(|| panic!("invalid compiled cluster VIP")),
        port: parse_port(port).unwrap_or_else(|| panic!("invalid compiled cluster TCP port")),
        backend_name: option_env!("CATTEN_CLUSTER_SERVICE_NAME").map(str::as_bytes),
    };
    alloc::vec![legacy]
}

fn parse_ipv4(address: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = address.split('.');
    for octet in &mut octets {
        *octet = parts.next()?.parse().ok()?;
    }
    (parts.next().is_none() && octets != [0; 4]).then_some(octets)
}

fn parse_port(port: &str) -> Option<u16> {
    port.parse::<u16>().ok().filter(|port| *port != 0)
}

fn encode_cluster_tcp_services(services: &[ClusterTcpService]) -> Vec<u8> {
    let bindings = services
        .iter()
        .map(|service| charlotte_launch::ingress::ServiceBinding {
            service: charlotte_launch::ingress::ServiceId::tcp_v4(service.address, service.port),
            backend_name: service.backend_name,
        })
        .collect::<Vec<_>>();
    let len = charlotte_launch::ingress::encoded_len(&bindings)
        .expect("invalid cluster ingress launch policy");
    let mut encoded = alloc::vec![0; len];
    charlotte_launch::ingress::encode(&bindings, &mut encoded)
        .expect("cluster ingress launch policy changed after validation");
    encoded
}

/// The normal deployment control plane: signed-descriptor administration,
/// the node artifact puller, and the bounded off-cluster notification ingress.
#[derive(Copy, Clone)]
pub struct DeploymentPlane {
    pub clusterctl: ServiceDomain,
    pub agent: ServiceDomain,
    pub ingress: ServiceDomain,
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

/// One application-facing Kafka capability endpoint. Its operation rights
/// are checked by the connector independently of IPC transport rights.
pub struct KafkaAuthorityEndpoint<'a> {
    pub service_name: &'a [u8],
    pub rights: u64,
}

/// Connector-only Kafka client authentication. Credentials are copied into
/// the read-only launch profile, erased from the launcher's temporary buffer,
/// and never delegated to applications.
#[derive(Clone, Copy)]
pub enum KafkaAuthentication<'a> {
    None,
    ScramSha256 {
        username: &'a [u8],
        password: &'a [u8],
    },
    MtlsP256 {
        certificate_der: &'a [u8],
        private_key_der: &'a [u8],
    },
    ScramSha256AndMtlsP256 {
        username: &'a [u8],
        password: &'a [u8],
        certificate_der: &'a [u8],
        private_key_der: &'a [u8],
    },
}

/// A capability profile for one Kafka data-plane service. Its declared access
/// points attenuate authority over this broker, fixed consume topic/partition,
/// allow-listed produce routes, consumer group, and transactional identity.
pub struct KafkaProfile<'a> {
    /// Stable connector identity used in status and logs.
    pub instance_name: &'a [u8],
    /// The only application-facing endpoints published by this connector.
    pub authority_endpoints: &'a [KafkaAuthorityEndpoint<'a>],
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
    pub authentication: KafkaAuthentication<'a>,
    pub rights: u64,
    pub transaction_timeout_ms: u32,
}

/// Orchestration policy for one generic transactional Kafka-step runner.
/// Service names are resolved only by the trusted runner; the procedure never
/// receives the connector connection.
pub struct KafkaStepProfile<'a> {
    pub procedure_name: &'a [u8],
    pub kafka_connector_name: &'a [u8],
    pub allowed_routes: &'a [u16],
    pub dlq_route: u16,
    pub max_outputs: u16,
    pub max_attempts: u16,
    pub procedure_timeout_ms: u32,
    pub retry_backoff_ms: u32,
    pub idle_poll_ms: u32,
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
    pub deployment: Option<DeploymentPlane>,
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
    launch_network_stack_with_services(ns, &[])
}

/// Spawn the physical network path, optionally enabling one cluster TCP VIP.
/// This is launch-authorized platform policy; applications receive socket
/// capabilities and never authority to alter the VIP or backend membership.
pub fn launch_network_stack_with_service(
    ns: &NameServiceHandle,
    service: Option<ClusterTcpService>,
) -> Option<NetworkStack> {
    launch_network_stack_with_services(ns, service.as_slice())
}

/// Spawn the physical network path with a bounded table of independently
/// assigned cluster service identities.
pub fn launch_network_stack_with_services(
    ns: &NameServiceHandle,
    services: &[ClusterTcpService],
) -> Option<NetworkStack> {
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
    const INGRESS_SERVICES_KEY: u64 = charlotte_launch::manifest_key(b"vips");
    let encoded_services = encode_cluster_tcp_services(services);
    let frouter_manifest = [ManifestEntry {
        key: INGRESS_SERVICES_KEY,
        flags: 0,
        value: ManifestValue::Bytes(&encoded_services),
    }];
    let frouter = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"frouter").expect("[launch] frouter.elf"),
        ns,
        ConnectionRights::CALL,
        if services.is_empty() {
            &[]
        } else {
            &frouter_manifest
        },
    );
    Some(NetworkStack {
        driver,
        frouter,
        driver_elf,
    })
}

/// Spawn this node's cluster services: `disco` (Ethernet-broadcast
/// discovery), `relmsg` (reliable messages), and `dns`.
///
/// DNS owns the node's one durable Raft member. Membership, names, deployment
/// state, and cluster events therefore share one ordered log.
pub fn launch_node_cluster(ns: &NameServiceHandle, cluster: &[u8]) -> Cluster {
    let trust = charlotte_launch::development_admission_trust(cluster)
        .expect("valid development admission trust");
    launch_node_cluster_with_trust_and_services(ns, cluster, trust, &[])
}

/// Spawn cluster services and bind distributed ingress to an optional
/// deployment-backed service identity.
pub fn launch_node_cluster_with_service(
    ns: &NameServiceHandle,
    cluster: &[u8],
    service: Option<ClusterTcpService>,
) -> Cluster {
    let trust = charlotte_launch::development_admission_trust(cluster)
        .expect("valid development admission trust");
    launch_node_cluster_with_trust_and_services(ns, cluster, trust, service.as_slice())
}

pub fn launch_node_cluster_with_services(
    ns: &NameServiceHandle,
    cluster: &[u8],
    services: &[ClusterTcpService],
) -> Cluster {
    let trust = charlotte_launch::development_admission_trust(cluster)
        .expect("valid development admission trust");
    launch_node_cluster_with_trust_and_services(ns, cluster, trust, services)
}

/// Spawn cluster services with caller-provisioned, role-separated public
/// admission trust. Production platform integration uses this entry point;
/// no private key is accepted by the launch contract.
pub fn launch_node_cluster_with_trust(
    ns: &NameServiceHandle,
    cluster: &[u8],
    trust: charlotte_launch::trust::AdmissionTrust,
) -> Cluster {
    launch_node_cluster_with_trust_and_services(ns, cluster, trust, &[])
}

/// Production cluster launch with role-separated admission trust and optional
/// service-specific ingress placement policy.
pub fn launch_node_cluster_with_trust_and_service(
    ns: &NameServiceHandle,
    cluster: &[u8],
    trust: charlotte_launch::trust::AdmissionTrust,
    service: Option<ClusterTcpService>,
) -> Cluster {
    launch_node_cluster_with_trust_and_services(ns, cluster, trust, service.as_slice())
}

/// Production cluster launch with one canonical operations-owned ingress
/// assignment table shared with DNS and the packet path.
pub fn launch_node_cluster_with_trust_and_services(
    ns: &NameServiceHandle,
    cluster: &[u8],
    trust: charlotte_launch::trust::AdmissionTrust,
    services: &[ClusterTcpService],
) -> Cluster {
    const CLUSTER_KEY: u64 = charlotte_launch::manifest_key(b"cluster");
    const ELECTION_KEY: u64 = charlotte_launch::manifest_key(b"elect-ms");
    const INGRESS_SERVICES_KEY: u64 = charlotte_launch::manifest_key(b"vips");
    assert_eq!(trust.cluster_id, charlotte_launch::trust::cluster_id(cluster).unwrap());
    let trust = trust.encode().expect("valid admission trust");

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
    let base_manifest = [
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
        ManifestEntry {
            key: charlotte_launch::ADMISSION_TRUST_MANIFEST_KEY,
            flags: 0,
            value: ManifestValue::Bytes(&trust),
        },
    ];
    let encoded_services = encode_cluster_tcp_services(services);
    let service_manifest = [
        base_manifest[0],
        base_manifest[1],
        base_manifest[2],
        ManifestEntry {
            key: INGRESS_SERVICES_KEY,
            flags: 0,
            value: ManifestValue::Bytes(&encoded_services),
        },
    ];
    let dns = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"dns").expect("[launch] dns.elf"),
        ns,
        ConnectionRights::CALL,
        if services.is_empty() {
            &base_manifest
        } else {
            &service_manifest
        },
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
    launch_network_appliance_with_services_mode(ns, persist_time, &[], true)
}

/// Spawn the IP/application-facing network services with optional local VIP
/// acceptance matching [`launch_network_stack_with_service`].
pub fn launch_network_appliance_with_service(
    ns: &NameServiceHandle,
    persist_time: bool,
    service: Option<ClusterTcpService>,
) -> NetworkAppliance {
    launch_network_appliance_with_services_mode(ns, persist_time, service.as_slice(), true)
}

pub fn launch_network_appliance_with_services(
    ns: &NameServiceHandle,
    persist_time: bool,
    services: &[ClusterTcpService],
) -> NetworkAppliance {
    launch_network_appliance_with_services_mode(ns, persist_time, services, true)
}

fn launch_network_appliance_with_services_mode(
    ns: &NameServiceHandle,
    persist_time: bool,
    services: &[ClusterTcpService],
    dhcp: bool,
) -> NetworkAppliance {
    const DHCP_KEY: u64 = charlotte_launch::manifest_key(b"dhcp");
    const INGRESS_SERVICES_KEY: u64 = charlotte_launch::manifest_key(b"vips");
    let encoded_services = encode_cluster_tcp_services(services);
    let dhcp_entry = ManifestEntry {
        key: DHCP_KEY,
        flags: 0,
        value: ManifestValue::Bytes(b"1"),
    };
    let ingress_entry = ManifestEntry {
        key: INGRESS_SERVICES_KEY,
        flags: 0,
        value: ManifestValue::Bytes(&encoded_services),
    };
    let ingress_entries = [ingress_entry];
    let dhcp_ingress_entries = [dhcp_entry, ingress_entry];
    let manifest: &[ManifestEntry<'_>] = match (dhcp, !services.is_empty()) {
        (true, true) => &dhcp_ingress_entries,
        (true, false) => core::slice::from_ref(&dhcp_entry),
        (false, true) => &ingress_entries,
        (false, false) => &[],
    };
    let tcpip = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"tcpip").expect("[launch] tcpip.elf"),
        ns,
        ConnectionRights::CALL,
        manifest,
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

/// Launch the signed deployment path once both durable local storage and the
/// network exist. The S3 connector remains separately provisioned because its
/// endpoint and credentials are machine policy, not deployment metadata.
pub fn launch_deployment_plane(ns: &NameServiceHandle, cluster: &[u8]) -> DeploymentPlane {
    let trust = charlotte_launch::development_admission_trust(cluster)
        .expect("valid development admission trust");
    // Development fixture matching `DEVELOPMENT_RECIPIENT_PUBLIC_KEY`. A
    // production platform must inject its sealed recipient key through
    // `launch_deployment_plane_with_operational_key` instead.
    const DEVELOPMENT_RECIPIENT_PRIVATE_KEY: [u8; 32] = [
        0xf0, 0x27, 0x76, 0xea, 0x15, 0x74, 0x49, 0x30, 0x94, 0xee, 0xf5, 0xb9, 0x9d, 0xb4, 0xd9,
        0x57, 0x89, 0x0d, 0x0f, 0x48, 0x3c, 0xd9, 0x2b, 0xad, 0xe2, 0x6c, 0xe3, 0xcb, 0x10, 0x7d,
        0x3b, 0x0d,
    ];
    launch_deployment_plane_with_operational_key(
        ns,
        cluster,
        trust,
        DEVELOPMENT_RECIPIENT_PRIVATE_KEY,
    )
}

/// Launch the administration and reconciliation plane with caller-provisioned
/// public trust. Artifact and deployment roles may be distinct.
pub fn launch_deployment_plane_with_trust(
    ns: &NameServiceHandle,
    cluster: &[u8],
    trust: charlotte_launch::trust::AdmissionTrust,
) -> DeploymentPlane {
    launch_deployment_plane_configured(ns, cluster, trust, None)
}

/// Launch the reconciliation plane with the cluster's HPKE recipient key held
/// only by the kernel. The key is checked against public admission trust and
/// is never copied into the agent or connector catalog.
pub fn launch_deployment_plane_with_operational_key(
    ns: &NameServiceHandle,
    cluster: &[u8],
    trust: charlotte_launch::trust::AdmissionTrust,
    recipient_private_key: [u8; 32],
) -> DeploymentPlane {
    launch_deployment_plane_configured(ns, cluster, trust, Some(recipient_private_key))
}

fn launch_deployment_plane_configured(
    ns: &NameServiceHandle,
    cluster: &[u8],
    trust: charlotte_launch::trust::AdmissionTrust,
    recipient_private_key: Option<[u8; 32]>,
) -> DeploymentPlane {
    assert_eq!(trust.cluster_id, charlotte_launch::trust::cluster_id(cluster).unwrap());
    assert!(crate::service::supervisor::configure_operational_launch_trust(
        *ns,
        trust,
        recipient_private_key,
    ));
    let admission_trust = trust.encode().expect("valid admission trust");
    let controller_trust = [ManifestEntry {
        key: charlotte_launch::ADMISSION_TRUST_MANIFEST_KEY,
        flags: 0,
        value: ManifestValue::Bytes(&admission_trust),
    }];
    let artifact_trust = [
        ManifestEntry {
            key: charlotte_launch::CLUSTER_KEY_MANIFEST_KEY,
            flags: 0,
            value: ManifestValue::Bytes(&trust.artifact_key),
        },
        ManifestEntry {
            key: charlotte_launch::ADMISSION_TRUST_MANIFEST_KEY,
            flags: 0,
            value: ManifestValue::Bytes(&admission_trust),
        },
    ];
    let clusterctl = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"clusterctl").expect("[launch] clusterctl.elf"),
        ns,
        ConnectionRights::CALL,
        &controller_trust,
    );
    let agent = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"agent").expect("[launch] agent.elf"),
        ns,
        ConnectionRights::CALL,
        &artifact_trust,
    );
    crate::service::supervisor::authorize_deployment_agent(&agent);
    let ingress = crate::service::supervisor::spawn_with_manifest(
        crate::service::store::service_elf(b"deployd").expect("[launch] deployd.elf"),
        ns,
        ConnectionRights::CALL,
        &[],
    );
    logln!(
        "[launch] deployment plane spawned: clusterctl={} agent={} ingress={} port={}",
        clusterctl.asid,
        agent.asid,
        ingress.asid,
        charlotte_launch::DEPLOY_NOTIFY_PORT
    );
    DeploymentPlane {
        clusterctl,
        agent,
        ingress,
    }
}

/// Spawn a separately configured S3 data-plane service.
///
/// This is intentionally not part of unconditional steady-state launch:
/// credentials and bucket policy must come from the machine's trusted
/// provisioning path. Multiple instances may eventually publish distinct
/// policy-selected names; the current protocol name supports one instance.
pub fn launch_s3_profile(ns: &NameServiceHandle, profile: &S3Profile<'_>) -> ServiceDomain {
    let encoded = zeroize::Zeroizing::new(
        charlotte_protocol_s3::Profile {
            endpoint_ipv4: profile.endpoint_ipv4,
            host: profile.host,
            port: profile.port,
            tls: profile.tls,
            ca_certificate_der: profile.ca_certificate_der.unwrap_or(&[]),
            region: profile.region,
            bucket: profile.bucket,
            prefix: profile.prefix,
            access_key: profile.access_key,
            secret_key: profile.secret_key,
            namespace: profile.namespace.unwrap_or(&[]),
            rights: profile.rights,
        }
        .encode()
        .expect("valid S3 launch profile"),
    );
    crate::service::supervisor::spawn_with_read_only_profile_and_limits(
        crate::service::store::service_elf(b"s3").expect("[launch] s3.elf"),
        ns,
        ConnectionRights::CALL,
        &encoded,
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
    let authority_endpoints: alloc::vec::Vec<charlotte_protocol_kafka::AuthorityEndpoint<'_>> =
        profile
            .authority_endpoints
            .iter()
            .map(|endpoint| charlotte_protocol_kafka::AuthorityEndpoint {
                service_name: endpoint.service_name,
                rights: endpoint.rights,
            })
            .collect();
    let encoded = zeroize::Zeroizing::new(
        charlotte_protocol_kafka::Profile {
            instance_name: profile.instance_name,
            authority_endpoints,
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
            authentication: match profile.authentication {
                KafkaAuthentication::None => charlotte_protocol_kafka::Authentication::None,
                KafkaAuthentication::ScramSha256 {
                    username,
                    password,
                } => charlotte_protocol_kafka::Authentication::ScramSha256 {
                    username,
                    password,
                },
                KafkaAuthentication::MtlsP256 {
                    certificate_der,
                    private_key_der,
                } => charlotte_protocol_kafka::Authentication::MtlsP256 {
                    certificate_der,
                    private_key_der,
                },
                KafkaAuthentication::ScramSha256AndMtlsP256 {
                    username,
                    password,
                    certificate_der,
                    private_key_der,
                } => charlotte_protocol_kafka::Authentication::ScramSha256AndMtlsP256 {
                    username,
                    password,
                    certificate_der,
                    private_key_der,
                },
            },
            rights: profile.rights,
            transaction_timeout_ms: profile.transaction_timeout_ms,
        }
        .encode()
        .expect("invalid Kafka profile"),
    );
    crate::service::supervisor::spawn_with_read_only_profile_and_limits(
        crate::service::store::service_elf(b"kafka").expect("[launch] kafka.elf"),
        ns,
        ConnectionRights::CALL,
        &encoded,
        ServiceLimits::default().with_user_stack_size(128 * 1024),
    )
}

/// Launch the generic Kafka transactional-step runner with a read-only,
/// digest-checked orchestration profile.
pub fn launch_kafka_step(ns: &NameServiceHandle, profile: &KafkaStepProfile<'_>) -> ServiceDomain {
    let encoded = charlotte_kafka_step::Profile {
        procedure_name: profile.procedure_name,
        kafka_connector_name: profile.kafka_connector_name,
        allowed_routes: profile.allowed_routes.to_vec(),
        dlq_route: profile.dlq_route,
        max_outputs: profile.max_outputs,
        max_attempts: profile.max_attempts,
        procedure_timeout_ms: profile.procedure_timeout_ms,
        retry_backoff_ms: profile.retry_backoff_ms,
        idle_poll_ms: profile.idle_poll_ms,
    }
    .encode()
    .expect("invalid Kafka-step profile");
    crate::service::supervisor::spawn_with_read_only_profile_and_limits(
        crate::service::store::service_elf(b"kafka_step").expect("[launch] kafka_step.elf"),
        ns,
        ConnectionRights::CALL,
        &encoded,
        ServiceLimits::default(),
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
    let cluster_services = configured_cluster_tcp_services();
    let network = launch_network_stack_with_services(&ns, &cluster_services);
    let (cluster, appliance) = match network {
        Some(_) => (
            Some(launch_node_cluster_with_services(&ns, b"charlotte", &cluster_services)),
            Some(launch_network_appliance_with_services_mode(
                &ns,
                storage.is_some(),
                &cluster_services,
                option_env!("CATTEN_CLUSTER_STATIC_NETWORK") != Some("1"),
            )),
        ),
        None => (None, None),
    };
    let deployment = if storage.is_some() && network.is_some() {
        Some(launch_deployment_plane(&ns, b"charlotte"))
    } else {
        None
    };
    *STEADY_STATE.lock() = Some(SteadyState {
        storage,
        entropy,
        network,
        cluster,
        appliance,
        deployment,
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

/// Transfer the launched service set into the node-shutdown coordinator.
/// Once taken, no observer may treat the former steady-state handles as a
/// source for new work.
pub(crate) fn take_steady_state_for_shutdown() -> Option<SteadyState> {
    STEADY_STATE.lock().take()
}

/// Derive the stable cluster node key from the trusted NIC driver's published
/// MAC before shutdown transfers the steady-state owner away. This lets the
/// kernel independently reject an otherwise valid intent for another node.
pub(crate) fn local_node_key() -> Option<u64> {
    let network = STEADY_STATE.lock().as_ref()?.network?;
    let base: *const u8 = network.driver.status_frame.into();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut any = false;
    for index in 0..6 {
        let byte = unsafe {
            core::ptr::read_volatile(base.add(charlotte_launch::net_status::MAC + index))
        };
        any |= byte != 0;
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    any.then_some(hash & 0xffff_ffff)
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
