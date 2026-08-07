#![no_std]

pub const CONFIG_VADDR: usize = 0x0000_0000_0001_0000;
pub const CONFIG_PAGE_SIZE: u32 = 4096;
pub const CQ_VADDR: usize = 0x0000_0000_0001_1000;
pub const CQ_ENTRIES: u32 = 32;
pub const INPUT_VADDR: usize = 0x0000_0000_0001_2000;
pub const INPUT_CAPACITY: usize = 4096;
pub const HEAP_VADDR: usize = 0x0000_0000_0030_0000;
/// Per-domain heap arena. 256 KiB: the object store's mount allocates a bitmap
/// (up to ~32 KiB) plus a directory entry vector (up to ~64 KiB for a 128 MiB
/// disk) while formatting. The arena sits well above the services' ELF load
/// segments (which start at `0x20000`) and below the status page (`0x7f0000`).
pub const HEAP_SIZE: usize = 0x40000;
/// Mutable program status/output page, deliberately separate from launch
/// configuration so applications cannot overwrite their launch contract.
// Kept below the per-shard CQ reservation at 0x0080_0000 and well above the
// linked application image, which begins at 0x0002_0000.
pub const STATUS_VADDR: usize = 0x0000_0000_007f_0000;
pub const STATUS_PAGE_SIZE: u32 = 4096;

/// Maximum ELF size accepted at the cluster-administration ingress and by the
/// kernel's deployment-agent launch gate. Keeping one shared bound prevents a
/// blessed artifact from being accepted into the store but rejected when its
/// assigned node tries to execute it.
pub const MAX_ARTIFACT_ELF_SIZE: usize = 4 * 1024 * 1024;

/// DNS service status-page ABI shared by the EL0 service and its kernel boot
/// verifier. Every field is an aligned little-endian `u32` byte offset.
pub mod sha256;

pub mod placement;
pub mod signature_note;

/// FNV-1a 64, the cluster's identity hash (node keys, artifact ids).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Tag for the cluster-wide artifact namespace in the object store.
pub const ARTIFACT_ID_TAG: u64 = 0xfffe_0000_0000_0000;

/// The stable, cluster-wide object-store id for a logical artifact name:
/// every node stores the artifact for a name at the same derived id.
pub fn artifact_object_id(name: &[u8]) -> u64 {
    ARTIFACT_ID_TAG | (fnv1a(name) & 0x0000_ffff_ffff_ffff)
}

/// The object store's packed name in the name service.
pub const OBJSTORE_NAME: u64 = 0x0000_0000_006a_626f; // "obj" packed LE.

pub mod dns_status {
    pub const STAGE: usize = 0;
    pub const PEER_COUNT: usize = 8;
    pub const TRANSPORT_COMPLETIONS: usize = 12;
    pub const IPC_REQUESTS_SERVED: usize = 16;
    pub const CURRENT_TERM: usize = 20;
    /// 1 = follower, 2 = candidate, 3 = leader.
    pub const RAFT_STATE: usize = 24;
    pub const CATALOG_ENTRIES: usize = 28;
    /// Local publication watcher: 0 = absent, 1 = armed, 2 = endpoint closed,
    /// 3 = leader unregister submitted, 4 = forwarded to leader.
    pub const PUBLICATION_LIFECYCLE: usize = 32;
    pub const REMOTE_CALL_ACKS: usize = 36;
    pub const REMOTE_CALLS_SERVED: usize = 40;
    pub const REMOTE_QUERIES_SERVED: usize = 44;
    /// Reliable-transport acknowledgements for remote query replies.
    pub const REMOTE_QUERY_REPLY_ACKS: usize = 48;
}

/// Status-page offsets written by the cluster deploy agent.
pub mod agent_status {
    /// Lifecycle stage: 4 = artifact uploaded, 6 = serving, 7 = retired.
    pub const STAGE: usize = 0;
    /// The replicated deployment generation this agent is serving.
    pub const SERVED_GENERATION: usize = 8;
}

pub const LAUNCH_HEADER_OFFSET: usize = 2112;
pub const CAPABILITY_VECTOR_OFFSET: usize = 2224;
pub const CAPABILITY_VECTOR_CAPACITY: usize = 32;
pub const LAUNCH_MAGIC: u64 = 0x4348_4152_4c4f_5454; // "CHARLOTT"
pub const LAUNCH_ABI_MAJOR: u16 = 2;
pub const LAUNCH_ABI_MINOR: u16 = 0;

pub const MANIFEST_VECTOR_OFFSET: usize = 32;
pub const MANIFEST_VECTOR_CAPACITY: usize = 32;
pub const MANIFEST_DATA_OFFSET: usize = 1024;
pub const MANIFEST_DATA_CAPACITY: usize = 1024;

/// Pack an ASCII manifest key of at most eight bytes into its stable ABI form.
pub const fn manifest_key(bytes: &[u8]) -> u64 {
    assert!(bytes.len() <= 8, "manifest keys are limited to eight bytes");
    let mut packed = [0u8; 8];
    let mut index = 0;
    while index < bytes.len() && index < packed.len() {
        packed[index] = bytes[index];
        index += 1;
    }
    u64::from_le_bytes(packed)
}

/// Well-known node-local name that the kernel registers with the name service
/// once a node has finished its boot storm. Network-initiating services
/// (cluster discovery, reliable-message/Raft membership) block on
/// `ns::OP_LOOKUP` for this name before starting to communicate, so a freshly
/// booted node never joins a cluster mid-boot. `"bootdone"` packed LE.
pub const LOCAL_READY_NAME: u64 = 0x0065_6e6f_6474_6f6f;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LaunchHeader {
    pub magic: u64,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub header_size: u16,
    pub reserved: u16,
    pub config_size: u32,
    pub flags: u32,
    pub manifest_offset: u32,
    pub manifest_count: u32,
    pub manifest_data_offset: u32,
    pub manifest_data_size: u32,
    pub capabilities_offset: u32,
    pub capabilities_count: u32,
    pub heap_base: u64,
    pub heap_size: u64,
    pub input_base: u64,
    pub input_size: u32,
    pub cq_entries: u32,
    pub cq_base: u64,
    pub status_base: u64,
    pub status_size: u32,
    pub reserved2: u32,
}

impl LaunchHeader {
    pub const fn new() -> Self {
        Self {
            magic: LAUNCH_MAGIC,
            abi_major: LAUNCH_ABI_MAJOR,
            abi_minor: LAUNCH_ABI_MINOR,
            header_size: core::mem::size_of::<Self>() as u16,
            reserved: 0,
            config_size: CONFIG_PAGE_SIZE,
            flags: 0,
            manifest_offset: MANIFEST_VECTOR_OFFSET as u32,
            manifest_count: 0,
            manifest_data_offset: MANIFEST_DATA_OFFSET as u32,
            manifest_data_size: 0,
            capabilities_offset: CAPABILITY_VECTOR_OFFSET as u32,
            capabilities_count: 0,
            heap_base: HEAP_VADDR as u64,
            heap_size: HEAP_SIZE as u64,
            input_base: INPUT_VADDR as u64,
            input_size: INPUT_CAPACITY as u32,
            cq_entries: CQ_ENTRIES,
            cq_base: CQ_VADDR as u64,
            status_base: STATUS_VADDR as u64,
            status_size: STATUS_PAGE_SIZE,
            reserved2: 0,
        }
    }

    pub const fn is_compatible(&self) -> bool {
        let manifest_end = (self.manifest_offset as usize).saturating_add(
            (self.manifest_count as usize).saturating_mul(core::mem::size_of::<ManifestRecord>()),
        );
        let manifest_data_end =
            (self.manifest_data_offset as usize).saturating_add(self.manifest_data_size as usize);
        let capabilities_end = (self.capabilities_offset as usize).saturating_add(
            (self.capabilities_count as usize)
                .saturating_mul(core::mem::size_of::<CapabilityRecord>()),
        );
        self.magic == LAUNCH_MAGIC
            && self.abi_major == LAUNCH_ABI_MAJOR
            && self.header_size as usize >= core::mem::size_of::<Self>()
            && self.config_size == CONFIG_PAGE_SIZE
            && self.manifest_offset as usize >= MANIFEST_VECTOR_OFFSET
            && self.manifest_count as usize <= MANIFEST_VECTOR_CAPACITY
            && self.manifest_data_offset as usize >= MANIFEST_DATA_OFFSET
            && self.manifest_data_size as usize <= MANIFEST_DATA_CAPACITY
            && self.capabilities_offset as usize >= CAPABILITY_VECTOR_OFFSET
            && self.capabilities_count as usize <= CAPABILITY_VECTOR_CAPACITY
            && manifest_end <= MANIFEST_DATA_OFFSET
            && manifest_data_end <= LAUNCH_HEADER_OFFSET
            && capabilities_end <= CONFIG_PAGE_SIZE as usize
            && self.heap_base != 0
            && self.heap_size != 0
            && self.input_size as usize <= INPUT_CAPACITY
            && self.cq_entries != 0
            && self.status_base != 0
            && self.status_size != 0
            && self.status_size <= STATUS_PAGE_SIZE
    }
}

impl Default for LaunchHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable identifiers for manifest value encodings.
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestValueKind {
    Unsigned = 1,
    Signed = 2,
    Bytes = 3,
}

impl ManifestValueKind {
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Unsigned),
            2 => Some(Self::Signed),
            3 => Some(Self::Bytes),
            _ => None,
        }
    }
}

/// One named launch-manifest value. Keys are packed ASCII names of at most
/// eight bytes. Byte values refer to the bounded manifest data area.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ManifestRecord {
    pub key: u64,
    pub kind: u16,
    pub flags: u16,
    pub value_len: u32,
    pub value: u64,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    Bootstrap = 1,
    Mmio = 2,
    Interrupt = 3,
    HandoffState = 4,
    HandoffEndpoint = 5,
    DmaDomain = 6,
    SystemObserver = 7,
}

impl CapabilityKind {
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Bootstrap),
            2 => Some(Self::Mmio),
            3 => Some(Self::Interrupt),
            4 => Some(Self::HandoffState),
            5 => Some(Self::HandoffEndpoint),
            6 => Some(Self::DmaDomain),
            7 => Some(Self::SystemObserver),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CapabilityRecord {
    pub kind: u16,
    pub rights: u16,
    pub flags: u32,
    pub handle: u64,
}

const _: [(); 104] = [(); core::mem::size_of::<LaunchHeader>()];
const _: [(); 24] = [(); core::mem::size_of::<ManifestRecord>()];
const _: [(); 16] = [(); core::mem::size_of::<CapabilityRecord>()];

/// The demo cluster's Ed25519 public key, injected at build time.
///
/// This is the cluster's signing authority as shipped in the kernel images:
/// nodes validate artifacts against it. The matching private key never enters
/// the cluster --- it is held off-cluster (the `tools/cluster-sign` tool) and
/// used to sign artifacts before upload. The same key can also be committed
/// to the replicated cluster state by the key ceremony (`clusterctl
/// OP_KEYCEREMONY`), which is how a node "obtains the public key from the
/// cluster" on join; the build-time copy is the bootstrap trust anchor.
pub const CLUSTER_PUBLIC_KEY: [u8; 32] = [
    0x3d, 0xdc, 0x95, 0xc2, 0x6b, 0xd5, 0xf4, 0x02, 0x2d, 0x95, 0xa4, 0xc6, 0xc8, 0xd0, 0x74, 0xf5,
    0x77, 0xf1, 0x1a, 0xf7, 0x87, 0x3e, 0x52, 0x7b, 0x01, 0x8b, 0x21, 0xbe, 0x2c, 0x03, 0x54, 0x63,
];

/// Launch-manifest key under which the kernel hands the cluster public key to
/// services (agents, clusterctl).
pub const CLUSTER_KEY_MANIFEST_KEY: u64 = manifest_key(b"ckey");
