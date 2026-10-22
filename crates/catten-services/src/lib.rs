//! Shared protocol definitions for the reference CharlotteOS services.
//!
//! This is the userspace half of the Phase 3 name-service architecture: the
//! kernel moves opaque capabilities, while interface ids, opcodes, names,
//! generations, and lookup policy are defined here.
#![no_std]

extern crate alloc;

pub mod local_invocation;
pub mod publication_generation;
pub use charlotte_protocol_dns as dns_call;
pub use charlotte_protocol_ns as ns_generation;
pub mod network_random;
pub mod socket_lifetime;
pub mod socket_state;

/// Authorization policy state machine shared by a co-located name/policy
/// service and a possible future standalone policy service.
pub use charlotte_authorization as authorization;
/// Transactional-step profile and procedure ABI.
pub use charlotte_kafka_step as kafka_step;
/// Capability-oriented Kafka client-service protocol.
pub use charlotte_protocol_kafka as kafka;
/// Capability-oriented S3 data-plane service protocol.
pub use charlotte_protocol_s3 as s3;
/// UTC time-service protocol shared with applications.
pub use charlotte_protocol_time as time;

pub mod clusterctl_client;
pub mod deadline;
pub mod deployment_control;
pub mod deployment_http;
pub mod deployment_input;
/// Owned application-side wrapper for the capability-grant controller.
pub mod grant_client;
/// Owned application-side wrappers for the Kafka service protocol.
pub mod kafka_client;
pub mod publication;
pub mod raft_clock;
pub mod reactor;
/// Owned application-side wrappers for the S3 service protocol.
pub mod s3_client;
/// Verified, owned TLS client transport shared by network services.
pub mod tls_client;

/// Disk-backed Raft persistent state and log store on top of the object store.
pub mod disk_raft;

mod objstore_client;
pub mod objstore_mount;

/// Persistent, cluster-scoped node identity (`{mnemonic}:{token}`).
pub mod node_identity;

pub mod broker;
/// Deterministic service/flow selection and packet helpers for distributed
/// L2 ingress.
pub mod cluster_ingress;
/// Bounded, versioned cluster-observability snapshot shared by DNS and HTTP.
pub mod cluster_observe;
/// Complete, bounded DNS catalog observations shared by DNS and HTTP.
pub mod dns_catalog;
/// Replicated name catalog: the Raft state machine for the distributed name
/// service.
pub mod name_catalog;
pub mod ns_publication;
pub mod operations_admission;
pub mod release_admission;

/// Raft peer transport over the reliable message layer (`relmsg`).
pub mod relmsg_transport;

/// Pack up to 8 ASCII bytes into a u64 service name (little-endian).
///
/// This interim scalar encoding is limited to 8 bytes; longer names travel
/// in a copied memory object (see [`ns::OP_REGISTER_NAMED`] and
/// [`ns::OP_LOOKUP_NAMED`]).
/// The registry name of a node's Raft administration endpoint: FNV-1a over
/// `raft-{id}`. Operationally this endpoint is owned by DNS and controls the
/// same node and log as the distributed catalog.
/// Unlike [`name`] this is not truncated, so identity-derived node ids
/// (e.g. `cluster:abcd1234`) cannot collide in the name service.
pub fn raft_name(id: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in b"raft-".iter().copied().chain(id.iter().copied()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub const fn name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

/// The scratch virtual address services use to stage a memory-carried name.
///
/// Chosen above the program image (linked at `0x20000`, well under 1 MiB) and
/// away from the fixed runtime pages: config (`0x10000`), CQ ring
/// (`0x11000`), launch input (`0x12000`), and heap (`0x300000..0x700000`).
pub const NAME_SCRATCH_VADDR: usize = 0x0000_0000_0010_0000;

/// Maximum memory-carried name length (fits one page with room to spare).
pub const MAX_NAME_LEN: usize = 256;

/// Capability-grant controller protocol. Applications submit their immutable,
/// signed deployment descriptor and request one named service. The controller
/// authenticates the descriptor and the kernel-provided caller identity, then
/// returns only the requested attenuated connection.
pub mod grant {
    pub const INTERFACE: u64 = super::name(b"GRANT");
    pub const VERSION: u32 = 1;
    pub const OP_ACQUIRE: u32 = 1;
    /// Publish an application-owned endpoint under an exact descriptor grant.
    pub const OP_PUBLISH: u32 = 2;

    pub const ERR_INVALID: i64 = -1;
    pub const ERR_UNAUTHORIZED: i64 = -2;
    pub const ERR_UNAVAILABLE: i64 = -3;

    pub const REQUEST_MAGIC: u32 = 0x3151_5247; // "GRQ1"
    pub const REQUEST_HEADER_LEN: usize = 16;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AcquireRequest<'a> {
        pub service: &'a [u8],
        pub rights: u16,
        pub descriptor: &'a [u8],
    }

    pub fn encode_request(
        service: &[u8],
        rights: u16,
        descriptor: &[u8],
        output: &mut [u8],
    ) -> Option<usize> {
        if service.is_empty()
            || service.len() > charlotte_launch::deployment::MAX_SERVICE_NAME_LEN
            || rights == 0
            || rights & !charlotte_launch::deployment::ALL_GRANT_RIGHTS != 0
            || charlotte_launch::deployment::decode(descriptor).is_none()
        {
            return None;
        }
        let len = REQUEST_HEADER_LEN.checked_add(service.len())?.checked_add(descriptor.len())?;
        let bytes = output.get_mut(..len)?;
        bytes.fill(0);
        bytes[0..4].copy_from_slice(&REQUEST_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&(VERSION as u16).to_le_bytes());
        bytes[6..8].copy_from_slice(&rights.to_le_bytes());
        bytes[8..10].copy_from_slice(&(service.len() as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&(descriptor.len() as u32).to_le_bytes());
        let service_end = REQUEST_HEADER_LEN + service.len();
        bytes[REQUEST_HEADER_LEN..service_end].copy_from_slice(service);
        bytes[service_end..len].copy_from_slice(descriptor);
        Some(len)
    }

    pub fn decode_request(bytes: &[u8]) -> Option<AcquireRequest<'_>> {
        if bytes.len() < REQUEST_HEADER_LEN
            || u32::from_le_bytes(bytes[0..4].try_into().ok()?) != REQUEST_MAGIC
            || u16::from_le_bytes(bytes[4..6].try_into().ok()?) != VERSION as u16
            || u16::from_le_bytes(bytes[10..12].try_into().ok()?) != 0
        {
            return None;
        }
        let rights = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
        let service_len = usize::from(u16::from_le_bytes(bytes[8..10].try_into().ok()?));
        let descriptor_len =
            usize::try_from(u32::from_le_bytes(bytes[12..16].try_into().ok()?)).ok()?;
        let service_end = REQUEST_HEADER_LEN.checked_add(service_len)?;
        let descriptor_end = service_end.checked_add(descriptor_len)?;
        if service_len == 0
            || service_len > charlotte_launch::deployment::MAX_SERVICE_NAME_LEN
            || rights == 0
            || rights & !charlotte_launch::deployment::ALL_GRANT_RIGHTS != 0
            || descriptor_end != bytes.len()
        {
            return None;
        }
        let service = bytes.get(REQUEST_HEADER_LEN..service_end)?;
        let descriptor = bytes.get(service_end..descriptor_end)?;
        charlotte_launch::deployment::decode(descriptor)?;
        Some(AcquireRequest {
            service,
            rights,
            descriptor,
        })
    }
}

/// Name-service interface 2; generation operations use CNSGEN01.
pub mod ns {
    /// Interface id: "NAME".
    pub const INTERFACE: u64 = super::name(b"NAME");
    pub const VERSION: u32 = charlotte_protocol_ns::VERSION;

    /// Register a service under a short (<= 8 byte) name. `arg0` = packed
    /// name; the call must attach a re-delegable connection
    /// (`SEND | CALL | MINT_CONNECTION`) to the service's endpoint. Reply
    /// result = new instance generation (>= 1).
    /// Prepare and activate a local publication through Raft. The reply is
    /// the committed distributed generation (>= 1).
    pub const OP_REGISTER: u32 = 1;
    /// Look up a service by short name. `arg0` = packed name. Reply result =
    /// current generation with an attenuated (`SEND | CALL`) connection cap
    /// attached, or [`ERR_NOT_FOUND`].
    pub const OP_LOOKUP: u32 = 2;
    /// Register under a memory-carried (long) name. `arg0` = name length in
    /// bytes; the call attaches both a copied memory object whose first
    /// `arg0` bytes are the name and a re-delegable connection. Reply result
    /// = new instance generation (>= 1), or [`ERR_INVALID`].
    pub const OP_REGISTER_NAMED: u32 = 3;
    /// Look up a service by memory-carried (long) name. `arg0` = name length;
    /// the call attaches a copied memory object holding the name. Reply as
    /// for [`OP_LOOKUP`].
    pub const OP_LOOKUP_NAMED: u32 = 4;

    /// The name is not registered.
    pub const ERR_NOT_FOUND: i64 = -1;
    /// A register call did not attach a re-delegable connection, or a named
    /// call carried a malformed/oversized name.
    pub const ERR_INVALID: i64 = -2;
    /// Unknown opcode.
    pub const ERR_BAD_OPCODE: i64 = -3;
    /// Principal/rights policy denied the request, or a public lookup targeted
    /// a policy-published service.
    pub const ERR_ACCESS_DENIED: i64 = -4;

    /// Best-effort short-name lookup for optional dependencies. Unlike
    /// [`OP_LOOKUP`], this replies with [`ERR_NOT_FOUND`] immediately rather
    /// than retaining the call until a future registration.
    pub const OP_TRY_LOOKUP: u32 = 7;
    /// Unpublish a short-named service while retaining its generation
    /// tombstone. `arg0` = packed name. Subsequent lookups defer until the
    /// replacement registers, at which point the generation advances
    /// normally. Existing connections retain their normal endpoint lifetime.
    pub const OP_UNREGISTER: u32 = 8;

    /// Move one complete CNSSTAT1 active-publication snapshot (see
    /// `ns_generation::snapshot`). Arg0 must be zero, with no attachments.
    /// Reply result is the exact byte length; tombstones are excluded and
    /// pending counts retained lookup requests. Invalid/allocation failure
    /// returns ERR_INVALID instead of a partial observation.
    pub const OP_STATUS: u32 = 9;
    /// Unpublish the full name and nonzero generation in a mandatory moved
    /// CNSGEN01 record; arg0 is its exact byte length. A stale request returns
    /// [`ERR_NOT_FOUND`] and leaves a replacement untouched.
    pub const OP_UNREGISTER_GENERATION: u32 = charlotte_protocol_ns::OP_UNREGISTER_GENERATION;

    /// Register a service through the production authorization path. The call
    /// attaches a re-delegable connection plus a copied
    /// `authorization::wire::Publish` request. Only a kernel-authenticated
    /// service-manager principal may publish. Reply = binding generation.
    pub const OP_REGISTER_AUTHORIZED: u32 = 11;
    /// Resolve a service through default-deny policy. The copied request is an
    /// `authorization::wire::Lookup`; the returned connection is attenuated
    /// to the explicitly requested rights and the decision is audited.
    pub const OP_LOOKUP_AUTHORIZED: u32 = 12;
    /// Replace an exact principal/service rule. The copied request is an
    /// `authorization::wire::SetPolicy`; only a kernel-authenticated policy
    /// administrator may mutate policy. Reply = new policy version.
    pub const OP_SET_POLICY: u32 = 13;
    /// Return a bounded, variable-length snapshot of authorization audit
    /// records in a moved page. Only policy administrators may inspect it.
    pub const OP_AUTH_AUDIT: u32 = 14;
    /// Resolve for an exact application identity on behalf of the trusted
    /// capability-grant controller. The copied request is an
    /// `authorization::wire::GrantLookup`. Only a policy administrator may
    /// call this operation. Its returned connection is re-delegable solely so
    /// the controller can attenuate and pass it to that application.
    pub const OP_LOOKUP_FOR_GRANT: u32 = 15;
    /// Best-effort memory-carried lookup. This is the long-name equivalent
    /// of [`OP_TRY_LOOKUP`] and never parks the caller.
    pub const OP_TRY_LOOKUP_NAMED: u32 = 16;
    /// Immediate equivalent of [`OP_LOOKUP_FOR_GRANT`], with identical
    /// authorization but no retained waitlist entry when the name is absent.
    pub const OP_TRY_LOOKUP_FOR_GRANT: u32 = 17;

    /// Read the live publication generation, without minting any connection
    /// or changing policy. Requires a moved CNSGEN01 observation record with
    /// generation zero; arg0 is its exact length. Missing/dead names reject.
    /// Like OP_STATUS, this returns naming metadata without granting application access.
    pub const OP_PUBLICATION_GENERATION: u32 = charlotte_protocol_ns::OP_PUBLICATION_GENERATION;

    /// `"AUA1"` little-endian, leading an authorization-audit snapshot.
    pub const AUDIT_MAGIC: u32 = 0x3141_5541;
}

/// Echo-service protocol (`charlotte-protocol-echo` v1).
pub mod echo {
    /// Interface id: "ECHO".
    pub const INTERFACE: u64 = super::name(b"ECHO");
    pub const VERSION: u32 = 1;
    /// The registered short service name.
    pub const NAME: u64 = super::name(b"echo");
    /// The registered long (memory-carried) service name, demonstrating
    /// names beyond the 8-byte scalar limit.
    pub const LONG_NAME: &[u8] = b"system.console.echo.primary.v1";

    /// Reply result = `arg0`.
    pub const OP_ECHO: u32 = 1;
    /// Reply 0, then the service exits its protection domain.
    pub const OP_SHUTDOWN: u32 = 2;
    /// Handoff: the service serialises its state into a memory object,
    /// moves it to the caller (the supervisor), and replies with the
    /// memory cap.  It then exits — the supervisor transfers the state
    /// to a new instance that resumes under the same name with a bumped
    /// generation (live-service-upgrade design).  The caller's ASID is
    /// passed in `arg0` so the service knows where to `move_to`.
    pub const OP_HANDOFF: u32 = 3;
}

/// Console-driver protocol (`charlotte-protocol-console` v1).
///
/// The reference userspace UART driver serves this interface. It is the
/// control/data plane a client uses to reach a device the driver owns
/// through delegated MMIO and interrupt capabilities (architecture doc
/// §10, Phase 8).
pub mod console {
    /// Interface id: "CONS".
    pub const INTERFACE: u64 = super::name(b"CONS");
    pub const VERSION: u32 = 1;
    /// The registered short service name.
    pub const NAME: u64 = super::name(b"uart");

    /// Write one byte (`arg0`'s low 8 bits) to the console device's transmit
    /// FIFO. Reply result = 0 on success.
    pub const OP_WRITE: u32 = 1;
    /// Query the driver. Reply result = the number of device interrupts the
    /// driver has observed and acknowledged (proves the delegated interrupt
    /// path is live).
    pub const OP_STATUS: u32 = 2;
    /// Reply 0, release the device (unmap MMIO, mask/unroute the interrupt),
    /// then exit the protection domain.
    pub const OP_SHUTDOWN: u32 = 3;
    /// Request a device-driven read. The driver does **not** reply
    /// immediately: it retains the reply token and returns to its wait loop,
    /// so the caller's shard is free to run other work (architecture doc
    /// §7.2, deferred replies). When the next device interrupt arrives, the
    /// driver reads the receive register and completes the retained reply,
    /// so the reply is genuinely driven by the hardware interrupt. Reply
    /// result = the byte read (0..=255) in the low bits with the driver's
    /// interrupt count in bits 8.. so the caller can confirm the reply was
    /// interrupt-driven. A second concurrent request replies -1 (busy).
    pub const OP_READ_DEFERRED: u32 = 4;
    /// Uncooperative exit (fault injection): the driver terminates its
    /// protection domain **without** releasing its device capabilities or
    /// completing any retained reply — modelling a crashed driver. The
    /// service manager must then reclaim the device authority (unmap MMIO,
    /// mask/unroute the interrupt) and reconcile the outstanding operation on
    /// teardown (architecture doc §13, success criterion 9). Use send for this
    /// request; it has no reply.
    pub const OP_CRASH: u32 = 5;
}

/// PL011 UART register offsets (ARM PrimeCell PL011), for the reference
/// userspace driver.
pub mod pl011 {
    /// Data register: writing transmits the low byte; reading returns a
    /// received byte in the low 8 bits.
    pub const DR: usize = 0x000;
    /// Flag register.
    pub const FR: usize = 0x018;
    /// FR bit 4: receive FIFO empty.
    pub const FR_RXFE: u32 = 1 << 4;
    /// FR bit 5: transmit FIFO full.
    pub const FR_TXFF: u32 = 1 << 5;
    /// Interrupt mask set/clear register.
    pub const IMSC: usize = 0x038;
    /// IMSC bit 4: receive interrupt.
    pub const IMSC_RXIM: u32 = 1 << 4;
    /// Interrupt clear register.
    pub const ICR: usize = 0x044;
}

/// Network-driver protocol (`charlotte-protocol-net` v1).
///
/// The reference userspace virtio-net driver serves this interface. Its
/// only job is Ethernet frame transport (§6 of the networking architecture
/// doc): it knows nothing about IP, TCP, or sockets. Higher-level services
/// (Reliable Message Layer, RPC, TCP/IP compatibility) consume raw frames
/// through this endpoint.
pub mod net {
    /// Interface id: "NET ".
    pub const INTERFACE: u64 = super::name(b"NET ");
    pub const VERSION: u32 = 1;
    /// The registered short service name.
    pub const NAME: u64 = super::name(b"net0");

    /// Query the driver. Reply result encodes: bits 0..7 = link status
    /// (1 = up), bits 8..55 = MAC address bytes 0..5 (network order), bits
    /// 56..63 = 0.
    pub const OP_STATUS: u32 = 1;

    /// Transmit a raw Ethernet frame. The call attaches a **moved** memory
    /// object holding the frame; the driver transfers ownership of that
    /// buffer to the device and replies when the frame has been handed off
    /// to the TX virtqueue (not when it has been sent on the wire — that is
    /// an IRQ-driven completion). Reply result = 0 on success.
    pub const OP_SEND: u32 = 2;

    /// Request a deferred frame receive. The driver does **not** reply
    /// immediately: it retains the reply token until a frame arrives from
    /// the device, at which point it replies with a **copied** memory object
    /// holding the received frame. The caller owns the returned buffer; the
    /// driver's RX ring is recycled internally. Reply result = 0 on
    /// success. A second concurrent request replies -1 (busy).
    pub const OP_RECV: u32 = 3;

    /// Reply 0, release the device, then exit.
    pub const OP_SHUTDOWN: u32 = 4;
}

/// Virtio-net device MMIO offsets (virtio legacy transport, common config
/// at BAR0 offset 0 on the transitional device QEMU exposes).
pub mod virtio {
    // QEMU virtio-pci modern capability layout within BAR 4.
    pub const MODERN_COMMON: usize = 0x0000;
    pub const MODERN_ISR: usize = 0x1000;
    pub const MODERN_DEVICE: usize = 0x2000;
    pub const MODERN_NOTIFY: usize = 0x3000;
    pub const MODERN_NOTIFY_MULTIPLIER: usize = 4;

    pub const M_DEVICE_FEATURE_SELECT: usize = 0x00;
    pub const M_DEVICE_FEATURE: usize = 0x04;
    pub const M_DRIVER_FEATURE_SELECT: usize = 0x08;
    pub const M_DRIVER_FEATURE: usize = 0x0c;
    pub const M_CONFIG_VECTOR: usize = 0x10;
    pub const M_DEVICE_STATUS: usize = 0x14;
    pub const M_QUEUE_SELECT: usize = 0x16;
    pub const M_QUEUE_SIZE: usize = 0x18;
    pub const M_QUEUE_VECTOR: usize = 0x1a;
    pub const M_QUEUE_ENABLE: usize = 0x1c;
    pub const M_QUEUE_NOTIFY_OFF: usize = 0x1e;
    pub const M_QUEUE_DESC: usize = 0x20;
    pub const M_QUEUE_DRIVER: usize = 0x28;
    pub const M_QUEUE_DEVICE: usize = 0x30;

    /// Offset within BAR0 of the common config (legacy layout).
    /// QEMU `virt` places the first legacy PCI I/O BAR at port 0x80. On
    /// AArch64 that aperture is translated into a page of system MMIO which
    /// the supervisor delegates to this reference driver.
    pub const COMMON_CFG_OFFSET: usize = 0x080;
    /// Device-specific config offset (legacy: immediate after common config).
    /// MSI-X adds the configuration-vector and queue-vector registers to the
    /// legacy common header, moving device-specific configuration by 4 bytes.
    pub const DEVICE_CFG_OFFSET: usize = 0x018;

    // Common config registers relative to COMMON_CFG_OFFSET.
    pub const DEVICE_FEATURES: usize = 0x00; // u32 r/o
    pub const GUEST_FEATURES: usize = 0x04; // u32 w/o
    pub const QUEUE_ADDRESS: usize = 0x08; // u32 w/o  (PFN)
    pub const QUEUE_SIZE: usize = 0x0c; // u16 w/o
    pub const QUEUE_SELECT: usize = 0x0e; // u16 w/o
    pub const QUEUE_NOTIFY: usize = 0x10; // u16 w/o
    pub const DEVICE_STATUS: usize = 0x12; // u8  r/w
    pub const ISR_STATUS: usize = 0x13; // u8  r/o
    pub const CONFIG_VECTOR: usize = 0x14; // u16 r/w (MSI-X)
    pub const QUEUE_VECTOR: usize = 0x16; // u16 r/w (MSI-X)

    // Device status bits.
    pub const STATUS_ACKNOWLEDGE: u8 = 1;
    pub const STATUS_DRIVER: u8 = 2;
    pub const STATUS_DRIVER_OK: u8 = 4;
    pub const STATUS_FEATURES_OK: u8 = 8;

    // Device-specific config (net).
    pub const NET_MAC: usize = 0x00; // 6 bytes r/o
    pub const NET_STATUS: usize = 0x06; // u16 r/o, bit 0 = link up

    // Virtqueue descriptor ring (per queue).
    /// Size of one virtqueue descriptor.
    pub const DESC_SIZE: usize = 16;
    /// Descriptor field offsets within the 16-byte struct.
    pub const DESC_ADDR_LO: usize = 0; // u32 (guest physical lo)
    pub const DESC_ADDR_HI: usize = 4; // u32 (guest physical hi)
    pub const DESC_LENGTH: usize = 8; // u32
    pub const DESC_FLAGS: usize = 12; // u16
    pub const DESC_NEXT: usize = 14; // u16

    pub const VRING_DESC_F_NEXT: u16 = 1;
    pub const VRING_DESC_F_WRITE: u16 = 2;

    /// Available ring layout (after descriptor table).
    pub const AVAIL_FLAGS: usize = 0; // u16
    pub const AVAIL_IDX: usize = 2; // u16
    pub const AVAIL_RING: usize = 4; // u16[queue_size]
    // AVAIL_USED_EVENT at ring[queue_size] (u16) — ignored for now.

    /// Used ring layout (separate page from avail ring).
    pub const USED_FLAGS: usize = 0; // u16
    pub const USED_IDX: usize = 2; // u16
    pub const USED_RING: usize = 4; // UsedElem[queue_size]

    pub const VIRTQ_RX: u16 = 0;
    pub const VIRTQ_TX: u16 = 1;

    /// Small queue size for the smoke test.
    pub const QUEUE_COUNT: u16 = 32;

    pub const FEATURE_MAC: u32 = 1 << 5;
    pub const FEATURE_STATUS: u32 = 1 << 16;
    pub const FEATURE_VERSION_1: u32 = 1;
    pub const FEATURE_ACCESS_PLATFORM: u32 = 1 << 1;
}

/// Cryptographic entropy service backed by a delegated VirtIO RNG device.
pub mod entropy {
    pub const INTERFACE: u64 = super::name(b"RNG");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"rng");

    /// Return up to `arg0` random bytes in a moved memory object. The scalar
    /// result is the exact initialized byte count.
    pub const OP_FILL: u32 = 1;
    pub const MAX_REQUEST: usize = 4_096;

    pub const ERR_INVALID: i64 = -1;
    pub const ERR_DEVICE: i64 = -2;
    pub const ERR_MEMORY: i64 = -3;
    pub const ERR_BAD_OPCODE: i64 = -4;
}

/// Stage a memory-carried name: allocate a one-page memory object, write
/// `name` at offset 0, and return the memory cap (unmapped, ready to attach
/// to a copied-memory call).
///
/// Returns `None` when the name is empty/oversized or allocation fails.
///
/// # Safety
/// Uses [`NAME_SCRATCH_VADDR`], which must be unmapped in the caller's
/// address space, and must not race with other users of the scratch page.
pub unsafe fn stage_name(name: &[u8]) -> Option<u64> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return None;
    }
    let cap = catten_syscall::memory_alloc(1);
    if cap == 0 {
        return None;
    }
    if catten_syscall::memory_map(cap, NAME_SCRATCH_VADDR, true) != 0 {
        catten_syscall::memory_close(cap);
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), NAME_SCRATCH_VADDR as *mut u8, name.len());
        catten_syscall::memory_unmap(cap);
    }
    Some(cap)
}

/// Stage a memory-carried service name using the ownership-aware runtime API.
/// The returned object is unmapped and can be copied or moved into an IPC
/// operation without any manual cleanup path.
pub fn stage_name_owned(name: &[u8]) -> Option<catten_rt::owned::OwnedMemory> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return None;
    }
    let memory = catten_rt::owned::OwnedMemory::allocate(1).ok()?;
    let mut mapping = memory.map_writable().ok()?;
    mapping.as_mut_slice()[..name.len()].copy_from_slice(name);
    mapping.unmap().ok()
}

/// Raft consensus protocol (`charlotte-protocol-raft` v1).
///
/// The opcodes are deliberately aligned with the `graft` crate's message
/// types so that the wire-level encoding stays identical across the
/// standard-library (Tokio) and CharlotteOS builds.
pub mod raft {
    pub const INTERFACE: u64 = super::name(b"RAFT");
    pub const VERSION: u32 = 1;

    /// Direct-network EtherType used by both the isolated generic Raft
    /// fixture and the operational DNS-owned Raft member. The frame router
    /// delivers the latter to `dns::OP_RAFT_FRAME`.
    pub const ETHERTYPE: u16 = 0x88b7;
    /// Well-known name the frame demultiplexer routes `ETHERTYPE` frames to.
    pub const FRAME_NAME: u64 = super::name(b"raft-f");
    /// Ingress opcode the frame demultiplexer delivers frames with.
    pub const OP_FRAME: u32 = 0x10;

    pub const OP_VOTE_REQUEST: u32 = 1;
    pub const OP_APPEND_ENTRIES: u32 = 2;
    pub const OP_INSTALL_SNAPSHOT: u32 = 3;
    pub const OP_CLIENT_COMMAND: u32 = 4;
    pub const OP_CLIENT_QUERY: u32 = 5;
    pub const OP_ADD_SERVER: u32 = 6;
    pub const OP_REMOVE_SERVER: u32 = 7;
    pub const OP_STATUS: u32 = 8;
    /// Cluster-wide status snapshot, memory-moved in the reply: a packed
    /// blob built by [`build_cluster_status`]. Unlike `OP_STATUS` this also
    /// reports the known leader's raft id (the redirect hint) and the number
    /// of configured members, so discovery can answer "who leads the cluster
    /// I am in (if any)".
    pub const OP_CLUSTER_STATUS: u32 = 9;

    pub const ERR_NOT_LEADER: i64 = -1;
    pub const ERR_LOG_INCONSISTENCY: i64 = -2;
    pub const ERR_STALE_TERM: i64 = -3;
    pub const ERR_NOT_FOUND: i64 = -4;

    /// Packed cluster-status reply: `[0..4)` u32 state (1 follower, 2
    /// candidate, 3 leader), `[4..12)` u64 term, `[12..20)` u64 commit
    /// index, `[20..24)` u32 member count, `[24]` u8 leader-id length,
    /// `[25..)` leader-id bytes, then `[len]` u8 + the node's own raft id.
    pub const CLUSTER_STATUS_HEADER_LEN: usize = 25;
    /// Minimum length of a packed peer spec: `[0]` u8 id length, `[1..)`
    /// id bytes, then u8-le-encoded u64 service name, then u8 role
    /// (0 = voter, nonzero = learner).
    pub const PEER_SPEC_MIN_LEN: usize = 10;

    /// Build a packed cluster-status blob into `buf`. Returns the written
    /// length, or `None` if the buffer is too small or the leader id too
    /// long.
    pub fn build_cluster_status(
        buf: &mut [u8],
        state: u32,
        term: u64,
        commit_index: u64,
        member_count: u32,
        leader_id: &[u8],
        self_raft_id: &[u8],
    ) -> Option<usize> {
        if leader_id.len() > 255
            || self_raft_id.len() > 255
            || buf.len() < CLUSTER_STATUS_HEADER_LEN + leader_id.len() + 1 + self_raft_id.len()
        {
            return None;
        }
        buf[0..4].copy_from_slice(&state.to_le_bytes());
        buf[4..12].copy_from_slice(&term.to_le_bytes());
        buf[12..20].copy_from_slice(&commit_index.to_le_bytes());
        buf[20..24].copy_from_slice(&member_count.to_le_bytes());
        buf[24] = leader_id.len() as u8;
        buf[25..25 + leader_id.len()].copy_from_slice(leader_id);
        let pos = 25 + leader_id.len();
        buf[pos] = self_raft_id.len() as u8;
        buf[pos + 1..pos + 1 + self_raft_id.len()].copy_from_slice(self_raft_id);
        Some(pos + 1 + self_raft_id.len())
    }

    /// A parsed cluster-status blob: `(state, term, commit_index,
    /// member_count, leader_id, self_raft_id)`.
    pub type ClusterStatus<'a> = (u32, u64, u64, u32, &'a [u8], &'a [u8]);

    /// Parse a packed cluster-status blob into a [`ClusterStatus`].
    pub fn parse_cluster_status(payload: &[u8]) -> Option<ClusterStatus<'_>> {
        if payload.len() < CLUSTER_STATUS_HEADER_LEN {
            return None;
        }
        let state = u32::from_le_bytes(payload[0..4].try_into().ok()?);
        let term = u64::from_le_bytes(payload[4..12].try_into().ok()?);
        let commit_index = u64::from_le_bytes(payload[12..20].try_into().ok()?);
        let member_count = u32::from_le_bytes(payload[20..24].try_into().ok()?);
        let leader_len = payload[24] as usize;
        if payload.len() < CLUSTER_STATUS_HEADER_LEN + leader_len + 1 {
            return None;
        }
        let leader_id = &payload[25..25 + leader_len];
        let pos = 25 + leader_len;
        let self_len = payload[pos] as usize;
        if payload.len() < pos + 1 + self_len {
            return None;
        }
        Some((
            state,
            term,
            commit_index,
            member_count,
            leader_id,
            &payload[pos + 1..pos + 1 + self_len],
        ))
    }

    /// Pack a peer spec for `OP_ADD_SERVER` into `buf`. Returns the written
    /// length, or `None` if the id is empty/too long or the buffer is too
    /// small.
    pub fn encode_peer_spec(
        buf: &mut [u8],
        id: &[u8],
        service_name: u64,
        learner: bool,
    ) -> Option<usize> {
        if id.is_empty() || id.len() > 255 || buf.len() < 1 + id.len() + 9 {
            return None;
        }
        buf[0] = id.len() as u8;
        buf[1..1 + id.len()].copy_from_slice(id);
        buf[1 + id.len()..1 + id.len() + 8].copy_from_slice(&service_name.to_le_bytes());
        buf[1 + id.len() + 8] = learner as u8;
        Some(1 + id.len() + 9)
    }

    /// Parse a packed peer spec into `(id, service_name, learner)`.
    pub fn decode_peer_spec(payload: &[u8]) -> Option<(&[u8], u64, bool)> {
        if payload.len() < PEER_SPEC_MIN_LEN {
            return None;
        }
        let id_len = payload[0] as usize;
        if id_len == 0 || id_len > 255 || payload.len() < 1 + id_len + 9 {
            return None;
        }
        let id = &payload[1..1 + id_len];
        let service_name = u64::from_le_bytes(payload[1 + id_len..1 + id_len + 8].try_into().ok()?);
        let learner = payload[1 + id_len + 8] != 0;
        Some((id, service_name, learner))
    }
}

/// Block device protocol (`charlotte-protocol-block` v1).
///
/// Defines the interface between block device consumers (filesystems, Raft
/// log stores, etc.) and block device drivers (NVMe, AHCI, etc.). The
/// driver knows nothing about filesystems, partitions, or higher-level
/// storage semantics — it reads and writes fixed-size blocks at linear
/// block addresses.
pub mod block {
    pub const INTERFACE: u64 = super::name(b"BLOCK");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"blk0");

    pub const OP_INFO: u32 = 1;
    pub const OP_READ: u32 = 2;
    pub const OP_WRITE: u32 = 3;
    pub const OP_FLUSH: u32 = 4;
    pub const OP_TRIM: u32 = 5;

    pub const ERR_OK: i64 = 0;
    pub const ERR_IO_ERROR: i64 = 1;
    pub const ERR_INVALID_RANGE: i64 = 2;
    pub const ERR_UNALIGNED: i64 = 3;
    pub const ERR_DEVICE_GONE: i64 = 4;
}

/// Socket protocol (`charlotte-protocol-socket` v1).
///
/// The TCP/IP service exposes this interface. A client looks up "tcpip"
/// from the name service and calls socket operations on the returned
/// connection capability. TCP and connected UDP sockets share the same
/// operations. Data payloads use memory-object transfer (`Move` for send,
/// `Move` on reply for recv). The service authenticates the sender envelope,
/// binds each socket to that sender generation/principal, and enforces the
/// launch-configured per-principal socket and buffer quotas.
pub mod socket {
    use catten_rt::owned::{
        CallResult,
        ConnectionRef,
        IpcError,
        MemoryError,
        OwnedMemory,
        PendingCall,
    };

    pub const INTERFACE: u64 = super::name(b"SKT");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"tcpip");

    pub const OP_SOCKET: u32 = 1;
    /// Select a remote IPv4/port from a six-byte moved memory object. TCP
    /// begins its handshake; UDP binds an ephemeral local port and retains
    /// the remote endpoint for subsequent send/receive filtering.
    pub const OP_CONNECT: u32 = 2;
    /// Bind TCP or UDP to the little-endian port in a moved memory object.
    pub const OP_BIND: u32 = 3;
    pub const OP_LISTEN: u32 = 4;
    /// Observe the current accept state without retaining a waiter.
    pub const OP_TRY_ACCEPT: u32 = 5;
    pub const OP_SEND: u32 = 6;
    /// Move one writable `REPLY_BUFFER_BYTES` buffer into the call. Reply
    /// returns that buffer with the byte count, or a scalar error/EOF. The
    /// original allocating principal remains its backing sponsor.
    pub const OP_RECV: u32 = 7;
    pub const OP_CLOSE: u32 = 8;
    pub const OP_FRAME: u32 = 9;
    /// Move one writable `REPLY_BUFFER_BYTES` buffer into the call. Reply
    /// returns it holding the packed `STATUS_*` snapshot.
    pub const OP_STATUS: u32 = 10;
    pub const REPLY_BUFFER_BYTES: usize = 4096;

    /// Receive/status never allocate service-sponsored reply backing. The
    /// caller moves its own buffer; submission rejection returns its owner.
    pub fn buffered_call(
        service: ConnectionRef<'_>,
        opcode: u32,
        arg0: u64,
    ) -> Result<PendingCall<'static>, SocketError> {
        let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
        service.call_move(opcode, arg0, memory).map_err(|(_, error)| SocketError::Ipc(error))
    }
    /// Cancel this socket's deferred `OP_RECV`, if any. The receive call is
    /// completed with `ERR_WOULD_BLOCK` before this request is acknowledged,
    /// allowing a bounded client wait to release the server-side reply slot
    /// without closing the socket.
    pub const OP_CANCEL_RECV: u32 = 11;
    /// Query one socket's connection state without consuming it. The scalar
    /// reply is one of `CONNECTION_STATE_*`; this lets event loops distinguish
    /// an accepted nonblocking connect request from an established TCP flow.
    pub const OP_CONNECTION_STATE: u32 = 12;
    /// Bind TCP or UDP to one exact IPv4 address and port from a six-byte
    /// moved memory object. This permits equal ports on distinct service VIPs.
    pub const OP_BIND_IPV4: u32 = 13;
    /// Listen for TCP on one exact IPv4 address and port. The payload matches
    /// [`OP_BIND_IPV4`].
    pub const OP_LISTEN_IPV4: u32 = 14;
    /// Retain one complete call until this TCP listener establishes a peer or
    /// closes. A successful scalar reply is zero. Bind its PendingCall to a
    /// CQ to combine accept readiness with other socket/deadline interests.
    pub const OP_ACCEPT: u32 = 15;

    pub const CONNECTION_STATE_CLOSED: i64 = 0;
    pub const CONNECTION_STATE_CONNECTING: i64 = 1;
    pub const CONNECTION_STATE_ESTABLISHED: i64 = 2;
    pub const CONNECTION_STATE_CLOSING: i64 = 3;

    /// `TcpipStatus` snapshot layout (all little-endian u32 words in a moved
    /// page). Offsets are in u32 words from the base of the reply page. Words
    /// 0..=4 are the original v1 layout; the remaining words extend it without
    /// disturbing existing readers, which only consume the leading words.
    pub const STATUS_OFFSET_IP: u32 = 0;
    pub const STATUS_OFFSET_RX_FRAMES: u32 = 1;
    pub const STATUS_OFFSET_TX_SENDS: u32 = 2;
    pub const STATUS_OFFSET_SOCKETS: u32 = 3;
    pub const STATUS_OFFSET_MAGIC: u32 = 4;
    /// Transmit calls that failed (send buffer full or socket error).
    pub const STATUS_OFFSET_TX_SEND_ERRORS: u32 = 5;
    /// Address-acquisition mode: 0 = static, 1 = DHCP.
    pub const STATUS_OFFSET_DHCP_MODE: u32 = 6;
    /// Default-route gateway in network order, 0 when none is configured.
    pub const STATUS_OFFSET_GATEWAY: u32 = 7;
    /// Interface MTU in bytes.
    pub const STATUS_OFFSET_MTU: u32 = 8;
    /// Effective SocketSet slots after heap-based resource clamping, including
    /// the reserved DHCP slot when active.
    pub const STATUS_OFFSET_SOCKET_CAPACITY: u32 = 9;
    /// Per-principal socket quota from the launch policy.
    pub const STATUS_OFFSET_SOCKET_QUOTA: u32 = 10;
    /// Per-principal buffer quota in bytes from the launch policy.
    pub const STATUS_OFFSET_BUFFER_QUOTA: u32 = 11;
    /// Requested socket-table ceiling before heap-based clamping.
    pub const STATUS_OFFSET_SOCKET_CEILING: u32 = 12;
    pub const STATUS_WORDS: usize = 13;
    pub const STATUS_MAGIC: u32 = 0x5443_5053;

    /// Shared SocketSet capacity (after reserving DHCP, when enabled) is full.
    pub const ERR_TOO_MANY_SOCKETS: i64 = -1;
    pub const ERR_BAD_SOCKET: i64 = -2;
    pub const ERR_CONNECTION_REFUSED: i64 = -3;
    pub const ERR_NOT_CONNECTED: i64 = -4;
    pub const ERR_WOULD_BLOCK: i64 = -5;
    pub const ERR_BAD_DOMAIN: i64 = -6;
    pub const ERR_BAD_OPCODE: i64 = -7;
    /// The caller's per-principal socket or buffer budget is exhausted.
    pub const ERR_QUOTA_EXCEEDED: i64 = -8;

    pub const DOMAIN_TCP: u64 = 1;
    pub const DOMAIN_UDP: u64 = 2;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SocketError {
        Ipc(IpcError),
        Memory(MemoryError),
        Service(i64),
        InvalidReply,
        RetryExhausted,
    }

    impl From<IpcError> for SocketError {
        fn from(value: IpcError) -> Self {
            Self::Ipc(value)
        }
    }

    /// A receive result whose memory remains owned until explicitly moved or
    /// dropped. Only `len` bytes at the start of the page are initialized.
    #[must_use = "dropping a received socket chunk releases its memory"]
    pub struct ReceivedChunk {
        memory: OwnedMemory,
        len: usize,
    }

    impl ReceivedChunk {
        pub const fn len(&self) -> usize {
            self.len
        }

        pub const fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn memory(&self) -> &OwnedMemory {
            &self.memory
        }

        pub fn into_parts(self) -> (OwnedMemory, usize) {
            (self.memory, self.len)
        }
    }

    /// A socket ID owned by a connection to the TCP/IP service.
    ///
    /// Remote socket IDs are protocol resources rather than kernel
    /// capabilities, so their teardown can fail. [`close`](Self::close)
    /// reports that failure; `Drop` provides a best-effort fallback for early
    /// returns and cancellation paths.
    #[must_use = "dropping a socket asks the TCP/IP service to close it"]
    pub struct OwnedSocket<'connection> {
        service: ConnectionRef<'connection>,
        id: Option<u64>,
    }

    impl<'connection> OwnedSocket<'connection> {
        pub fn open(service: ConnectionRef<'connection>, domain: u64) -> Result<Self, IpcError> {
            let result = service.call(OP_SOCKET, domain)?.wait()?;
            if result.result < 1 {
                return Err(IpcError::Status(result.result as u64));
            }
            Ok(Self {
                service,
                id: Some(result.result as u64),
            })
        }

        pub fn id(&self) -> u64 {
            self.id.expect("socket already closed")
        }

        pub fn call(&self, opcode: u32, arg0: u64) -> Result<PendingCall<'static>, IpcError> {
            self.service.call(opcode, arg0)
        }

        pub fn accept_call(&self) -> Result<PendingCall<'static>, IpcError> {
            self.service.call(OP_ACCEPT, self.id())
        }

        /// Return the protocol-neutral connection state exposed by tcpip.
        pub fn connection_state(&self) -> Result<i64, SocketError> {
            self.service
                .call(OP_CONNECTION_STATE, self.id())
                .map_err(SocketError::Ipc)?
                .wait()
                .map(|result| result.result)
                .map_err(SocketError::Ipc)
        }

        /// Select an IPv4 peer and begin connecting. TCP handshaking proceeds
        /// in the tcpip reactor; [`send_all`](Self::send_all) handles its
        /// transient `ERR_WOULD_BLOCK` result.
        pub fn connect_ipv4(&self, address: [u8; 4], port: u16) -> Result<(), SocketError> {
            let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
            let mut mapping =
                memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
            mapping.as_mut_slice()[..4].copy_from_slice(&address);
            mapping.as_mut_slice()[4..6].copy_from_slice(&port.to_le_bytes());
            let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
            let result = self
                .service
                .call_move(OP_CONNECT, self.id(), memory)
                .map_err(|(_, error)| SocketError::Ipc(error))?
                .wait()
                .map_err(SocketError::Ipc)?
                .result;
            if result == 0 {
                Ok(())
            } else {
                Err(SocketError::Service(result))
            }
        }

        fn local_ipv4_operation(
            &self,
            opcode: u32,
            address: [u8; 4],
            port: u16,
        ) -> Result<(), SocketError> {
            let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
            let mut mapping =
                memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
            mapping.as_mut_slice()[..4].copy_from_slice(&address);
            mapping.as_mut_slice()[4..6].copy_from_slice(&port.to_le_bytes());
            let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
            let result = self
                .service
                .call_move(opcode, self.id(), memory)
                .map_err(|(_, error)| SocketError::Ipc(error))?
                .wait()
                .map_err(SocketError::Ipc)?
                .result;
            if result == 0 {
                Ok(())
            } else {
                Err(SocketError::Service(result))
            }
        }

        pub fn bind_ipv4(&self, address: [u8; 4], port: u16) -> Result<(), SocketError> {
            self.local_ipv4_operation(OP_BIND_IPV4, address, port)
        }

        /// Listen on any local address using the port-only protocol operation.
        /// This borrows the socket owner through setup and consumes its moved
        /// request backing exactly once; rejection leaves the socket owned.
        pub fn listen(&self, port: u16) -> Result<(), SocketError> {
            let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
            let mut mapping =
                memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
            mapping.as_mut_slice()[..2].copy_from_slice(&port.to_le_bytes());
            let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
            let result = self
                .service
                .call_move(OP_LISTEN, self.id(), memory)
                .map_err(|(_, error)| SocketError::Ipc(error))?
                .wait()
                .map_err(SocketError::Ipc)?
                .result;
            if result == 0 {
                Ok(())
            } else {
                Err(SocketError::Service(result))
            }
        }

        pub fn listen_ipv4(&self, address: [u8; 4], port: u16) -> Result<(), SocketError> {
            self.local_ipv4_operation(OP_LISTEN_IPV4, address, port)
        }

        /// Send the entire byte slice as one or more moved pages, handling
        /// partial acceptance and tcpip backpressure without exposing socket
        /// IDs or cleanup work to the caller.
        pub fn send_all(
            &self,
            bytes: &[u8],
            attempts: usize,
            retry_ms: u64,
        ) -> Result<(), SocketError> {
            let mut offset = 0usize;
            for _ in 0..attempts {
                if offset == bytes.len() {
                    return Ok(());
                }
                let chunk_len = (bytes.len() - offset).min(4096);
                let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
                let mut mapping =
                    memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
                mapping.as_mut_slice()[..chunk_len]
                    .copy_from_slice(&bytes[offset..offset + chunk_len]);
                let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
                let packed = ((chunk_len as u64) << 32) | (self.id() & 0xffff_ffff);
                let result = self
                    .service
                    .call_move(OP_SEND, packed, memory)
                    .map_err(|(_, error)| SocketError::Ipc(error))?
                    .wait()
                    .map_err(SocketError::Ipc)?
                    .result;
                if result > 0 && result as usize <= chunk_len {
                    offset += result as usize;
                } else if result == ERR_WOULD_BLOCK || result == 0 {
                    super::sleep_ms(retry_ms);
                } else if result > chunk_len as i64 {
                    return Err(SocketError::InvalidReply);
                } else {
                    return Err(SocketError::Service(result));
                }
            }
            Err(SocketError::RetryExhausted)
        }

        /// Submit a receive using this caller's backing, without waiting.
        pub fn receive_call(&self) -> Result<PendingCall<'static>, SocketError> {
            buffered_call(self.service, OP_RECV, self.id())
        }

        /// Wait for and own the receive page supplied by this caller.
        pub fn receive(&self) -> Result<Option<ReceivedChunk>, SocketError> {
            let result = self.receive_call()?.wait().map_err(SocketError::Ipc)?;
            Self::decode_receive(result)
        }

        /// Poll for one receive page, returning `RetryExhausted` after a
        /// bounded wait. On timeout this explicitly cancels tcpip's retained
        /// receive slot before releasing the local pending-call capability.
        pub fn receive_timeout(
            &self,
            attempts: usize,
            retry_ms: u64,
        ) -> Result<Option<ReceivedChunk>, SocketError> {
            let mut pending = self.receive_call()?;
            for _ in 0..attempts {
                if let Some(result) = pending.poll().map_err(SocketError::Ipc)? {
                    return Self::decode_receive(result);
                }
                super::sleep_ms(retry_ms);
            }
            let cancel =
                self.service.call(OP_CANCEL_RECV, self.id())?.wait().map_err(SocketError::Ipc)?;
            if cancel.result != 0 {
                return Err(SocketError::Service(cancel.result));
            }
            // tcpip processes messages from this connection in order. If the
            // reactor delivered bytes before observing the cancellation, keep
            // them; otherwise OP_CANCEL_RECV completes this call with the
            // distinguished timeout result.
            let result = pending.wait().map_err(SocketError::Ipc)?;
            if result.result == ERR_WOULD_BLOCK
                && result.connection.is_none()
                && result.memory.is_none()
            {
                Err(SocketError::RetryExhausted)
            } else {
                Self::decode_receive(result)
            }
        }

        pub(crate) fn decode_receive(
            result: CallResult,
        ) -> Result<Option<ReceivedChunk>, SocketError> {
            if result.connection.is_some() {
                return Err(SocketError::InvalidReply);
            }
            if result.result < 0 {
                return Err(SocketError::Service(result.result));
            }
            if result.result == 0 {
                return if result.memory.is_none() {
                    Ok(None)
                } else {
                    Err(SocketError::InvalidReply)
                };
            }
            let memory = result.memory.ok_or(SocketError::InvalidReply)?;
            let len = result.result as usize;
            if len > memory.len() {
                return Err(SocketError::InvalidReply);
            }
            Ok(Some(ReceivedChunk {
                memory,
                len,
            }))
        }

        pub fn close(mut self) -> Result<(), IpcError> {
            self.close_inner()
        }

        fn close_inner(&mut self) -> Result<(), IpcError> {
            let Some(id) = self.id.take() else {
                return Ok(());
            };
            // Retry only rejected admission, never an accepted close. Keep
            // the remote owner through the total, bounded queue-pressure wait.
            let deadline = super::deadline::Deadline::after(1_000);
            let call = loop {
                match self.service.call(OP_CLOSE, id) {
                    Err(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL))
                        if !deadline.expired() =>
                    {
                        super::sleep_ms(5)
                    }
                    result => break result,
                }
            };
            let call = match call {
                Ok(call) => call,
                Err(error) => {
                    // Submission did not transfer the request. Retain the ID
                    // so the consuming close's Drop fallback can retry once.
                    self.id = Some(id);
                    return Err(error);
                }
            };
            let result = match call.wait() {
                Ok(result) => result,
                Err(error) => {
                    self.id = Some(id);
                    return Err(error);
                }
            };
            if result.result == 0 {
                Ok(())
            } else {
                self.id = Some(id);
                Err(IpcError::Status(result.result as u64))
            }
        }
    }

    impl Drop for OwnedSocket<'_> {
        fn drop(&mut self) {
            let _ = self.close_inner();
        }
    }
}

/// The initial service exports scheduler statistics for its own protection
/// domain. Other services can expose or voluntarily publish their own
/// snapshots without granting an observer ambient cross-domain authority.
pub mod observability {
    pub const INTERFACE: u64 = super::name(b"OBSERVE");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"observe");

    /// Reply moves a memory object containing the
    /// `catten_syscall::THREAD_STATISTICS_*` wire format. The scalar result is
    /// its exact byte length.
    pub const OP_THREAD_SNAPSHOT: u32 = 1;

    /// Reply moves a memory object containing a bounded history of system
    /// resource samples in the `CCHIST` wire format below. The scalar result
    /// is its exact byte length. History is retained in the observe service's
    /// own heap and lost on restart.
    pub const OP_HISTORY: u32 = 2;

    pub const ERR_UNAVAILABLE: i64 = -1;
    pub const ERR_BAD_OPCODE: i64 = -2;

    /// `CCHIST01`, little-endian.
    pub const HISTORY_MAGIC: u64 = 0x3130_5453_4948_4343;
    pub const HISTORY_VERSION: u64 = 2;

    pub mod history_header {
        pub const MAGIC: usize = 0;
        pub const VERSION: usize = 1;
        pub const RECORD_BYTES: usize = 2;
        pub const RECORD_COUNT: usize = 3;
        pub const COUNTER_FREQUENCY_HZ: usize = 4;
        pub const SAMPLE_INTERVAL_MS: usize = 5;
        pub const WORDS: usize = 6;
    }

    pub mod history_record {
        pub const MONOTONIC_TICKS: usize = 0;
        pub const THREADS: usize = 1;
        pub const DOMAINS: usize = 2;
        pub const OWNED_FRAMES: usize = 3;
        pub const STACK_PAGES: usize = 4;
        pub const STACK_USED_HIGH_WATER: usize = 5;
        pub const THREADS_HIGH_WATER: usize = 6;
        pub const FREE_FRAMES: usize = 7;
        pub const USABLE_FRAMES: usize = 8;
        pub const LOGICAL_PROCESSORS: usize = 9;
        pub const CPU_BUSY_TICKS: usize = 10;
        pub const HEAP_ALLOCATED_BYTES: usize = 11;
        pub const HEAP_PEAK_BYTES: usize = 12;
        pub const WORDS: usize = 13;
    }

    /// Sample cadence published in the history header.
    pub const HISTORY_SAMPLE_INTERVAL_MS: u64 = 1_000;
    /// Bounded ring capacity: about four minutes of history per MiB-free heap.
    pub const HISTORY_CAPACITY: usize = 256;

    /// `CCARCH01`, little-endian.
    pub const ARCHIVE_MAGIC: u64 = 0x3130_4843_5241_4343;
    pub const ARCHIVE_VERSION: u64 = 2;
    /// Stable object-store IDs for the archive chunk ring. The `0xfffc`
    /// partition is reserved for telemetry between the Raft (0x8000), node
    /// identity (0x9000), time (0xfffd), and artifact (0xfffe) partitions.
    pub const ARCHIVE_BASE_ID: u64 = 0xfffc_0000_0000_0001;
    /// Number of chunk objects in the ring. Bounds directory-slot use.
    pub const ARCHIVE_CHUNKS: u64 = 16;
    /// One chunk is two pages, the size the object store writes efficiently.
    pub const ARCHIVE_CHUNK_BYTES: usize = 8192;
    /// A partially filled chunk is rewritten at this cadence.
    pub const ARCHIVE_FLUSH_INTERVAL_MS: u64 = 10_000;

    pub mod archive_header {
        pub const MAGIC: usize = 0;
        pub const VERSION: usize = 1;
        pub const CHUNK_INDEX: usize = 2;
        pub const SESSION_TICKS: usize = 3;
        pub const FIRST_SEQUENCE: usize = 4;
        pub const RECORD_COUNT: usize = 5;
        pub const COUNTER_FREQUENCY_HZ: usize = 6;
        pub const WORDS: usize = 7;
    }

    pub mod archive_record {
        pub const SEQUENCE: usize = 0;
        pub const MONOTONIC_TICKS: usize = 1;
        pub const THREADS: usize = 2;
        pub const DOMAINS: usize = 3;
        pub const OWNED_FRAMES: usize = 4;
        pub const STACK_PAGES: usize = 5;
        pub const STACK_USED_HIGH_WATER: usize = 6;
        pub const THREADS_HIGH_WATER: usize = 7;
        pub const FREE_FRAMES: usize = 8;
        pub const USABLE_FRAMES: usize = 9;
        pub const LOGICAL_PROCESSORS: usize = 10;
        pub const CPU_BUSY_TICKS: usize = 11;
        pub const HEAP_ALLOCATED_BYTES: usize = 12;
        pub const HEAP_PEAK_BYTES: usize = 13;
        pub const WORDS: usize = 14;
    }
}

/// Frame demultiplexer status protocol.
pub mod frouter {
    pub const INTERFACE: u64 = super::name(b"FROUTER");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"frouter");

    pub const OP_STATUS: u32 = 1;

    /// Normal interval for refreshing the DNS-owned ingress projection.
    pub const SNAPSHOT_REFRESH_MS: u64 = 1_000;
    /// Maximum age of a follower's last successful leader replication when
    /// DNS materializes an ingress snapshot. This bounds the upstream source
    /// that is allowed to renew the frame router's longer admission lease.
    pub const SNAPSHOT_SOURCE_MAX_AGE_MS: u64 = SNAPSHOT_REFRESH_MS;
    /// Maximum age of the last complete projection for VIP advertisement and
    /// admission of a flow not already present in the local binding table.
    pub const SNAPSHOT_LEASE_MS: u64 = SNAPSHOT_REFRESH_MS * 5;

    pub const ERR_BAD_OPCODE: i64 = -1;

    /// `FrouterStatus` snapshot layout (little-endian u32 words in a moved
    /// page). Offsets are in u32 words from the base of the reply page.
    pub const STATUS_OFFSET_STAGE: u32 = 0;
    pub const STATUS_OFFSET_RX: u32 = 1;
    pub const STATUS_OFFSET_FORWARDED: u32 = 2;
    pub const STATUS_OFFSET_DROPPED: u32 = 3;
    pub const STATUS_OFFSET_UNKNOWN: u32 = 4;
    pub const STATUS_OFFSET_ROUTES: u32 = 5;
    pub const STATUS_OFFSET_MAGIC: u32 = 6;
    pub const STATUS_OFFSET_EPOCH_LO: u32 = 7;
    pub const STATUS_OFFSET_EPOCH_HI: u32 = 8;
    pub const STATUS_OFFSET_BACKENDS: u32 = 9;
    pub const STATUS_OFFSET_VIP_ADVERTISER: u32 = 10;
    pub const STATUS_OFFSET_INGRESS_LOCAL: u32 = 11;
    pub const STATUS_OFFSET_INGRESS_FORWARDED: u32 = 12;
    pub const STATUS_OFFSET_INGRESS_DROPPED: u32 = 13;
    pub const STATUS_OFFSET_FLOW_BINDINGS: u32 = 14;
    pub const STATUS_OFFSET_IS_ADVERTISER: u32 = 15;
    /// Admitted/routable members, including members draining existing flows.
    pub const STATUS_OFFSET_MEMBERS: u32 = 16;
    /// One while the latest complete snapshot remains within its monotonic
    /// new-flow admission lease.
    pub const STATUS_OFFSET_SNAPSHOT_FRESH: u32 = 17;
    /// Frames rejected because an unbound flow arrived after lease expiry.
    pub const STATUS_OFFSET_SNAPSHOT_STALE_DROPPED: u32 = 18;
    /// Frames rejected because their pinned epoch left snapshot history.
    pub const STATUS_OFFSET_MISSING_EPOCH_DROPPED: u32 = 19;
    /// Number of transitions from a fresh snapshot lease to fail-closed state.
    pub const STATUS_OFFSET_SNAPSHOT_EXPIRATIONS: u32 = 20;
    /// Number of independently assigned IPv4/TCP service identities.
    pub const STATUS_OFFSET_SERVICE_COUNT: u32 = 21;
    pub const STATUS_WORDS: usize = 22;
    pub const STATUS_MAGIC: u32 = 0x4652_5453;
}

/// Persistent object store protocol (`charlotte-protocol-objstore` v1).
pub mod objstore {
    pub const INTERFACE: u64 = super::name(b"OBJSTR ");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"obj");
    pub const TEST_DONE_NAME: u64 = super::name(b"objdone");

    pub const OP_CREATE: u32 = 1;
    pub const OP_DELETE: u32 = 2;
    pub const OP_WRITE: u32 = 3;
    pub const OP_READ: u32 = 4;
    pub const OP_RESIZE: u32 = 5;
    pub const OP_FLUSH: u32 = 6;
    pub const OP_INFO: u32 = 7;
    pub const OP_CREATE_AT: u32 = 8;
    pub const OP_SET_SIZE: u32 = 9;
    /// Like [`OP_CREATE_AT`], but marks the object shared-read: any domain
    /// authenticated by the kernel may read it. Used for the node identity,
    /// which discovery, DNS and the Raft fixture all need.
    pub const OP_CREATE_AT_SHARED: u32 = 10;
    /// Install an operator scope table (mirrors the protocol constant).
    pub const OP_APPLY_SCOPE: u32 = 11;

    /// Reserved object holding the persisted operator scope table.
    pub const SCOPE_OBJECT_ID: u64 = 0xffff_0000_0000_0002;
    /// Scope grant bits.
    pub const SCOPE_RIGHT_READ: u32 = 1;
    pub const SCOPE_RIGHT_WRITE: u32 = 2;
    pub const SCOPE_MAGIC: u32 = 0x5043_534f; // "OSCP" LE
    pub const SCOPE_VERSION: u16 = 1;
    pub const SCOPE_HEADER_LEN: usize = 16;
    pub const SCOPE_ENTRY_LEN: usize = 32;
    pub const SCOPE_MAX_ENTRIES: usize = 64;
    pub const SCOPE_MAX_LEN: usize = SCOPE_HEADER_LEN + SCOPE_MAX_ENTRIES * SCOPE_ENTRY_LEN;

    pub const ERR_OK: i64 = 0;
    pub const ERR_NOT_FOUND: i64 = 1;
    pub const ERR_NO_SPACE: i64 = 2;
    pub const ERR_INVALID_ID: i64 = 3;
    pub const ERR_IO_ERROR: i64 = 4;
    pub const ERR_EXISTS: i64 = 5;
    pub const ERR_TOO_LARGE: i64 = 6;
    pub const ERR_DENIED: i64 = 7;
    pub const EXECUTABLE_ECHO_ID: u64 = 0xffff_0000_0000_0001;

    /// Automatic IDs are positive scalar replies, disjoint from high-range
    /// stable service IDs and the reserved artifact/executable namespaces.
    pub const MAX_AUTOMATIC_ID: u64 = i64::MAX as u64;

    /// Choose an unused ID with bounded work. A directory with N slots can
    /// contain at most N occupied candidates; N + 1 probes suffice when a
    /// free slot exists. Explicit creation does not advance this counter.
    pub fn next_automatic_id(
        next: &mut u64,
        attempts: usize,
        mut occupied: impl FnMut(u64) -> bool,
    ) -> Result<u64, i64> {
        for candidate in ((*next).max(1)..=MAX_AUTOMATIC_ID).take(attempts) {
            if !occupied(candidate) {
                *next = candidate + 1;
                return Ok(candidate);
            }
        }
        Err(ERR_NO_SPACE)
    }

    /// Preserve explicit-ID existence errors even when the directory is full.
    /// No automatic counter changes or candidate probes occur without a slot.
    pub fn admit_create_id(
        requested: Option<u64>,
        next: &mut u64,
        attempts: usize,
        slot_available: bool,
        mut occupied: impl FnMut(u64) -> bool,
    ) -> Result<u64, i64> {
        if let Some(id) = requested {
            if id == 0 {
                return Err(ERR_INVALID_ID);
            }
            if occupied(id) {
                return Err(ERR_EXISTS);
            }
            return if slot_available {
                Ok(id)
            } else {
                Err(ERR_NO_SPACE)
            };
        }
        if !slot_available {
            return Err(ERR_NO_SPACE);
        }
        next_automatic_id(next, attempts, occupied)
    }

    /// Resolve one existing locator/header. Only an absent locator is
    /// NotFound; any header I/O or validation failure is an error. Return the
    /// same snapshot for authorization and data/size processing, without retry.
    pub fn resolve_header<E: Copy, H>(
        entry: Option<E>,
        read: impl FnOnce(E) -> Option<H>,
    ) -> Result<(E, H), i64> {
        let entry = entry.ok_or(ERR_NOT_FOUND)?;
        let header = read(entry).ok_or(ERR_IO_ERROR)?;
        Ok((entry, header))
    }

    /// Kernel envelope principal for `KERNEL_ASID` senders
    /// (`ipc` sets it to 1; loaded user domains never use zero or one).
    const KERNEL_PRINCIPAL: u64 = 1;

    /// Object is readable by every kernel-authenticated domain, including
    /// non-owners. Set at creation and preserved across rewrites.
    pub const ATTR_SHARED_READ: u32 = 1;

    /// Whether `id` names a reserved artifact or executable object. Both
    /// namespaces are partitioned top-down so ordinary objects can never
    /// collide with them.
    pub const fn is_reserved_object(id: u64) -> bool {
        id >> 48 >= 0xfffe
    }

    /// Reserved objects are mutated only by the kernel (seeding and tests)
    /// and the deployment administration service; every other principal is
    /// denied even though it may hold a store connection.
    pub fn reserved_mutator(principal: u64) -> bool {
        principal == KERNEL_PRINCIPAL
            || principal == charlotte_launch::artifact_principal_id(b"clusterctl")
    }

    /// Whether `principal` may create `id`. Creation of a new ordinary object
    /// is open; reserved artifact/executable IDs require a reserved mutator.
    pub fn may_create(id: u64, principal: u64) -> bool {
        principal != 0 && (!is_reserved_object(id) || reserved_mutator(principal))
    }

    /// Every stored object has an explicit nonzero creating principal.
    /// Reserved mutation additionally requires kernel/administration authority.
    pub fn may_mutate(id: u64, owner: u64, principal: u64) -> bool {
        owner != 0
            && principal != 0
            && if is_reserved_object(id) {
                reserved_mutator(principal)
            } else {
                principal == KERNEL_PRINCIPAL || owner == principal
            }
    }

    /// Reserved objects are public to authenticated callers; ordinary objects
    /// require ownership, kernel authority or an explicit shared-read attribute.
    pub fn may_read(id: u64, owner: u64, attributes: u32, principal: u64) -> bool {
        owner != 0
            && principal != 0
            && (is_reserved_object(id)
                || attributes & ATTR_SHARED_READ != 0
                || owner == principal
                || principal == KERNEL_PRINCIPAL)
    }

    /// Scope grants extend ordinary authority, never missing ownership or
    /// the reserved mutation allowlist.
    pub fn may_mutate_scoped(id: u64, owner: u64, principal: u64, scope_write: bool) -> bool {
        owner != 0
            && principal != 0
            && (may_mutate(id, owner, principal) || (!is_reserved_object(id) && scope_write))
    }

    pub fn may_read_scoped(
        id: u64,
        owner: u64,
        attributes: u32,
        principal: u64,
        scope_read: bool,
    ) -> bool {
        owner != 0 && principal != 0 && (may_read(id, owner, attributes, principal) || scope_read)
    }

    /// One operator scope grant: `principal` may exercise `rights`
    /// (read/write bits) over the inclusive object-id range `first..=last`.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct ScopeEntry {
        pub principal: u64,
        pub first: u64,
        pub last: u64,
        pub rights: u32,
    }

    /// Bounded operator scope table. Applied by the kernel or the signed
    /// administration service through `OP_APPLY_SCOPE`; the raw bytes are
    /// persisted in a reserved object and re-applied at mount.
    #[derive(Clone, Debug)]
    pub struct ScopeTable {
        entries: [ScopeEntry; SCOPE_MAX_ENTRIES],
        len: usize,
    }

    impl Default for ScopeTable {
        fn default() -> Self {
            Self {
                entries: [ScopeEntry::default(); SCOPE_MAX_ENTRIES],
                len: 0,
            }
        }
    }

    impl ScopeTable {
        pub fn len(&self) -> usize {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        /// Strict decoder for the persisted or attached wire table. Rejects a
        /// bad magic, version, reserved padding, over-count, truncated or
        /// padded length, zero principal, inverted range or unknown right.
        pub fn parse(bytes: &[u8]) -> Option<Self> {
            if bytes.len() < SCOPE_HEADER_LEN || bytes.len() > SCOPE_MAX_LEN {
                return None;
            }
            let magic = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
            let version = u16::from_le_bytes(bytes.get(4..6)?.try_into().ok()?);
            let count = u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?) as usize;
            let reserved = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
            if magic != SCOPE_MAGIC
                || version != SCOPE_VERSION
                || reserved != 0
                || count > SCOPE_MAX_ENTRIES
                || bytes.len() != SCOPE_HEADER_LEN + count * SCOPE_ENTRY_LEN
            {
                return None;
            }
            let mut table = Self::default();
            for index in 0..count {
                let offset = SCOPE_HEADER_LEN + index * SCOPE_ENTRY_LEN;
                let spin = bytes.get(offset..offset + SCOPE_ENTRY_LEN)?;
                let entry = ScopeEntry {
                    principal: u64::from_le_bytes(spin.get(0..8)?.try_into().ok()?),
                    first: u64::from_le_bytes(spin.get(8..16)?.try_into().ok()?),
                    last: u64::from_le_bytes(spin.get(16..24)?.try_into().ok()?),
                    rights: u32::from_le_bytes(spin.get(24..28)?.try_into().ok()?),
                };
                let tail = u32::from_le_bytes(spin.get(28..32)?.try_into().ok()?);
                if entry.principal == 0
                    || entry.first > entry.last
                    || entry.rights == 0
                    || entry.rights & !(SCOPE_RIGHT_READ | SCOPE_RIGHT_WRITE) != 0
                    || tail != 0
                {
                    return None;
                }
                table.entries[index] = entry;
            }
            table.len = count;
            Some(table)
        }

        /// Canonical encoding of the applied table (used by fixtures and by
        /// the store when persisting an accepted table).
        pub fn encode(&self) -> alloc::vec::Vec<u8> {
            let mut bytes = alloc::vec![0u8; SCOPE_HEADER_LEN + self.len * SCOPE_ENTRY_LEN];
            bytes[0..4].copy_from_slice(&SCOPE_MAGIC.to_le_bytes());
            bytes[4..6].copy_from_slice(&SCOPE_VERSION.to_le_bytes());
            bytes[6..8].copy_from_slice(&(self.len as u16).to_le_bytes());
            for (index, entry) in self.entries[..self.len].iter().enumerate() {
                let offset = SCOPE_HEADER_LEN + index * SCOPE_ENTRY_LEN;
                bytes[offset..offset + 8].copy_from_slice(&entry.principal.to_le_bytes());
                bytes[offset + 8..offset + 16].copy_from_slice(&entry.first.to_le_bytes());
                bytes[offset + 16..offset + 24].copy_from_slice(&entry.last.to_le_bytes());
                bytes[offset + 24..offset + 28].copy_from_slice(&entry.rights.to_le_bytes());
            }
            bytes
        }

        pub fn allows(&self, principal: u64, id: u64, right: u32) -> bool {
            self.entries[..self.len].iter().any(|entry| {
                entry.principal == principal
                    && entry.first <= id
                    && id <= entry.last
                    && entry.rights & right != 0
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn clusterctl() -> u64 {
            charlotte_launch::artifact_principal_id(b"clusterctl")
        }
        fn ordinary() -> u64 {
            charlotte_launch::artifact_principal_id(b"greet")
        }
        fn other() -> u64 {
            charlotte_launch::artifact_principal_id(b"echo")
        }

        #[test]
        fn metadata_failure_never_becomes_public_ownership_or_retries() {
            use core::cell::Cell;
            let reads = Cell::new(0);
            for _opcode in [OP_READ, OP_INFO] {
                let snapshot = resolve_header(Some(42), |_| {
                    reads.set(reads.get() + 1);
                    None::<(u64, u32, u64)>
                });
                assert_eq!(snapshot, Err(ERR_IO_ERROR));
            }
            assert_eq!(reads.get(), 2);
            let missing =
                resolve_header(None::<u64>, |_| panic!("a missing locator must not read metadata"));
            assert_eq!(missing, Err::<(u64, ()), _>(ERR_NOT_FOUND));
            let (_, (owner, attributes, size)) =
                resolve_header(Some(42), |_| Some((ordinary(), 0, 123))).unwrap();
            assert_eq!(size, 123);
            assert!(!may_read_scoped(42, owner, attributes, other(), false));
            assert!(may_read_scoped(42, owner, attributes, ordinary(), false));
            // Missing ownership cannot manufacture public authority.
            let (_, (owner, attributes)) = resolve_header(Some(42), |_| Some((0, 0))).unwrap();
            assert!(!may_read_scoped(42, owner, attributes, other(), false));
        }

        #[test]
        fn automatic_ids_preserve_explicit_foreign_objects_across_restart_and_delete() {
            use alloc::collections::BTreeMap;
            let mut owners = BTreeMap::from([(1, ordinary()), (2, ordinary())]);
            let mut next = 1;
            let chosen = next_automatic_id(&mut next, 4, |id| owners.contains_key(&id)).unwrap();
            assert_eq!(chosen, 3);
            assert_eq!(owners.insert(chosen, other()), None);
            assert_eq!(owners.get(&1), Some(&ordinary()));
            assert_eq!(owners.get(&2), Some(&ordinary()));
            // Mount restores this persisted counter; a later explicit ID
            // must still be skipped, and deleting an old ID cannot rewind it.
            let mut restored_next = next;
            owners.insert(4, ordinary());
            owners.remove(&2);
            let chosen =
                next_automatic_id(&mut restored_next, 5, |id| owners.contains_key(&id)).unwrap();
            assert_eq!(chosen, 5);
            assert_eq!(owners.insert(chosen, other()), None);
            assert_eq!(owners.get(&1), Some(&ordinary()));
            assert_eq!(owners.get(&4), Some(&ordinary()));
        }

        #[test]
        fn full_directory_preserves_existing_ids_and_rejects_without_advancing() {
            let mut next = 1;
            assert_eq!(
                admit_create_id(Some(1), &mut next, 2, false, |id| id == 1),
                Err(ERR_EXISTS)
            );
            assert_eq!(
                admit_create_id(Some(0), &mut next, 2, false, |_| panic!("invalid ID")),
                Err(ERR_INVALID_ID)
            );
            assert_eq!(
                admit_create_id(Some(2), &mut next, 2, false, |id| id == 1),
                Err(ERR_NO_SPACE)
            );
            assert_eq!(
                admit_create_id(None, &mut next, 2, false, |_| panic!("no free slot")),
                Err(ERR_NO_SPACE)
            );
            assert_eq!(next, 1);
            assert_eq!(admit_create_id(None, &mut next, 2, true, |id| id == 1), Ok(2));
            assert_eq!(next, 3);
        }

        #[test]
        fn automatic_id_search_is_bounded_and_excludes_reserved_and_negative_replies() {
            let mut next = 1;
            let mut probes = 0;
            assert_eq!(
                next_automatic_id(&mut next, 3, |_| {
                    probes += 1;
                    true
                }),
                Err(ERR_NO_SPACE)
            );
            assert_eq!(probes, 3);
            assert_eq!(next, 1);
            next = MAX_AUTOMATIC_ID;
            assert_eq!(next_automatic_id(&mut next, 2, |_| false), Ok(MAX_AUTOMATIC_ID));
            assert_eq!(
                next_automatic_id(&mut next, 2, |_| panic!("outside automatic range")),
                Err(ERR_NO_SPACE)
            );
            next = EXECUTABLE_ECHO_ID;
            assert_eq!(
                next_automatic_id(&mut next, 2, |_| panic!("reserved range")),
                Err(ERR_NO_SPACE)
            );
        }

        #[test]
        fn reserved_namespaces_cover_artifacts_and_executables_only() {
            assert!(is_reserved_object(charlotte_launch::ARTIFACT_ID_TAG | 1));
            assert!(is_reserved_object(EXECUTABLE_ECHO_ID));
            assert!(is_reserved_object(0xffff_ffff_ffff_ffff));
            assert!(!is_reserved_object(0));
            assert!(!is_reserved_object(1));
            assert!(!is_reserved_object(0xfffd_ffff_ffff_ffff));
            assert!(!is_reserved_object(0x7fff_ffff_ffff_ff00));
        }

        #[test]
        fn reserved_mutation_requires_kernel_or_administration() {
            assert!(may_mutate(
                charlotte_launch::ARTIFACT_ID_TAG | 7,
                KERNEL_PRINCIPAL,
                KERNEL_PRINCIPAL
            ));
            assert!(may_mutate(EXECUTABLE_ECHO_ID, KERNEL_PRINCIPAL, clusterctl()));
            assert!(!may_mutate(EXECUTABLE_ECHO_ID, KERNEL_PRINCIPAL, ordinary()));
            assert!(!may_mutate(EXECUTABLE_ECHO_ID, KERNEL_PRINCIPAL, 0));
            // Creation of ordinary IDs is open; reserved IDs are not.
            assert!(may_create(42, ordinary()));
            assert!(!may_create(42, 0));
            assert!(may_create(EXECUTABLE_ECHO_ID, clusterctl()));
            assert!(!may_create(EXECUTABLE_ECHO_ID, ordinary()));
        }

        #[test]
        fn owned_ordinary_objects_are_private_to_their_owner() {
            assert!(may_mutate(42, ordinary(), ordinary()));
            assert!(may_mutate(42, ordinary(), KERNEL_PRINCIPAL));
            assert!(!may_mutate(42, ordinary(), other()));
            assert!(!may_mutate(42, ordinary(), 0));
            // Invalid ownership rejects even kernel/scope/shared-read shortcuts.
            assert!(!may_read(42, 0, 0, other()));
            assert!(!may_read(42, 0, ATTR_SHARED_READ, other()));
            assert!(!may_read_scoped(42, 0, 0, other(), true));
            assert!(!may_mutate_scoped(42, 0, other(), true));
            assert!(!may_mutate(42, 0, other()));
            assert!(!may_mutate(42, 0, KERNEL_PRINCIPAL));
        }

        #[test]
        fn reads_follow_owner_shared_and_reserved_policy() {
            assert!(may_read(42, ordinary(), 0, ordinary()));
            assert!(may_read(42, ordinary(), 0, KERNEL_PRINCIPAL));
            assert!(!may_read(42, ordinary(), 0, other()));
            assert!(may_read(42, ordinary(), ATTR_SHARED_READ, other()));
            assert!(may_read(EXECUTABLE_ECHO_ID, ordinary(), 0, other()));
        }

        fn encoded_table(entries: &[(u64, u64, u64, u32)]) -> alloc::vec::Vec<u8> {
            let mut bytes = alloc::vec![0u8; SCOPE_HEADER_LEN + entries.len() * SCOPE_ENTRY_LEN];
            bytes[0..4].copy_from_slice(&SCOPE_MAGIC.to_le_bytes());
            bytes[4..6].copy_from_slice(&SCOPE_VERSION.to_le_bytes());
            bytes[6..8].copy_from_slice(&(entries.len() as u16).to_le_bytes());
            for (index, (principal, first, last, rights)) in entries.iter().enumerate() {
                let offset = SCOPE_HEADER_LEN + index * SCOPE_ENTRY_LEN;
                bytes[offset..offset + 8].copy_from_slice(&principal.to_le_bytes());
                bytes[offset + 8..offset + 16].copy_from_slice(&first.to_le_bytes());
                bytes[offset + 16..offset + 24].copy_from_slice(&last.to_le_bytes());
                bytes[offset + 24..offset + 28].copy_from_slice(&rights.to_le_bytes());
            }
            bytes
        }

        #[test]
        fn scope_table_grants_rights_within_inclusive_ranges() {
            let bytes = encoded_table(&[
                (ordinary(), 100, 199, SCOPE_RIGHT_READ),
                (other(), 100, 199, SCOPE_RIGHT_READ | SCOPE_RIGHT_WRITE),
            ]);
            let table = ScopeTable::parse(&bytes).unwrap();
            assert_eq!(table.len(), 2);
            assert!(table.allows(ordinary(), 100, SCOPE_RIGHT_READ));
            assert!(table.allows(ordinary(), 199, SCOPE_RIGHT_READ));
            assert!(!table.allows(ordinary(), 99, SCOPE_RIGHT_READ));
            assert!(!table.allows(ordinary(), 200, SCOPE_RIGHT_READ));
            assert!(!table.allows(ordinary(), 100, SCOPE_RIGHT_WRITE));
            assert!(table.allows(other(), 150, SCOPE_RIGHT_WRITE));
            assert!(!table.allows(other(), 150, 8));
            assert_eq!(table.encode(), bytes);
        }

        #[test]
        fn scope_table_rejects_malformed_tables() {
            let valid = encoded_table(&[(ordinary(), 1, 2, SCOPE_RIGHT_READ)]);
            assert!(ScopeTable::parse(&valid).is_some());
            let mut bad_magic = valid.clone();
            bad_magic[0] ^= 0xff;
            assert!(ScopeTable::parse(&bad_magic).is_none());
            let mut bad_version = valid.clone();
            bad_version[4] = 2;
            assert!(ScopeTable::parse(&bad_version).is_none());
            let mut bad_padding = valid.clone();
            bad_padding[8] = 1;
            assert!(ScopeTable::parse(&bad_padding).is_none());
            assert!(ScopeTable::parse(&valid[..valid.len() - 1]).is_none());
            let mut zero_principal = valid.clone();
            zero_principal[SCOPE_HEADER_LEN..SCOPE_HEADER_LEN + 8].fill(0);
            assert!(ScopeTable::parse(&zero_principal).is_none());
            let mut inverted = valid.clone();
            inverted[SCOPE_HEADER_LEN + 8..SCOPE_HEADER_LEN + 16].fill(9);
            assert!(ScopeTable::parse(&inverted).is_none());
            let mut unknown_right = valid.clone();
            unknown_right[SCOPE_HEADER_LEN + 24] = 4;
            assert!(ScopeTable::parse(&unknown_right).is_none());
            let mut entry_tail = valid;
            entry_tail[SCOPE_HEADER_LEN + 28] = 1;
            assert!(ScopeTable::parse(&entry_tail).is_none());
        }

        #[test]
        fn scope_grants_never_extend_the_reserved_allowlist() {
            assert!(may_mutate_scoped(42, ordinary(), other(), true));
            assert!(may_mutate_scoped(42, ordinary(), other(), false) == false);
            assert!(!may_mutate_scoped(EXECUTABLE_ECHO_ID, 0, other(), true));
            assert!(may_read_scoped(42, ordinary(), 0, other(), true));
            assert!(!may_read_scoped(42, ordinary(), 0, other(), false));
        }
    }
}

/// Native filesystem protocol (`charlotte-protocol-fs` v1).
pub mod fs {
    pub const INTERFACE: u64 = super::name(b"FFS");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"fs");
    pub const OP_SET_SIZE: u32 = 8;

    pub const OP_LOOKUP: u32 = 1;
    pub const OP_CREATE: u32 = 2;
    pub const OP_READ: u32 = 3;
    pub const OP_WRITE: u32 = 4;
    pub const OP_DELETE: u32 = 5;
    pub const OP_LIST: u32 = 6;
    pub const OP_FLUSH: u32 = 7;

    pub const FLAG_DIR: u32 = 1 << 0;

    pub const ERR_OK: i64 = 0;
    pub const ERR_NOT_FOUND: i64 = 1;
    pub const ERR_EXISTS: i64 = 2;
    pub const ERR_NO_SPACE: i64 = 3;
    pub const ERR_IO_ERROR: i64 = 4;
    pub const ERR_NOT_DIR: i64 = 5;
    pub const ERR_DIR_NOT_EMPTY: i64 = 6;
}

/// Reliable Message Layer protocol (`charlotte-protocol-relmsg` v3).
///
/// Exposes sequenced, acknowledged, retransmitted message delivery.
/// Clients send messages addressed to peer service names; the RML
/// encapsulates them in `charlotte-protocol-msg` frames and delivers
/// them via the NIC driver (or directly for same-machine peers).
pub mod relmsg {
    pub const INTERFACE: u64 = super::name(b"RELMSG");
    pub const VERSION: u32 = 3;
    pub const NAME: u64 = super::name(b"relmsg");

    pub const OP_SEND: u32 = 1;
    pub const OP_RECV: u32 = 2;
    pub const OP_STATUS: u32 = 3;
    /// Internal ingress used by the isolated NIC-receive pump.
    pub const OP_FRAME: u32 = 4;
    pub const OP_SHUTDOWN: u32 = 5;
    /// Richer transport diagnostics. Reply moves a page holding the packed
    /// `RelmsgDiag` snapshot (`crate::relmsg::DIAG_*` layout). Unlike
    /// [`OP_STATUS`] (a scalar carrying only the local MAC), this reports the
    /// live counters the transport maintains for retransmits, send failures,
    /// deliveries, and peer load.
    pub const OP_DIAG: u32 = 6;

    /// `RelmsgDiag` snapshot layout (little-endian u32 words in a moved page).
    pub const DIAG_OFFSET_MAGIC: u32 = 0;
    pub const DIAG_OFFSET_PEERS: u32 = 1;
    pub const DIAG_OFFSET_HANDLED: u32 = 2;
    pub const DIAG_OFFSET_RETRANSMITS: u32 = 3;
    pub const DIAG_OFFSET_SEND_FAILURES: u32 = 4;
    pub const DIAG_OFFSET_RECEIVED: u32 = 5;
    pub const DIAG_OFFSET_IN_FLIGHT: u32 = 6;
    pub const DIAG_WORDS: usize = 7;
    /// `"RMLD"` LE.
    pub const DIAG_MAGIC: u32 = 0x444c_4d52;

    pub const ERR_PEER_UNREACHABLE: i64 = -1;
    pub const ERR_BAD_OPCODE: i64 = -2;
    pub const ERR_UNKNOWN: i64 = -3;
    pub const ERR_BUSY: i64 = -4;

    pub const MAX_PEERS: usize = 16;
    /// Initial operational ceiling for an application payload. The v3 wire
    /// and IPC formats represent lengths with `u32`; this lower policy bound
    /// limits allocation, buffering, and head-of-line blocking independently
    /// of that representation and can be raised without a protocol revision.
    pub const MAX_MSG: usize = 1024 * 1024;
    pub const RETRANSMIT_MS: u64 = 200;
    /// Number of retransmission rounds before an uncertain send is abandoned.
    /// The wait between rounds scales with the retained fragment count (up to
    /// [`MAX_RETRY_DELAY_TICKS`]), so a one-frame RPC retains its 200 ms retry
    /// while a large message is not resent wholesale before the receiver has
    /// had a bounded opportunity to drain its first transmission.
    pub const MAX_RETRIES: u32 = 10;
    pub const MAX_RETRY_DELAY_TICKS: u32 = 25;
}

/// Receive-side reassembly for the reliable message layer.
///
/// The advertised total is remote-controlled, so admission precedes any state
/// creation or allocation, and assembly only ever sees an admitted total.
pub mod relmsg_reassembly {
    use alloc::{
        collections::BTreeMap,
        vec::Vec,
    };

    #[derive(Debug)]
    pub struct Reassembly {
        pub seq: u32,
        /// Fragment offset -> payload bytes, deduplicated by offset.
        pub fragments: BTreeMap<usize, Vec<u8>>,
        pub buffered_bytes: usize,
        /// Total message length advertised consistently by every fragment.
        pub total: usize,
    }

    impl Reassembly {
        pub fn new(seq: u32, total: usize) -> Self {
            Self {
                seq,
                fragments: BTreeMap::new(),
                buffered_bytes: 0,
                total,
            }
        }
    }

    /// Whether a fragment may create or update reassembly state. The fragment
    /// must lie inside an advertised total that is within `max`.
    pub fn fragment_within_ceiling(
        total: usize,
        offset: usize,
        payload_len: usize,
        max: usize,
    ) -> bool {
        total <= max && offset.checked_add(payload_len).is_some_and(|end| end <= total)
    }

    /// Assemble the buffered fragments into one contiguous message, or `None`
    /// if a fragment is missing or the assembly cannot be reserved. `total`
    /// must have been admitted with [`fragment_within_ceiling`].
    pub fn assemble(reassembly: &Reassembly, max: usize) -> Option<Vec<u8>> {
        let total = reassembly.total;
        if total > max {
            return None;
        }
        let mut out = Vec::new();
        out.try_reserve_exact(total).ok()?;
        let mut offset = 0usize;
        while offset < total {
            let bytes = reassembly.fragments.get(&offset)?;
            out.extend_from_slice(bytes);
            offset += bytes.len();
        }
        (offset == total).then_some(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const MAX: usize = super::super::relmsg::MAX_MSG;

        #[test]
        fn oversized_totals_are_never_admitted() {
            assert!(!fragment_within_ceiling(u32::MAX as usize, 0, 1, MAX));
            assert!(!fragment_within_ceiling(MAX + 1, 0, 1, MAX));
            assert!(!fragment_within_ceiling(16, 8, 9, MAX));
            assert!(fragment_within_ceiling(MAX, 0, 1, MAX));
            assert!(fragment_within_ceiling(16, 8, 8, MAX));
        }

        #[test]
        fn assembly_requires_an_admitted_total_and_every_fragment() {
            let mut reassembly = Reassembly::new(1, 4);
            reassembly.fragments.insert(0, alloc::vec![1, 2]);
            reassembly.fragments.insert(2, alloc::vec![3, 4]);
            assert_eq!(assemble(&reassembly, 4), Some(alloc::vec![1, 2, 3, 4]));
            reassembly.fragments.remove(&2);
            assert_eq!(assemble(&reassembly, 4), None);
            let oversized = Reassembly::new(1, 0xffff_ffff);
            assert_eq!(assemble(&oversized, MAX), None);
        }
    }
}

/// Cluster discovery protocol (`charlotte-protocol-disco` v1).
///
/// Nodes broadcast probes on EtherType `0x88B6`; peers reply with unicast
/// responses carrying node identity and service-registration information.
/// Reliability comes from probe retransmission rather than sequenced ACKs,
/// matching the pattern used by mDNS, SSDP, and LLDP.
pub mod disco {
    pub const INTERFACE: u64 = super::name(b"DISC O");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"disco");

    pub const OP_PROBE: u32 = 1;
    pub const OP_LIST_PEERS: u32 = 2;
    pub const OP_STATUS: u32 = 3;
    pub const OP_SHUTDOWN: u32 = 4;
    /// Cluster-location query: see `charlotte-protocol-disco`'s
    /// `OP_CLUSTER_STATUS` for the reply layout.
    pub const OP_CLUSTER_STATUS: u32 = 6;
    /// `OP_CLUSTER_STATUS` arg0 flag: retain the call until the local Raft
    /// identity and at least one remote Raft leader are known.
    pub const CLUSTER_STATUS_WAIT_READY: u64 = 1;
    /// Internal ingress used by the frame demultiplexer (frouter): delivers
    /// one raw frame whose EtherType matched `DISCO_ETHERTYPE`. The call
    /// attaches a **moved** memory object holding the frame; the service
    /// processes it and replies 0.
    pub const OP_FRAME: u32 = 5;

    /// Richer discovery diagnostics. Reply moves a page holding the packed
    /// `DiscoDiag` snapshot (`crate::disco::DIAG_*` layout): the live probe
    /// traffic counters, cluster posture, and heartbeat, which are not
    /// carried by the scalar [`OP_STATUS`].
    pub const OP_DIAG: u32 = 7;

    /// `DiscoDiag` snapshot layout (little-endian u32 words in a moved page).
    pub const DIAG_OFFSET_MAGIC: u32 = 0;
    pub const DIAG_OFFSET_RUNNING: u32 = 1;
    pub const DIAG_OFFSET_PEERS: u32 = 2;
    pub const DIAG_OFFSET_CLUSTER_ROLE: u32 = 3;
    pub const DIAG_OFFSET_RX_RAW: u32 = 4;
    pub const DIAG_OFFSET_SENT_OK: u32 = 5;
    pub const DIAG_OFFSET_SENT_FAIL: u32 = 6;
    pub const DIAG_OFFSET_DECODED: u32 = 7;
    pub const DIAG_OFFSET_CALLED: u32 = 8;
    pub const DIAG_OFFSET_HEARTBEAT: u32 = 9;
    pub const DIAG_WORDS: usize = 10;
    /// `"DCOD"` LE.
    pub const DIAG_MAGIC: u32 = 0x444f_4344;

    pub const PROBE_COUNT: usize = 3;
    pub const PROBE_INTERVAL_MS: u64 = 200;
    pub const PEER_TTL_MS: u64 = 30_000;

    /// Maximum live discovered peers. The cluster is far smaller; the bound
    /// keeps an unauthenticated L2 peer from growing the discovery table
    /// without limit by advertising fresh MACs.
    pub const MAX_DISCOVERED_PEERS: usize = 64;

    /// Whether a learned peer may be inserted or refreshed. Known MACs refresh
    /// in place; new MACs are admitted only below the bound.
    pub const fn peer_learning_admitted(current: usize, known: bool) -> bool {
        known || current < MAX_DISCOVERED_PEERS
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn peer_learning_refuses_new_macs_at_the_bound() {
            assert!(peer_learning_admitted(0, false));
            assert!(peer_learning_admitted(MAX_DISCOVERED_PEERS - 1, false));
            assert!(!peer_learning_admitted(MAX_DISCOVERED_PEERS, false));
            assert!(!peer_learning_admitted(MAX_DISCOVERED_PEERS + 1, false));
            // Known peers can always refresh, even at capacity.
            assert!(peer_learning_admitted(MAX_DISCOVERED_PEERS, true));
        }
    }
}

/// Distributed name service (DNS interface 4).
///
/// One replica runs per node. The replicas form a Raft group whose state
/// machine is a replicated `name -> node` catalog; the node-local name
/// service still owns the actual connection capabilities. `OP_REGISTER`
/// proposes a `name -> local node` entry to the cluster (and registers the
/// connection with the local name service once committed, replying with the
/// committed distributed generation). `OP_LOOKUP`
/// answers from the replicated catalog: local names resolve to the local
/// name service, remote names report the hosting node. `OP_CALL` is the
/// remote-invocation entry point: it resolves a name through the catalog and
/// invokes it locally (when hosted here) or forwards it to the hosting node
/// over the reliable message layer.
pub mod dns {
    use alloc::vec::Vec;

    pub const INTERFACE: u64 = super::name(b"DNS ");
    pub const VERSION: u32 = charlotte_protocol_dns::VERSION;
    pub const NAME: u64 = super::name(b"dns");

    /// Register a locally published service, or a uniquely attached mintable
    /// connection. Missing local publication rejects; catalog-only events use
    /// the explicit memory-carried OP_EVENT_FIRE operation.
    pub const OP_REGISTER: u32 = 1;
    /// Quorum-fenced placement observation: RESULT_LOCAL carries no capability;
    /// RESULT_REMOTE carries the hosting node identity in moved memory. Invoke
    /// through OP_CALL or obtain authority from the appropriate local grant path.
    pub const OP_LOOKUP: u32 = 2;
    pub const OP_STATUS: u32 = 3;
    pub const OP_SHUTDOWN: u32 = 4;
    /// Full-name invocation. `arg0` is the exact moved-memory byte length.
    /// Encode the mandatory CDNSCL01 envelope with [`crate::dns_call`].
    /// Names contain 1..=256 bytes; retired packed-name/12-byte calls reject.
    /// The deferred reply is the target scalar result; accepted work that
    /// cannot be confirmed returns ERR_UNCERTAIN and must not be replayed.
    pub const OP_CALL: u32 = charlotte_protocol_dns::OP_CALL;
    /// Return the complete locally applied active catalog as a moved
    /// [`crate::dns_catalog`] record set. The scalar result is its exact byte
    /// length. A catalog above the wire bound rejects instead of returning a
    /// prefix or truncating identities. The request has no attachments/arg0.
    pub const OP_CATALOG: u32 = 6;
    /// Replicate an exact-generation tombstone for a locally hosted service.
    /// `arg0` is the packed name and the first eight bytes of the attached
    /// memory object contain the expected distributed generation. The reply
    /// is that generation on success.
    pub const OP_UNREGISTER: u32 = 7;
    /// Wait for a cluster event. `arg0` is the packed event name (the
    /// `event:` convention below). If the event has already fired — the name
    /// is present in this node's *applied* catalog — the reply is its
    /// committed generation, immediately. Otherwise the reply token is
    /// parked in a per-node waitlist and resolved when the replicated
    /// catalog gains the entry (the event is communicated through Raft
    /// consensus, like the name service's deferred lookups are through the
    /// local registry). The caller may bound the wait with a reply timeout.
    pub const OP_EVENT_WAIT: u32 = 10;
    /// Like `OP_REGISTER`, but the name travels in the moved memory object
    /// (`arg0` = byte length): the packed scalar encoding is limited to 8
    /// bytes, while event names (and long service names) exceed it.
    pub const OP_REGISTER_NAMED: u32 = 11;
    /// Fire a cluster event: commit `event:{name}` to the replicated
    /// catalog (catalog-only — no local name-service publication). `arg0` is
    /// the event-name byte length carried in the moved memory object. Only
    /// the dns leader may commit; an external firing side retries on
    /// `ERR_NOT_LEADER`. The reply is the committed generation (deferred
    /// until replicated).
    pub const OP_EVENT_FIRE: u32 = 12;
    // Opcode 15 was a redundant named deployment envelope; retired.
    /// Admit one current signed deployment descriptor in moved memory.
    /// `arg0` is the exact descriptor length. Identity, requested placement
    /// and variant content come only from these signed bytes.
    pub const OP_DEPLOY_SUBMIT: u32 = 33;
    /// Query one deployment by a full artifact name carried in moved memory.
    /// `arg0` is the name length. The reply always includes generation,
    /// signed descriptor and exact replica node/variant pins; see
    /// [`crate::name_catalog::decode_deployment_result`]. Missing sections are rejected rather
    /// than supplying singleton placement.
    pub const OP_DEPLOY_QUERY_NAMED: u32 = 16;
    /// Enumerate desired deployment names. The reply is
    /// `[count:u16] ([name_len:u8][name])*`, sorted by name. Callers query
    /// individual records with `OP_DEPLOY_QUERY_NAMED`.
    pub const OP_DEPLOY_LIST: u32 = 17;
    /// Query the rollout condition for a full artifact name carried in moved
    /// memory (`arg0` is the name length). The fixed-size reply uses
    /// [`crate::clusterctl::RolloutStatus`]. A deployment is ready only when
    /// its service name is active in the replicated catalog on the node
    /// selected by the desired deployment record.
    pub const OP_DEPLOY_ROLLOUT_NAMED: u32 = 18;
    /// Register a deployed application under its full artifact name. Moved
    /// memory is `[deployment_generation:u64][name]`; `arg0` is the complete
    /// byte length. This is agent-only in practice and binds readiness to the
    /// exact desired generation rather than a stale endpoint with the same
    /// name.
    pub const OP_REGISTER_DEPLOYMENT_NAMED: u32 = 19;
    /// Atomically admit one signed `CRELEASE` envelope. `arg0` is the byte
    /// length of the envelope in moved memory. The leader resolves every
    /// automatic node assignment and commits the complete component set in
    /// one Raft command. Success moves a complete `CRLADM01` admission receipt;
    /// its result is the byte length. Errors are negative scalar results.
    pub const OP_DEPLOY_RELEASE: u32 = 20;
    /// Admit one fully verified `COPSBND2` operational bundle. `arg0` is the
    /// byte length in moved memory. A follower relays the complete signed
    /// proof; the leader re-verifies both authorities and trusted UTC before
    /// committing only compact references and replay fences.
    pub const OP_DEPLOY_OPERATIONS: u32 = 21;
    /// Enumerate active compact operational bindings from locally applied
    /// Raft state. The reply is a bounded `COPSLST1` record set; it contains
    /// object keys and authenticated digests, never ciphertext or plaintext.
    pub const OP_OPERATIONAL_LIST: u32 = 22;
    /// Query the exact signed `CRELEASE` envelope by release name carried in
    /// moved memory. This lets the privileged launch gate prove that a target
    /// descriptor belongs to the release named by a compact binding.
    pub const OP_RELEASE_QUERY_NAMED: u32 = 23;
    /// Admit one operator-signed, node-targeted `CSHUTDN1` shutdown intent.
    /// The leader verifies trusted UTC and the cluster signature before the
    /// exact envelope is replicated; followers relay the bounded envelope.
    pub const OP_SHUTDOWN_SUBMIT: u32 = 24;
    /// Query the latest locally applied shutdown intent for `arg0` = node
    /// key. The reply moves `[generation:u64][CSHUTDN1 envelope]`.
    pub const OP_SHUTDOWN_QUERY: u32 = 25;
    /// Return the locally applied committed ingress snapshot. `arg0` is the
    /// compact [`charlotte_launch::ingress::ServiceId::pack`] identity. Its routable
    /// member set comes from Raft membership; when launch policy binds a VIP
    /// to an application name, new-flow eligibility also requires matching
    /// committed placement and exact-generation readiness.
    /// The moved reply is encoded by [`crate::cluster_ingress::BackendSnapshot`].
    /// This is a local materialization query; it never starts consensus and
    /// therefore remains off the packet hot path.
    pub const OP_INGRESS_MEMBERSHIP: u32 = 26;
    /// Raw Ethernet ingress for the DNS-owned Raft transport. The frouter
    /// routes only `raft::ETHERTYPE` frames here; join and application control
    /// traffic continue to use relmsg.
    pub const OP_RAFT_FRAME: u32 = 27;
    /// Resolve all launch-authorized external identities assigned to one
    /// artifact/service name. The moved request contains the complete name
    /// (`arg0` = length); the reply is a filtered canonical
    /// [`charlotte_launch::ingress`] table.
    pub const OP_INGRESS_ASSIGNMENTS_NAMED: u32 = 28;
    /// Admit a complete operator-signed `CINGPOL1` assignment replacement.
    /// Followers relay the envelope and the leader re-verifies operations
    /// authority, cluster identity and trusted UTC before Raft submission.
    pub const OP_INGRESS_POLICY_SUBMIT: u32 = 29;
    /// Return `[generation:u64][CINGPOL1 envelope]` for the committed policy.
    pub const OP_INGRESS_POLICY_QUERY: u32 = 30;
    /// Return the effective canonical assignment table: committed policy when
    /// present, otherwise the launch-time bootstrap table.
    pub const OP_INGRESS_ASSIGNMENTS: u32 = 31;
    /// Return a bounded, versioned snapshot of locally applied cluster state.
    /// The producer answers only while its applied state satisfies the same
    /// freshness bound used by ingress; callers decode the moved reply with
    /// [`crate::cluster_observe`]. This is an observability read and never
    /// starts consensus.
    pub const OP_CLUSTER_SNAPSHOT: u32 = 32;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum IngressAssignmentError {
        InvalidName,
        Ipc(catten_rt::owned::IpcError),
        Memory(catten_rt::owned::MemoryError),
        Service(i64),
        InvalidReply,
    }

    /// Resolve the operations-owned cluster identities assigned to an exact
    /// deployed artifact name.
    ///
    /// The caller needs a suitably attenuated DNS connection. Request and
    /// reply memory remain linearly owned throughout the operation; the
    /// returned values contain no live capabilities or borrowed mappings.
    pub fn ingress_assignments(
        connection: catten_rt::owned::ConnectionRef<'_>,
        artifact_name: &[u8],
    ) -> Result<Vec<charlotte_launch::ingress::ServiceId>, IngressAssignmentError> {
        if !charlotte_launch::deployment::valid_artifact_name(artifact_name) {
            return Err(IngressAssignmentError::InvalidName);
        }
        let request =
            super::stage_name_owned(artifact_name).ok_or(IngressAssignmentError::InvalidName)?;
        let result = connection
            .call_move(OP_INGRESS_ASSIGNMENTS_NAMED, artifact_name.len() as u64, request)
            .map_err(|(_, error)| IngressAssignmentError::Ipc(error))?
            .wait()
            .map_err(IngressAssignmentError::Ipc)?;
        if result.result < 0 {
            return Err(IngressAssignmentError::Service(result.result));
        }
        if result.connection.is_some() {
            return Err(IngressAssignmentError::InvalidReply);
        }
        let memory = result.memory.ok_or(IngressAssignmentError::InvalidReply)?;
        let len =
            usize::try_from(result.result).map_err(|_| IngressAssignmentError::InvalidReply)?;
        if len > memory.len() {
            return Err(IngressAssignmentError::InvalidReply);
        }
        let mapping =
            memory.map_read_only().map_err(|(_, error)| IngressAssignmentError::Memory(error))?;
        let assignments = charlotte_launch::ingress::decode(&mapping.as_slice()[..len])
            .ok_or(IngressAssignmentError::InvalidReply)?
            .map(|binding| binding.service)
            .collect();
        let memory = mapping.unmap().map_err(|(_, error)| IngressAssignmentError::Memory(error))?;
        drop(memory);
        Ok(assignments)
    }

    /// Event-name prefix: events are ordinary replicated catalog names so the
    /// existing register/commit/replicate machinery fires them; the prefix
    /// keeps them distinct from service registrations.
    pub const EVENT_PREFIX: &[u8] = b"event:";

    /// Build a packed event name for `OP_EVENT_WAIT` into `buf`. Returns the
    /// written length, or `None` if the event name is empty/too long.
    pub fn pack_event_name(buf: &mut [u8], event: &[u8]) -> Option<usize> {
        if event.is_empty() || event.len() > 64 || buf.len() < EVENT_PREFIX.len() + event.len() {
            return None;
        }
        buf[..EVENT_PREFIX.len()].copy_from_slice(EVENT_PREFIX);
        buf[EVENT_PREFIX.len()..EVENT_PREFIX.len() + event.len()].copy_from_slice(event);
        Some(EVENT_PREFIX.len() + event.len())
    }
    /// Commit the cluster's Ed25519 public key to the replicated state (the
    /// key ceremony). `arg0` is unused; the attached memory object holds the
    /// 32 key bytes. The reply is the committed key generation (deferred
    /// until it has replicated).
    pub const OP_SET_KEY: u32 = 13;
    /// Read the cluster public key from locally applied state. The reply
    /// moves a page holding the 32 key bytes, or is `ERR_NOT_FOUND` when no
    /// ceremony has committed a key yet.
    pub const OP_KEY: u32 = 14;

    /// Stable object-store id of the deploy demo payload.
    pub const DEPLOY_OBJECT_ID: u64 = 0x0000_0000_0000_0042;

    /// Tag for the cluster-wide artifact namespace in the object store.
    ///
    /// Artifact ids are cluster-global: every node stores the artifact for a
    /// logical name at the same derived id (`artifact_object_id`), so the
    /// physical reference in a deployment record is the pair
    /// `(node_key, artifact_object_id(name))` and the node dimension never
    /// leaks into the identifier itself.
    pub const ARTIFACT_ID_TAG: u64 = charlotte_launch::ARTIFACT_ID_TAG;

    /// The stable, cluster-wide object-store id for a logical artifact name.
    pub fn artifact_object_id(name: &[u8]) -> u64 {
        charlotte_launch::artifact_object_id(name)
    }

    /// Shared byte offsets in the DNS service's diagnostic status page.
    pub use charlotte_launch::dns_status as status;

    /// The name is registered on this node (lookup returns a connection).
    pub const RESULT_LOCAL: i64 = 0;
    /// The name is registered on a remote node (lookup returns its node id).
    pub const RESULT_REMOTE: i64 = 1;
    pub const ERR_NOT_FOUND: i64 = -1;
    pub const ERR_NOT_LEADER: i64 = -2;
    pub const ERR_BAD_OPCODE: i64 = -3;
    pub const ERR_TOO_LARGE: i64 = -4;
    /// The accepted invocation may have executed, but no authoritative result
    /// was confirmed before timeout or reply-binding failure. Do not replay a
    /// non-idempotent operation.
    pub const ERR_UNCERTAIN: i64 = -5;
    pub const ERR_BUSY: i64 = -6;
    pub const ERR_STALE_GENERATION: i64 = -7;
    pub const ERR_UNTRUSTED_KEY: i64 = -8;
    /// Catalog observation could not prepare its output backing/mapping.
    pub const ERR_RESOURCE_LIMIT: i64 = -9;
}

/// Protocol of the cluster-deployment demo artifact: a tiny service that the
/// per-node deploy agent hosts under whatever name the cluster manifest
/// assigns it. The artifact is a note-signed ELF in the object store
/// (`.note.charlotte-sig`); `OP_GET` returns its leading eight bytes as the
/// scalar result, proving that the calling node reached the exact artifact
/// the cluster assigned and the serving node verified.
pub mod deploy {
    pub const INTERFACE: u64 = super::name(b"DPLY");
    pub const VERSION: u32 = 1;
    /// The demo artifact's deployment name (packed LE).
    pub const NAME: u64 = super::name(b"greet");
    pub const OP_GET: u32 = 1;

    /// The scalar value `OP_GET` returns: the deployed artifact's leading
    /// eight bytes — the little-endian ELF header of the note-signed `greet`
    /// binary (`0x7f 'E' 'L' 'F' 2 1 1 0`).
    pub const GREET_VALUE: u64 = 0x0001_0102_464c_457f;
}

/// Protocol of the cluster administration service (`clusterctl`): the
/// "outside" interface to a cluster. It wraps the raw dns manifest ops and
/// the object store behind admin-level operations: upload a signed artifact,
/// deploy it to a node, and query the deployment manifest.
///
/// Artifact names are bare cluster-global names ("greet"); the object-store
/// id is derived from the name (`dns::artifact_object_id`), and the node
/// dimension appears only in the deployment record.
pub mod clusterctl {
    pub const INTERFACE: u64 = super::name(b"CTL");
    /// Version 3 requires full-name status queries and removes unsigned deploy.
    pub const VERSION: u32 = 3;
    /// The service's short name (packed LE).
    pub const NAME: u64 = super::name(b"ctl");
    /// Plain HTTP notification ingress. Authenticity and integrity come from
    /// the signed descriptor; the transport deliberately carries no secrets.
    pub const NOTIFY_PORT: u16 = charlotte_launch::DEPLOY_NOTIFY_PORT;
    pub const NOTIFY_PATH: &[u8] = b"/v1/deployments";
    pub const RELEASE_PATH: &[u8] = b"/v1/releases";
    pub const OPERATIONS_PATH: &[u8] = b"/v1/operations";
    pub const SHUTDOWN_PATH: &[u8] = b"/v1/shutdowns";
    pub const INGRESS_POLICY_PATH: &[u8] = b"/v1/ingress-policy";

    /// Upload an artifact. `arg0` is the packed artifact name; the attached
    /// memory object holds `[artifact_len:u64 LE][artifact]`, where the
    /// artifact is a note-signed ELF (`.note.charlotte-sig`) produced
    /// off-cluster with `tools/cluster-sign elf-sign` and the cluster's
    /// private key. The service stores it as-is at the artifact's derived
    /// id; nodes validate the signature against the cluster public key at
    /// pickup. The reply is the object id.
    pub const OP_UPLOAD: u32 = 1;
    // Opcode 2 was descriptor-less deployment; retired, never reuse.
    /// Query a deployment by its full artifact name in moved memory. `arg0`
    /// is the name length. The reply is the complete current record encoded by
    /// [`crate::name_catalog::encode_deployment_result`], including signed
    /// descriptor and per-replica variant pins. Missing records return
    /// `ERR_NOT_FOUND`; malformed names return `ERR_TOO_LARGE`.
    pub const OP_STATUS: u32 = 3;
    /// Commit the cluster's Ed25519 public key to the replicated state (the
    /// key ceremony, performed once during cluster establishment). `arg0` is
    /// unused; the attached memory object holds the 32 key bytes. The key is
    /// the one the IT department's private key matches; after it is
    /// committed, every joining node obtains it from the cluster. The reply
    /// is the committed key generation.
    pub const OP_KEYCEREMONY: u32 = 4;
    /// Read the cluster public key committed by the ceremony. The reply moves
    /// a page holding the 32 key bytes, or is `ERR_NOT_FOUND` before the
    /// first ceremony.
    pub const OP_KEY: u32 = 5;
    /// Notify the cluster of a signed `CDEPLOY6` descriptor. The artifact
    /// name is taken from the signed descriptor and the moved memory uses the same
    /// `[len:u64][bytes]` envelope as `OP_UPLOAD`. The descriptor contains the
    /// central object key, digest, target node, revision, per-thread stack
    /// pages, maximum thread count, placement policy, and capability grants. No object-store
    /// credentials cross this interface.
    pub const OP_NOTIFY: u32 = 6;
    /// Join the cluster on the local network segment. The service asks the
    /// local discovery service for the cluster's leader (or a follower that
    /// redirects towards it), then asks that leader's DNS-owned Raft
    /// administration endpoint to admit this node. The reply is the committed JOIN log index, or a
    /// negative error (`ERR_NO_CLUSTER` when no peer on the segment reports a
    /// cluster).
    pub const OP_JOIN: u32 = 7;
    /// Query a named deployment's observable rollout condition. The full
    /// artifact name is carried in moved memory and `arg0` is its length.
    /// The reply is [`ROLLOUT_STATUS_LEN`] bytes encoded by
    /// [`RolloutStatus::encode`].
    pub const OP_ROLLOUT: u32 = 8;
    /// Notify the cluster of a signed `CRELEASE` envelope. The moved memory
    /// uses `[len:u64][bytes]`, as for `OP_NOTIFY`. Success moves a complete
    /// `CRLADM01` receipt with its byte length; errors are negative scalars.
    pub const OP_NOTIFY_RELEASE: u32 = 9;
    /// Notify the cluster of an independently operator-signed `COPSBND2`.
    /// The moved memory uses `[len:u64][bytes]`; the leader verifies release,
    /// operations, recipient, cluster and expiry context before admission.
    pub const OP_NOTIFY_OPERATIONS: u32 = 10;
    /// Notify the cluster of an operator-signed, node-targeted `CSHUTDN1`
    /// intent. The moved memory uses `[len:u64][bytes]`; no private signing
    /// material enters the cluster.
    pub const OP_NOTIFY_SHUTDOWN: u32 = 11;
    /// Notify the cluster of an operations-signed `CINGPOL1` complete ingress
    /// assignment replacement. The moved memory uses `[len:u64][bytes]`.
    pub const OP_NOTIFY_INGRESS_POLICY: u32 = 12;
    /// Return `[generation:u64][CINGPOL1 envelope]` for operational
    /// inspection, or `ERR_NOT_FOUND` before the first committed policy.
    pub const OP_INGRESS_POLICY_STATUS: u32 = 13;

    pub const ROLLOUT_COMMITTED: u8 = 1;
    pub const ROLLOUT_READY: u8 = 2;
    /// Some but not all exact-generation replicas are ready. This is
    /// observable during replacement and is not a successful rollout.
    pub const ROLLOUT_REPLACING: u8 = 3;
    pub const ROLLOUT_STATUS_LEN: usize = 40;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RolloutStatus {
        pub state: u8,
        pub deployment_generation: u64,
        pub service_generation: u64,
        pub node_key: u64,
        pub desired_replicas: u16,
        pub ready_replicas: u16,
    }

    impl RolloutStatus {
        pub fn encode(self) -> [u8; ROLLOUT_STATUS_LEN] {
            let mut bytes = [0u8; ROLLOUT_STATUS_LEN];
            bytes[0] = self.state;
            bytes[8..16].copy_from_slice(&self.deployment_generation.to_le_bytes());
            bytes[16..24].copy_from_slice(&self.service_generation.to_le_bytes());
            bytes[24..32].copy_from_slice(&self.node_key.to_le_bytes());
            bytes[32..34].copy_from_slice(&self.desired_replicas.to_le_bytes());
            bytes[34..36].copy_from_slice(&self.ready_replicas.to_le_bytes());
            bytes
        }

        pub fn decode(bytes: &[u8]) -> Option<Self> {
            if bytes.len() != ROLLOUT_STATUS_LEN
                || !matches!(bytes[0], ROLLOUT_COMMITTED | ROLLOUT_READY | ROLLOUT_REPLACING)
                || bytes[1..8].iter().any(|byte| *byte != 0)
                || bytes[36..40].iter().any(|byte| *byte != 0)
            {
                return None;
            }
            Some(Self {
                state: bytes[0],
                deployment_generation: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
                service_generation: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
                node_key: u64::from_le_bytes(bytes[24..32].try_into().ok()?),
                desired_replicas: u16::from_le_bytes(bytes[32..34].try_into().ok()?),
                ready_replicas: u16::from_le_bytes(bytes[34..36].try_into().ok()?),
            })
        }
    }

    pub const ERR_NOT_FOUND: i64 = -1;
    pub const ERR_NOT_LEADER: i64 = -2;
    pub const ERR_TOO_LARGE: i64 = -3;
    pub const ERR_UPLOAD_FAILED: i64 = -10;
    /// No node on the local segment reported membership in a cluster (or the
    /// local discovery/DNS Raft endpoint is unavailable): the honest "nothing
    /// to join" answer.
    pub const ERR_NO_CLUSTER: i64 = -8;
    /// The ELF is unsigned, signed by another key, or blessed for a
    /// different logical artifact name.
    pub const ERR_UNTRUSTED_ARTIFACT: i64 = -11;
    pub const ERR_UNTRUSTED_KEY: i64 = -12;
    /// The deployment descriptor is malformed, untrusted, or does not bind
    /// the requested artifact name.
    pub const ERR_UNTRUSTED_DESCRIPTOR: i64 = -13;
    /// The signed sequence is older than the committed descriptor sequence.
    pub const ERR_STALE_DESCRIPTOR: i64 = -14;
    /// The sequence is already committed with different signed bytes.
    pub const ERR_CONFLICTING_DESCRIPTOR: i64 = -15;
    /// UTC was unavailable, so expiry validation failed closed.
    pub const ERR_TIME_UNAVAILABLE: i64 = -16;
    /// At least one encrypted operational profile has expired.
    pub const ERR_EXPIRED_OPERATION: i64 = -17;
    /// The trusted UTC clock is outside a signed request's validity window.
    pub const ERR_OUTSIDE_VALIDITY: i64 = -18;
    /// The signed replica policy cannot be satisfied by the current admitted,
    /// non-draining voting members.
    pub const ERR_UNSATISFIABLE_PLACEMENT: i64 = -19;
    /// Accepting the command would exceed a bounded replicated collection.
    pub const ERR_CAPACITY: i64 = -20;
    /// Eligible nodes exist, but none can host the deployment's declared
    /// resource demand (currently its stack reservation and runtime pages).
    pub const ERR_INSUFFICIENT_CAPACITY: i64 = -21;
    pub const ERR_INCOMPATIBLE_TARGET: i64 = -23;
    /// Local observation timed out. A received mutation may already have
    /// committed; this error is not permission to replay it or undo authority.
    pub const ERR_RPC_TIMEOUT: i64 = -22;
}

/// Remote-invocation wire protocol carried over the reliable message layer.
///
/// Generation-fenced remote invocation. The transport prepends the type tag.
/// Current records retain complete bounded identities; retired 0x10/0x11
/// records are never reinterpreted as the current grammar.
pub mod rcall {
    pub const TAG_REQUEST: u8 = 0x26;
    pub const TAG_REPLY: u8 = 0x27;
    pub const MAX_IDENTITY_BYTES: usize = 4096;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum EncodeError {
        InvalidIdentity,
        TooLarge,
        AllocationFailed,
    }

    impl EncodeError {
        pub fn status(self) -> i64 {
            match self {
                Self::AllocationFailed => super::dns::ERR_RESOURCE_LIMIT,
                Self::InvalidIdentity | Self::TooLarge => super::dns::ERR_TOO_LARGE,
            }
        }
    }

    fn identity_len(identity: &[u8]) -> Result<u32, EncodeError> {
        if identity.is_empty() {
            return Err(EncodeError::InvalidIdentity);
        }
        if identity.len() > MAX_IDENTITY_BYTES {
            return Err(EncodeError::TooLarge);
        }
        Ok(identity.len() as u32)
    }

    fn buffer(length: usize) -> Result<alloc::vec::Vec<u8>, EncodeError> {
        let mut bytes = alloc::vec::Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| EncodeError::AllocationFailed)?;
        Ok(bytes)
    }

    fn copy_identity(identity: &[u8]) -> Option<alloc::vec::Vec<u8>> {
        let mut bytes = buffer(identity.len()).ok()?;
        bytes.extend_from_slice(identity);
        Some(bytes)
    }

    /// Encode the complete body before publishing an in-flight invocation.
    pub fn encode_request(
        session: u64,
        call_id: u64,
        caller: &[u8],
        name: &[u8],
        target_generation: u64,
        opcode: u32,
        arg: i64,
    ) -> Result<alloc::vec::Vec<u8>, EncodeError> {
        let caller_len = identity_len(caller)?;
        let name_len = identity_len(name)?;
        let mut frame = buffer(44 + caller.len() + name.len())?;
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&call_id.to_le_bytes());
        frame.extend_from_slice(&caller_len.to_le_bytes());
        frame.extend_from_slice(caller);
        frame.extend_from_slice(&name_len.to_le_bytes());
        frame.extend_from_slice(name);
        frame.extend_from_slice(&target_generation.to_le_bytes());
        frame.extend_from_slice(&opcode.to_le_bytes());
        frame.extend_from_slice(&arg.to_le_bytes());
        Ok(frame)
    }

    pub type Request = (u64, u64, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>, u64, u32, i64);

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.first().copied()? != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame.get(1..9)?.try_into().ok()?);
        let call_id = u64::from_le_bytes(frame.get(9..17)?.try_into().ok()?);
        let caller_len = u32::from_le_bytes(frame.get(17..21)?.try_into().ok()?) as usize;
        let caller_end = 21usize.checked_add(caller_len)?;
        let caller = frame.get(21..caller_end)?;
        let name_start = caller_end.checked_add(4)?;
        let name_len =
            u32::from_le_bytes(frame.get(caller_end..name_start)?.try_into().ok()?) as usize;
        let tail = name_start.checked_add(name_len)?;
        let name = frame.get(name_start..tail)?;
        if frame.len() != tail.checked_add(20)? {
            return None;
        }
        identity_len(caller).ok()?;
        identity_len(name).ok()?;
        let target_generation = u64::from_le_bytes(frame.get(tail..tail + 8)?.try_into().ok()?);
        let opcode = u32::from_le_bytes(frame.get(tail + 8..tail + 12)?.try_into().ok()?);
        let arg = i64::from_le_bytes(frame.get(tail + 12..tail + 20)?.try_into().ok()?);
        Some((
            session,
            call_id,
            copy_identity(caller)?,
            copy_identity(name)?,
            target_generation,
            opcode,
            arg,
        ))
    }

    /// Prepare reply storage before admitting any target operation. Completion
    /// only fills the result in that same buffer, without another allocation.
    pub struct PreparedReply {
        bytes: alloc::vec::Vec<u8>,
    }

    impl PreparedReply {
        pub fn try_new(
            session: u64,
            call_id: u64,
            target_generation: u64,
        ) -> Result<Self, EncodeError> {
            let mut bytes = buffer(32)?;
            bytes.extend_from_slice(&session.to_le_bytes());
            bytes.extend_from_slice(&call_id.to_le_bytes());
            bytes.extend_from_slice(&target_generation.to_le_bytes());
            bytes.extend_from_slice(&0i64.to_le_bytes());
            Ok(Self {
                bytes,
            })
        }

        pub fn finish(mut self, result: i64) -> alloc::vec::Vec<u8> {
            self.bytes[24..32].copy_from_slice(&result.to_le_bytes());
            self.bytes
        }
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, u64, i64)> {
        if frame.len() != 33 || frame[0] != TAG_REPLY {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let call_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let target_generation = u64::from_le_bytes(frame[17..25].try_into().ok()?);
        let result = i64::from_le_bytes(frame[25..33].try_into().ok()?);
        Some((session, call_id, target_generation, result))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn tagged(tag: u8, body: &[u8]) -> alloc::vec::Vec<u8> {
            [&[tag], body].concat()
        }

        #[test]
        fn invocation_golden_and_prepared_reply_are_exact() {
            let request = encode_request(1, 2, b"n", b"a", 3, 4, -5).unwrap();
            let mut expected = alloc::vec::Vec::new();
            expected.extend_from_slice(&1u64.to_le_bytes());
            expected.extend_from_slice(&2u64.to_le_bytes());
            expected.extend_from_slice(b"\x01\0\0\0n\x01\0\0\0a");
            expected.extend_from_slice(&3u64.to_le_bytes());
            expected.extend_from_slice(&4u32.to_le_bytes());
            expected.extend_from_slice(&(-5i64).to_le_bytes());
            assert_eq!(request, expected);
            assert_eq!(
                decode_request(&tagged(TAG_REQUEST, &request)),
                Some((1, 2, b"n".to_vec(), b"a".to_vec(), 3, 4, -5))
            );
            let prepared = PreparedReply::try_new(1, 2, 3).unwrap();
            let allocation = prepared.bytes.as_ptr();
            let capacity = prepared.bytes.capacity();
            let reply = prepared.finish(-5);
            assert_eq!(reply.as_ptr(), allocation);
            assert_eq!(reply.capacity(), capacity);
            assert_eq!(reply.len(), 32);
            assert_eq!(decode_reply(&tagged(TAG_REPLY, &reply)), Some((1, 2, 3, -5)));
        }

        #[test]
        fn invocation_identity_bounds_preserve_suffixes() {
            let mut first = alloc::vec![b'x'; MAX_IDENTITY_BYTES];
            let mut second = first.clone();
            first[MAX_IDENTITY_BYTES - 1] = b'a';
            second[MAX_IDENTITY_BYTES - 1] = b'b';
            for identity in [&first[..], &second[..]] {
                let request = encode_request(1, 2, identity, identity, 3, 4, -5).unwrap();
                assert_eq!(
                    decode_request(&tagged(TAG_REQUEST, &request)),
                    Some((1, 2, identity.to_vec(), identity.to_vec(), 3, 4, -5))
                );
            }
            let oversized = alloc::vec![b'x'; MAX_IDENTITY_BYTES + 1];
            assert_eq!(encode_request(1, 2, &oversized, b"a", 3, 4, 5), Err(EncodeError::TooLarge));
            assert_eq!(encode_request(1, 2, b"n", &oversized, 3, 4, 5), Err(EncodeError::TooLarge));
            assert_eq!(encode_request(1, 2, b"", b"a", 3, 4, 5), Err(EncodeError::InvalidIdentity));
            assert_eq!(encode_request(1, 2, b"n", b"", 3, 4, 5), Err(EncodeError::InvalidIdentity));
            let mut request = encode_request(1, 2, b"n", &first, 3, 4, 5).unwrap();
            request.insert(25 + MAX_IDENTITY_BYTES, b'x');
            request[21..25].copy_from_slice(&((MAX_IDENTITY_BYTES + 1) as u32).to_le_bytes());
            assert!(decode_request(&tagged(TAG_REQUEST, &request)).is_none());
            let mut request = encode_request(1, 2, &first, b"a", 3, 4, 5).unwrap();
            request.insert(20 + MAX_IDENTITY_BYTES, b'x');
            request[16..20].copy_from_slice(&((MAX_IDENTITY_BYTES + 1) as u32).to_le_bytes());
            assert!(decode_request(&tagged(TAG_REQUEST, &request)).is_none());
            for (caller, name) in
                [(b"".as_slice(), b"a".as_slice()), (b"n".as_slice(), b"".as_slice())]
            {
                let mut request = alloc::vec::Vec::new();
                request.extend_from_slice(&1u64.to_le_bytes());
                request.extend_from_slice(&2u64.to_le_bytes());
                request.extend_from_slice(&(caller.len() as u32).to_le_bytes());
                request.extend_from_slice(caller);
                request.extend_from_slice(&(name.len() as u32).to_le_bytes());
                request.extend_from_slice(name);
                request.extend_from_slice(&3u64.to_le_bytes());
                request.extend_from_slice(&4u32.to_le_bytes());
                request.extend_from_slice(&5i64.to_le_bytes());
                assert!(decode_request(&tagged(TAG_REQUEST, &request)).is_none());
            }
        }

        #[test]
        fn invocation_rejects_retired_mistagged_incomplete_and_extended_records() {
            let request =
                tagged(TAG_REQUEST, &encode_request(1, 2, b"node", b"name", 3, 4, 5).unwrap());
            let reply = tagged(TAG_REPLY, &PreparedReply::try_new(1, 2, 3).unwrap().finish(5));
            for bytes in [&request, &reply] {
                for end in 0..bytes.len() {
                    assert!(decode_request(&bytes[..end]).is_none());
                    assert!(decode_reply(&bytes[..end]).is_none());
                }
                let mut extended = bytes.clone();
                extended.push(0);
                assert!(decode_request(&extended).is_none());
                assert!(decode_reply(&extended).is_none());
                for tag in [0x10, 0x11, 0xff] {
                    let mut retired = bytes.clone();
                    retired[0] = tag;
                    assert!(decode_request(&retired).is_none());
                    assert!(decode_reply(&retired).is_none());
                }
            }
            assert!(decode_request(&reply).is_none());
            assert!(decode_reply(&request).is_none());
            let mut invalid = request.clone();
            invalid[17..21].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(decode_request(&invalid).is_none());
            let mut invalid = request.clone();
            invalid[25..29].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(decode_request(&invalid).is_none());
            let mut old = alloc::vec![0x10];
            old.extend_from_slice(&1u64.to_le_bytes());
            old.extend_from_slice(&2u64.to_le_bytes());
            old.extend_from_slice(b"\x04node\x04name");
            old.extend_from_slice(&3u64.to_le_bytes());
            old.extend_from_slice(&4u32.to_le_bytes());
            old.extend_from_slice(&5i64.to_le_bytes());
            assert!(decode_request(&old).is_none());
            old[0] = TAG_REQUEST;
            assert!(decode_request(&old).is_none());
            let old_reply = tagged(0x11, &PreparedReply::try_new(1, 2, 3).unwrap().finish(5));
            assert!(decode_reply(&old_reply).is_none());
        }
    }
}

/// Correlated follower-to-leader catalog queries. Followers must not answer
/// `dns::OP_LOOKUP` or choose an `OP_CALL` target from their local applied
/// state, because that state can be stale. The leader replies only after its
/// Graft read barrier admits a linearizable query.
pub mod rquery {
    // Tags 0x12/0x13 used lossy u8 identities; retired, never reinterpreted.
    pub const TAG_REQUEST: u8 = 0x24;
    pub const TAG_REPLY: u8 = 0x25;
    /// Bounds transport metadata, independently of local publication policy.
    /// The persisted node identity object is also bounded to one 4 KiB page.
    pub const MAX_IDENTITY_BYTES: usize = 4096;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum EncodeError {
        InvalidIdentity,
        TooLarge,
        AllocationFailed,
    }

    impl EncodeError {
        pub fn status(self) -> i64 {
            match self {
                Self::AllocationFailed => super::dns::ERR_RESOURCE_LIMIT,
                Self::InvalidIdentity | Self::TooLarge => super::dns::ERR_TOO_LARGE,
            }
        }
    }

    fn identity_len(identity: &[u8], allow_empty: bool) -> Result<u32, EncodeError> {
        if identity.is_empty() && !allow_empty {
            return Err(EncodeError::InvalidIdentity);
        }
        if identity.len() > MAX_IDENTITY_BYTES {
            return Err(EncodeError::TooLarge);
        }
        Ok(identity.len() as u32)
    }

    fn buffer(length: usize) -> Result<alloc::vec::Vec<u8>, EncodeError> {
        let mut bytes = alloc::vec::Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| EncodeError::AllocationFailed)?;
        Ok(bytes)
    }

    fn copy_identity(identity: &[u8], allow_empty: bool) -> Option<alloc::vec::Vec<u8>> {
        identity_len(identity, allow_empty).ok()?;
        let mut bytes = buffer(identity.len()).ok()?;
        bytes.extend_from_slice(identity);
        Some(bytes)
    }

    /// Encode the body without the tag; the transport prepends it.
    pub fn encode_request(
        session: u64,
        query_id: u64,
        caller: &[u8],
        name: &[u8],
    ) -> Result<alloc::vec::Vec<u8>, EncodeError> {
        let caller_len = identity_len(caller, false)?;
        let name_len = identity_len(name, false)?;
        let mut frame = buffer(24 + caller.len() + name.len())?;
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&query_id.to_le_bytes());
        frame.extend_from_slice(&caller_len.to_le_bytes());
        frame.extend_from_slice(caller);
        frame.extend_from_slice(&name_len.to_le_bytes());
        frame.extend_from_slice(name);
        Ok(frame)
    }

    pub type Request = (u64, u64, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>);

    /// Validate exact framing and identity bounds before fallible copies.
    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.first().copied()? != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame.get(1..9)?.try_into().ok()?);
        let query_id = u64::from_le_bytes(frame.get(9..17)?.try_into().ok()?);
        let caller_len = u32::from_le_bytes(frame.get(17..21)?.try_into().ok()?) as usize;
        let caller_end = 21usize.checked_add(caller_len)?;
        let caller = frame.get(21..caller_end)?;
        let name_start = caller_end.checked_add(4)?;
        let name_len =
            u32::from_le_bytes(frame.get(caller_end..name_start)?.try_into().ok()?) as usize;
        let end = name_start.checked_add(name_len)?;
        let name = frame.get(name_start..end)?;
        if frame.len() != end {
            return None;
        }
        identity_len(caller, false).ok()?;
        identity_len(name, false).ok()?;
        Some((session, query_id, copy_identity(caller, false)?, copy_identity(name, false)?))
    }

    pub fn encode_reply(
        session: u64,
        query_id: u64,
        status: i64,
        generation: u64,
        node: &[u8],
    ) -> Result<alloc::vec::Vec<u8>, EncodeError> {
        // Empty owner/generation zero represent an ordinary missing result.
        let node_len = identity_len(node, true)?;
        let mut frame = buffer(36 + node.len())?;
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&query_id.to_le_bytes());
        frame.extend_from_slice(&status.to_le_bytes());
        frame.extend_from_slice(&generation.to_le_bytes());
        frame.extend_from_slice(&node_len.to_le_bytes());
        frame.extend_from_slice(node);
        Ok(frame)
    }

    pub type Reply = (u64, u64, i64, u64, alloc::vec::Vec<u8>);

    pub fn decode_reply(frame: &[u8]) -> Option<Reply> {
        if frame.first().copied()? != TAG_REPLY {
            return None;
        }
        let session = u64::from_le_bytes(frame.get(1..9)?.try_into().ok()?);
        let query_id = u64::from_le_bytes(frame.get(9..17)?.try_into().ok()?);
        let status = i64::from_le_bytes(frame.get(17..25)?.try_into().ok()?);
        let generation = u64::from_le_bytes(frame.get(25..33)?.try_into().ok()?);
        let node_len = u32::from_le_bytes(frame.get(33..37)?.try_into().ok()?) as usize;
        if frame.len() != 37usize.checked_add(node_len)? {
            return None;
        }
        let node = copy_identity(frame.get(37..)?, true)?;
        Some((session, query_id, status, generation, node))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn tagged(tag: u8, body: &[u8]) -> alloc::vec::Vec<u8> {
            [&[tag], body].concat()
        }

        #[test]
        fn query_golden_framing_and_missing_result_are_exact() {
            let request = encode_request(1, 2, b"n", b"a").unwrap();
            let expected = b"\x01\0\0\0\0\0\0\0\x02\0\0\0\0\0\0\0\x01\0\0\0n\x01\0\0\0a";
            assert_eq!(request, expected);
            assert_eq!(
                decode_request(&tagged(TAG_REQUEST, &request)),
                Some((1, 2, b"n".to_vec(), b"a".to_vec()))
            );
            let reply = encode_reply(1, 2, 0, 7, b"n").unwrap();
            assert_eq!(reply.len(), 37);
            assert_eq!(&reply[32..], b"\x01\0\0\0n");
            assert_eq!(decode_reply(&tagged(TAG_REPLY, &reply)), Some((1, 2, 0, 7, b"n".to_vec())));
            for status in [0, super::super::dns::ERR_NOT_LEADER] {
                let missing = encode_reply(1, 2, status, 0, b"").unwrap();
                assert_eq!(
                    decode_reply(&tagged(TAG_REPLY, &missing)),
                    Some((1, 2, status, 0, alloc::vec::Vec::new()))
                );
            }
        }

        #[test]
        fn query_full_identities_and_bounds_do_not_alias_prefixes() {
            let mut first = alloc::vec![b'x'; MAX_IDENTITY_BYTES];
            let mut second = first.clone();
            first[MAX_IDENTITY_BYTES - 1] = b'a';
            second[MAX_IDENTITY_BYTES - 1] = b'b';
            for identity in [&first[..], &second[..]] {
                let request = encode_request(3, 4, identity, identity).unwrap();
                assert_eq!(
                    decode_request(&tagged(TAG_REQUEST, &request)),
                    Some((3, 4, identity.to_vec(), identity.to_vec()))
                );
                let reply = encode_reply(3, 4, 0, 9, identity).unwrap();
                assert_eq!(
                    decode_reply(&tagged(TAG_REPLY, &reply)),
                    Some((3, 4, 0, 9, identity.to_vec()))
                );
            }
            let oversized = alloc::vec![b'x'; MAX_IDENTITY_BYTES + 1];
            assert_eq!(encode_request(1, 2, &oversized, b"a"), Err(EncodeError::TooLarge));
            assert_eq!(encode_request(1, 2, b"n", &oversized), Err(EncodeError::TooLarge));
            assert_eq!(encode_reply(1, 2, 0, 1, &oversized), Err(EncodeError::TooLarge));
            let mut oversized_reply = encode_reply(1, 2, 0, 1, &first).unwrap();
            oversized_reply.push(b'x');
            oversized_reply[32..36]
                .copy_from_slice(&((MAX_IDENTITY_BYTES + 1) as u32).to_le_bytes());
            assert!(decode_reply(&tagged(TAG_REPLY, &oversized_reply)).is_none());
            let mut oversized_request = encode_request(1, 2, b"n", &first).unwrap();
            oversized_request.push(b'x');
            oversized_request[21..25]
                .copy_from_slice(&((MAX_IDENTITY_BYTES + 1) as u32).to_le_bytes());
            assert!(decode_request(&tagged(TAG_REQUEST, &oversized_request)).is_none());
            assert_eq!(encode_request(1, 2, b"", b"a"), Err(EncodeError::InvalidIdentity));
            assert_eq!(encode_request(1, 2, b"n", b""), Err(EncodeError::InvalidIdentity));
        }

        #[test]
        fn query_decoders_reject_retired_mistagged_incomplete_and_extended_frames() {
            let request = tagged(TAG_REQUEST, &encode_request(3, 4, b"node", b"name").unwrap());
            let reply = tagged(TAG_REPLY, &encode_reply(3, 4, 0, 9, b"node").unwrap());
            for bytes in [&request, &reply] {
                for end in 0..bytes.len() {
                    assert!(decode_request(&bytes[..end]).is_none());
                    assert!(decode_reply(&bytes[..end]).is_none());
                }
                let mut extended = bytes.clone();
                extended.push(0);
                assert!(decode_request(&extended).is_none());
                assert!(decode_reply(&extended).is_none());
                for tag in [0x12, 0x13, 0xff] {
                    let mut retired = bytes.clone();
                    retired[0] = tag;
                    assert!(decode_request(&retired).is_none());
                    assert!(decode_reply(&retired).is_none());
                }
            }
            assert!(decode_request(&reply).is_none());
            assert!(decode_reply(&request).is_none());
            let mut invalid = request.clone();
            invalid[17..21].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(decode_request(&invalid).is_none());
            let mut invalid = reply.clone();
            invalid[33..37].copy_from_slice(&0u32.to_le_bytes());
            assert!(decode_reply(&invalid).is_none());
            // Complete former grammars cannot be upgraded by relabelling tags.
            let mut old_request = alloc::vec![0x12];
            old_request.extend_from_slice(&3u64.to_le_bytes());
            old_request.extend_from_slice(&4u64.to_le_bytes());
            old_request.extend_from_slice(b"\x04node\x04name");
            let mut old_reply = alloc::vec![0x13];
            old_reply.extend_from_slice(&3u64.to_le_bytes());
            old_reply.extend_from_slice(&4u64.to_le_bytes());
            old_reply.extend_from_slice(&0i64.to_le_bytes());
            old_reply.extend_from_slice(&9u64.to_le_bytes());
            old_reply.extend_from_slice(b"\x04node");
            assert!(decode_request(&old_request).is_none());
            assert!(decode_reply(&old_reply).is_none());
            old_request[0] = TAG_REQUEST;
            old_reply[0] = TAG_REPLY;
            assert!(decode_request(&old_request).is_none());
            assert!(decode_reply(&old_reply).is_none());
        }
    }
}

/// Block until a pending call completes, returning
/// `(result, returned_connection_cap)`.
///
/// The wait is scheduler-backed and unbounded; a caller that needs a deadline
/// must poll an owned [`catten_rt::owned::PendingCall`] instead.
/// # Safety
///
/// `call` must be a live pending-call capability owned by the caller.
pub unsafe fn wait_reply(call: u64) -> (i64, u64) {
    let (status, result, cap) = catten_syscall::ipc_reply_wait(call);
    catten_syscall::ipc_close(call);
    if status == 0 {
        (result as i64, cap)
    } else {
        (-1, 0)
    }
}

/// Wait for a short name to be registered in the node-local name service.
///
/// A successful `OP_LOOKUP` remains pending in the name service until the
/// publisher registers `name`, and [`wait_reply`] parks the caller on that
/// pending call. The only retry here is admission of the lookup itself: a
/// burst of boot-time registrations and lookups can temporarily fill the name
/// service's endpoint queue. Sleeping between attempts keeps that
/// backpressure path scheduler-friendly instead of turning it into a spin
/// loop or making service startup depend on queue timing.
pub fn wait_for_registered_name(ns_conn: u64, name: u64) -> Option<(i64, u64)> {
    let call = scalar_call_with_backpressure(ns_conn, ns::OP_LOOKUP, name);

    let (generation, connection) = unsafe { wait_reply(call) };
    if generation >= 1 && connection != 0 {
        Some((generation, connection))
    } else {
        if connection != 0 {
            catten_syscall::ipc_close(connection);
        }
        None
    }
}

/// Block until a pending call completes.
pub fn spin_reply(call: u64) -> (i64, u64) {
    let (status, result, cap) = catten_syscall::ipc_reply_wait(call);
    catten_syscall::ipc_close(call);
    if status == 0 {
        (result as i64, cap)
    } else {
        (-1, 0)
    }
}

/// Timer-backed pause for explicit protocol/hardware admission backoff.
/// Rejection must not turn a caller's retry loop into a runnable spin.
pub fn sleep_ms(milliseconds: u64) {
    let timer = catten_rt::owned::Completion::timer(milliseconds).unwrap_or_else(|error| {
        catten_rt::logln!("[reactor] pause timer admission rejected: {:?}", error);
        catten_rt::domain_abort()
    });
    match timer.wait() {
        Ok(0) => {}
        result => {
            catten_rt::logln!("[reactor] pause timer did not complete: {:?}", result);
            catten_rt::domain_abort()
        }
    }
}

/// Submit a scalar call while retaining ownership of its pending-call
/// capability. Transient endpoint backpressure is retried with a timer-backed
/// pause, matching [`scalar_call_with_backpressure`] without exposing handles.
pub fn owned_call_with_backpressure(
    connection: catten_rt::owned::ConnectionRef<'_>,
    opcode: u32,
    arg0: u64,
) -> catten_rt::owned::PendingCall<'static> {
    loop {
        if let Ok(call) = connection.call(opcode, arg0) {
            return call;
        }
        sleep_ms(1);
    }
}

/// Wait for a service and return a uniquely owned connection to it.
pub fn wait_for_registered_name_owned(
    ns_connection: catten_rt::owned::ConnectionRef<'_>,
    name: u64,
) -> Option<(i64, catten_rt::owned::Connection)> {
    let result = owned_call_with_backpressure(ns_connection, ns::OP_LOOKUP, name).wait().ok()?;
    let connection = result.connection?;
    if result.result >= 1 {
        Some((result.result, connection))
    } else {
        None
    }
}

/// Resolve a service dependency while allowing node shutdown to interrupt
/// startup before the dependency registers.
pub fn wait_for_registered_name_or_shutdown_owned(
    ctx: &catten_rt::Context,
    ns_connection: catten_rt::owned::ConnectionRef<'_>,
    name: u64,
) -> Result<(i64, catten_rt::owned::Connection), catten_rt::ShutdownRequest> {
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return Err(request);
        }
        if let Some(found) = try_registered_name_owned(ns_connection, name) {
            return Ok(found);
        }
        sleep_ms(10);
    }
}

/// Wait for either a short or memory-carried service name and return one
/// uniquely owned connection. The staged name remains owned by this call
/// until the name-service lookup terminates.
pub fn wait_for_registered_name_bytes_owned(
    ns_connection: catten_rt::owned::ConnectionRef<'_>,
    service_name: &[u8],
) -> Option<(i64, catten_rt::owned::Connection)> {
    if service_name.is_empty() || service_name.len() > MAX_NAME_LEN {
        return None;
    }
    if service_name.len() <= 8 {
        return wait_for_registered_name_owned(ns_connection, name(service_name));
    }
    let staged = stage_name_owned(service_name)?;
    let result = loop {
        match ns_connection.call_copy(ns::OP_LOOKUP_NAMED, service_name.len() as u64, &staged) {
            Ok(call) => break call.wait().ok()?,
            Err(_) => sleep_ms(1),
        }
    };
    let connection = result.connection?;
    (result.result >= 1).then_some((result.result, connection))
}

/// Resolve an optional service without waiting for a future registration.
pub fn try_registered_name_owned(
    ns_connection: catten_rt::owned::ConnectionRef<'_>,
    name: u64,
) -> Option<(i64, catten_rt::owned::Connection)> {
    let result =
        owned_call_with_backpressure(ns_connection, ns::OP_TRY_LOOKUP, name).wait().ok()?;
    let connection = result.connection?;
    if result.result >= 1 {
        Some((result.result, connection))
    } else {
        None
    }
}

/// Resolve an optional full-length service name without waiting for a future
/// registration. The staged name remains borrowed until the call completes.
pub fn try_registered_name_bytes_owned(
    ns_connection: catten_rt::owned::ConnectionRef<'_>,
    service_name: &[u8],
) -> Option<(i64, catten_rt::owned::Connection)> {
    if service_name.is_empty() || service_name.len() > MAX_NAME_LEN {
        return None;
    }
    if service_name.len() <= 8 {
        return try_registered_name_owned(ns_connection, name(service_name));
    }
    let staged = stage_name_owned(service_name)?;
    let result = ns_connection
        .call_copy(ns::OP_TRY_LOOKUP_NAMED, service_name.len() as u64, &staged)
        .ok()?
        .wait()
        .ok()?;
    let connection = result.connection?;
    (result.result >= 1).then_some((result.result, connection))
}

/// Observe a live publication without acquiring invocation authority. This
/// also works for policy-published names that reject every legacy lookup.
pub fn publication_generation_owned(
    names: catten_rt::owned::ConnectionRef<'_>,
    service_name: &[u8],
) -> Option<u64> {
    publication_generation::observe(names, service_name).ok().flatten()
}

/// Wait for the local boot-ready marker using ownership-aware IPC.
pub fn wait_for_local_ready_owned(ns_connection: catten_rt::owned::ConnectionRef<'_>) -> bool {
    wait_for_registered_name_owned(ns_connection, charlotte_launch::LOCAL_READY_NAME).is_some()
}

/// Wait for the node-ready marker without making startup an uninterruptible
/// part of the service lifecycle.
///
/// Long-lived services should prefer this over [`wait_for_local_ready_owned`]:
/// the name service's ordinary deferred lookup intentionally has no deadline,
/// whereas a node shutdown must be able to retire a service that is still
/// waiting for one of its startup gates. The short non-blocking lookup and
/// owned timer keep both operations bounded without exposing capabilities.
pub fn wait_for_local_ready_or_shutdown(
    ctx: &catten_rt::Context,
    ns_connection: catten_rt::owned::ConnectionRef<'_>,
) -> Result<(), catten_rt::ShutdownRequest> {
    wait_for_registered_name_or_shutdown_owned(
        ctx,
        ns_connection,
        charlotte_launch::LOCAL_READY_NAME,
    )
    .map(|_| ())
}

/// Sleep for at most `milliseconds`, checking the lifecycle page every 10 ms.
///
/// Service event loops should use this for retry and reconciliation delays so
/// a configured interval does not silently become their minimum shutdown
/// latency.
pub fn sleep_ms_or_shutdown(
    ctx: &catten_rt::Context,
    milliseconds: u64,
) -> Result<(), catten_rt::ShutdownRequest> {
    let mut remaining = milliseconds;
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return Err(request);
        }
        if remaining == 0 {
            return Ok(());
        }
        let slice = remaining.min(10);
        sleep_ms(slice);
        remaining -= slice;
    }
}

/// Submit a scalar call without making transient endpoint backpressure fatal.
///
/// The syscall's compact ABI reports a zero call capability for a full target
/// queue. Callers use this helper only with a connection whose validity is
/// already part of their launch contract. A timer-backed sleep lets the
/// receiver drain its queue before another admission attempt; once submitted,
/// the returned pending-call capability should be consumed with
/// [`wait_reply`] or the corresponding memory-aware wait operation.
pub fn scalar_call_with_backpressure(connection: u64, opcode: u32, arg0: u64) -> u64 {
    loop {
        let call = catten_syscall::ipc_scalar_call(connection, opcode, arg0);
        if call != 0 {
            return call;
        }
        sleep_ms(1);
    }
}

/// Block until the kernel has registered [`charlotte_launch::LOCAL_READY_NAME`]
/// in the name service, signalling that the *local* node is ready — its disk
/// stack is serving and the boot storm has settled — before it starts any
/// cluster-facing communication.
///
/// `ns::OP_LOOKUP` defers until the name is registered, so this returns as
/// soon as the kernel publishes the marker. Returns `false` only if the call
/// itself could not be made. Network-initiating services (cluster discovery,
/// reliable-message/Raft membership clients) must call this before starting
/// to communicate with other nodes.
pub fn wait_for_local_ready(ns_conn: u64) -> bool {
    let Some((_, connection)) =
        wait_for_registered_name(ns_conn, charlotte_launch::LOCAL_READY_NAME)
    else {
        return false;
    };
    catten_syscall::ipc_close(connection);
    true
}

/// Generation-fenced automatic service retraction sent by a publication's
/// owning node to the current Raft leader.
mod registration_relay;
pub use registration_relay::{
    rregister,
    runregister,
};

/// Advisory node-capacity reports from every node to the current leader.
///
/// Fire-and-forget: a lost report is refreshed by the next sampling interval,
/// and stale or unknown execution profiles are ineligible for placement. A
/// per-boot nonce lets the receiver fence reports within one boot while still
/// accepting a restarted reporter whose monotonic clock has reset. The nonce
/// identifies the boot. The enclosing reliable-message session supplies
/// monotonic incarnation order and rejects delayed older
/// sessions.
pub mod rcapacity {
    pub const TAG_REQUEST: u8 = 0x21;
    pub const FRAME_LEN: usize = 8 + 8 + 8 + 8 + 8 + 2 + 16;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Report {
        pub target: charlotte_launch::execution_target::ExecutionTarget,
        pub node_key: u64,
        /// Nonzero random word generated once at service start. Reporting is
        /// withheld until either CPU or delegated VirtIO entropy is available.
        pub boot_nonce: u64,
        pub epoch: u64,
        pub free_frames: u64,
        pub usable_frames: u64,
        pub cpu_load_permille: u16,
    }

    pub fn encode_request(report: Report) -> alloc::vec::Vec<u8> {
        let mut frame = alloc::vec::Vec::with_capacity(FRAME_LEN);
        frame.extend_from_slice(&report.node_key.to_le_bytes());
        frame.extend_from_slice(&report.boot_nonce.to_le_bytes());
        frame.extend_from_slice(&report.epoch.to_le_bytes());
        frame.extend_from_slice(&report.free_frames.to_le_bytes());
        frame.extend_from_slice(&report.usable_frames.to_le_bytes());
        frame.extend_from_slice(&report.cpu_load_permille.to_le_bytes());
        frame.extend_from_slice(&report.target.encode().unwrap_or([0; 16]));
        frame
    }

    /// Decode a delivered frame, tag byte included, matching the transport's
    /// receive convention.
    pub fn decode_request(frame: &[u8]) -> Option<Report> {
        if frame.len() != FRAME_LEN + 1 || frame[0] != TAG_REQUEST {
            return None;
        }
        let payload = &frame[1..];
        let report = Report {
            target: charlotte_launch::execution_target::ExecutionTarget::decode(&payload[42..58])?,
            node_key: u64::from_le_bytes(payload[0..8].try_into().ok()?),
            boot_nonce: u64::from_le_bytes(payload[8..16].try_into().ok()?),
            epoch: u64::from_le_bytes(payload[16..24].try_into().ok()?),
            free_frames: u64::from_le_bytes(payload[24..32].try_into().ok()?),
            usable_frames: u64::from_le_bytes(payload[32..40].try_into().ok()?),
            cpu_load_permille: u16::from_le_bytes(payload[40..42].try_into().ok()?),
        };
        (report.node_key != 0
            && report.boot_nonce != 0
            && report.epoch != 0
            && report.usable_frames != 0
            && report.free_frames <= report.usable_frames
            && report.cpu_load_permille <= 1000)
            .then_some(report)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn capacity_report_round_trips() {
            let report = Report {
                target: charlotte_launch::execution_target::ExecutionTarget::baseline(
                    charlotte_launch::execution_target::Architecture::Aarch64,
                ),
                node_key: 0x1234,
                boot_nonce: 0xfeed_face,
                epoch: 7,
                free_frames: 90_000,
                usable_frames: 113_000,
                cpu_load_permille: 420,
            };
            let payload = encode_request(report);
            assert_eq!(payload.len(), FRAME_LEN);
            let mut frame = alloc::vec::Vec::with_capacity(FRAME_LEN + 1);
            frame.push(TAG_REQUEST);
            frame.extend_from_slice(&payload);
            assert_eq!(decode_request(&frame), Some(report));
            assert_eq!(decode_request(&frame[..frame.len() - 1]), None);
            assert_eq!(decode_request(&payload), None);

            let mut invalid = frame.clone();
            invalid[9..17].copy_from_slice(&0u64.to_le_bytes());
            assert_eq!(decode_request(&invalid), None, "zero boot nonce must be rejected");
            let mut invalid = frame;
            invalid[41..43].copy_from_slice(&1001u16.to_le_bytes());
            assert_eq!(decode_request(&invalid), None, "CPU load must be bounded permille");
        }
    }
}

/// Correlated follower-to-leader deployment submission.
///
/// `clusterctl` verifies the signed descriptor before calling its node-local
/// DNS. A follower relays the already-bounded deployment request to the Raft
/// leader, which resolves automatic placement and commits it. The caller node
/// identity is checked against the reliable-message source route before the
/// leader accepts the request.
pub mod rdeploy {
    // 0x17/0x18 carried redundant deployment fields; retired.
    pub const TAG_REQUEST: u8 = 0x22;
    pub const TAG_REPLY: u8 = 0x23;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Request {
        pub session: u64,
        pub request_id: u64,
        pub caller: alloc::vec::Vec<u8>,
        pub descriptor: alloc::vec::Vec<u8>,
    }

    pub fn encode_request(request: &Request) -> Option<alloc::vec::Vec<u8>> {
        if request.caller.is_empty()
            || request.caller.len() > 255
            || request.descriptor.len() > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
            || charlotte_launch::deployment::decode(&request.descriptor).is_none()
        {
            return None;
        }
        let mut frame =
            alloc::vec::Vec::with_capacity(19 + request.caller.len() + request.descriptor.len());
        frame.extend_from_slice(&request.session.to_le_bytes());
        frame.extend_from_slice(&request.request_id.to_le_bytes());
        frame.push(request.caller.len() as u8);
        frame.extend_from_slice(&request.caller);
        frame.extend_from_slice(&(request.descriptor.len() as u16).to_le_bytes());
        frame.extend_from_slice(&request.descriptor);
        Some(frame)
    }

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 20 || frame[0] != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let request_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = usize::from(frame[17]);
        let len_offset = 18usize.checked_add(caller_len)?;
        let len = usize::from(u16::from_le_bytes(
            frame.get(len_offset..len_offset + 2)?.try_into().ok()?,
        ));
        let start = len_offset + 2;
        let end = start.checked_add(len)?;
        let descriptor = frame.get(start..end)?;
        if caller_len == 0
            || frame.len() != end
            || len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
            || charlotte_launch::deployment::decode(descriptor).is_none()
        {
            return None;
        }
        Some(Request {
            session,
            request_id,
            caller: frame.get(18..len_offset)?.to_vec(),
            descriptor: descriptor.to_vec(),
        })
    }

    pub fn encode_reply(session: u64, request_id: u64, result: i64) -> alloc::vec::Vec<u8> {
        let mut frame = alloc::vec::Vec::with_capacity(24);
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&request_id.to_le_bytes());
        frame.extend_from_slice(&result.to_le_bytes());
        frame
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, i64)> {
        if frame.len() != 25 || frame[0] != TAG_REPLY {
            return None;
        }
        Some((
            u64::from_le_bytes(frame[1..9].try_into().ok()?),
            u64::from_le_bytes(frame[9..17].try_into().ok()?),
            i64::from_le_bytes(frame[17..25].try_into().ok()?),
        ))
    }
}

/// Correlated follower-to-leader atomic release submission.
pub mod rrelease {
    pub const TAG_REQUEST: u8 = 0x2b;
    pub const TAG_REPLY: u8 = 0x2c;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Request {
        pub session: u64,
        pub request_id: u64,
        pub caller: alloc::vec::Vec<u8>,
        pub envelope: alloc::vec::Vec<u8>,
    }

    pub fn encode_request(request: &Request) -> Option<alloc::vec::Vec<u8>> {
        if request.caller.is_empty()
            || request.caller.len() > 255
            || request.envelope.len() > charlotte_launch::release::MAX_RELEASE_LEN
            || charlotte_launch::release::decode(&request.envelope).is_none()
        {
            return None;
        }
        let mut frame = alloc::vec::Vec::new();
        frame.try_reserve_exact(19 + request.caller.len() + request.envelope.len()).ok()?;
        frame.extend_from_slice(&request.session.to_le_bytes());
        frame.extend_from_slice(&request.request_id.to_le_bytes());
        frame.push(request.caller.len() as u8);
        frame.extend_from_slice(&request.caller);
        frame.extend_from_slice(&(request.envelope.len() as u16).to_le_bytes());
        frame.extend_from_slice(&request.envelope);
        Some(frame)
    }

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 20 || frame[0] != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let request_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = usize::from(frame[17]);
        let caller_start = 18usize;
        let envelope_len_offset = caller_start.checked_add(caller_len)?;
        let envelope_len = usize::from(u16::from_le_bytes(
            frame.get(envelope_len_offset..envelope_len_offset + 2)?.try_into().ok()?,
        ));
        let envelope_start = envelope_len_offset + 2;
        let envelope_end = envelope_start.checked_add(envelope_len)?;
        if caller_len == 0
            || envelope_len > charlotte_launch::release::MAX_RELEASE_LEN
            || frame.len() != envelope_end
            || charlotte_launch::release::decode(frame.get(envelope_start..envelope_end)?).is_none()
        {
            return None;
        }
        Some(Request {
            session,
            request_id,
            caller: frame.get(caller_start..envelope_len_offset)?.to_vec(),
            envelope: frame.get(envelope_start..envelope_end)?.to_vec(),
        })
    }

    /// Bounded terminal storage admitted before Raft submission.
    pub struct PreparedReply {
        bytes: alloc::vec::Vec<u8>,
    }

    impl PreparedReply {
        pub fn prepare(session: u64, request_id: u64) -> Option<Self> {
            let mut bytes = alloc::vec::Vec::new();
            bytes.try_reserve_exact(18 + charlotte_launch::release_admission::MAX_LEN).ok()?;
            bytes.resize(18 + charlotte_launch::release_admission::MAX_LEN, 0);
            bytes[..8].copy_from_slice(&session.to_le_bytes());
            bytes[8..16].copy_from_slice(&request_id.to_le_bytes());
            Some(Self {
                bytes,
            })
        }

        pub fn finish(mut self, result: &[u8]) -> alloc::vec::Vec<u8> {
            let fallback = crate::dns::ERR_UNCERTAIN.to_le_bytes();
            let result = if charlotte_launch::release_admission::decode_result(result).is_some() {
                result
            } else {
                &fallback
            };
            self.bytes[16..18].copy_from_slice(&(result.len() as u16).to_le_bytes());
            self.bytes[18..18 + result.len()].copy_from_slice(result);
            self.bytes.truncate(18 + result.len());
            self.bytes
        }

        pub fn reject(self, code: i64) -> alloc::vec::Vec<u8> {
            self.finish(&code.to_le_bytes())
        }
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, &[u8])> {
        if frame.first().copied()? != TAG_REPLY {
            return None;
        }
        let len = usize::from(u16::from_le_bytes(frame.get(17..19)?.try_into().ok()?));
        if frame.len() != 19 + len {
            return None;
        }
        let result = frame.get(19..)?;
        let _ = charlotte_launch::release_admission::decode_result(result)?;
        Some((
            u64::from_le_bytes(frame[1..9].try_into().ok()?),
            u64::from_le_bytes(frame[9..17].try_into().ok()?),
            result,
        ))
    }
}

/// Correlated follower-to-leader encrypted operational-bundle submission.
/// The body uses a 32-bit bundle length because `COPSBND2` intentionally
/// exceeds the historical 64 KiB release envelope.
pub mod roperations {
    pub const TAG_REQUEST: u8 = 0x1b;
    pub const TAG_REPLY: u8 = 0x1c;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Request {
        pub session: u64,
        pub request_id: u64,
        pub caller: alloc::vec::Vec<u8>,
        pub bundle: alloc::vec::Vec<u8>,
    }

    pub fn encode_request(request: &Request) -> Option<alloc::vec::Vec<u8>> {
        if request.caller.is_empty()
            || request.caller.len() > 255
            || request.bundle.len() > charlotte_launch::operations_bundle::MAX_BUNDLE_LEN
            || charlotte_launch::operations_bundle::decode(&request.bundle).is_none()
        {
            return None;
        }
        let frame_len =
            21usize.checked_add(request.caller.len())?.checked_add(request.bundle.len())?;
        // The transport adds the one-byte type tag.
        if frame_len + 1 > super::relmsg::MAX_MSG {
            return None;
        }
        let mut frame = alloc::vec::Vec::with_capacity(frame_len);
        frame.extend_from_slice(&request.session.to_le_bytes());
        frame.extend_from_slice(&request.request_id.to_le_bytes());
        frame.push(request.caller.len() as u8);
        frame.extend_from_slice(&request.caller);
        frame.extend_from_slice(&(request.bundle.len() as u32).to_le_bytes());
        frame.extend_from_slice(&request.bundle);
        Some(frame)
    }

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 22 || frame[0] != TAG_REQUEST || frame.len() > super::relmsg::MAX_MSG {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let request_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = usize::from(frame[17]);
        let caller_start = 18usize;
        let bundle_len_offset = caller_start.checked_add(caller_len)?;
        let bundle_len = usize::try_from(u32::from_le_bytes(
            frame.get(bundle_len_offset..bundle_len_offset + 4)?.try_into().ok()?,
        ))
        .ok()?;
        let bundle_start = bundle_len_offset + 4;
        let bundle_end = bundle_start.checked_add(bundle_len)?;
        let bundle = frame.get(bundle_start..bundle_end)?;
        if caller_len == 0
            || bundle_len > charlotte_launch::operations_bundle::MAX_BUNDLE_LEN
            || frame.len() != bundle_end
            || charlotte_launch::operations_bundle::decode(bundle).is_none()
        {
            return None;
        }
        Some(Request {
            session,
            request_id,
            caller: frame.get(caller_start..bundle_len_offset)?.to_vec(),
            bundle: bundle.to_vec(),
        })
    }

    pub fn encode_reply(session: u64, request_id: u64, result: i64) -> alloc::vec::Vec<u8> {
        super::rdeploy::encode_reply(session, request_id, result)
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, i64)> {
        if frame.len() != 25 || frame[0] != TAG_REPLY {
            return None;
        }
        Some((
            u64::from_le_bytes(frame[1..9].try_into().ok()?),
            u64::from_le_bytes(frame[9..17].try_into().ok()?),
            i64::from_le_bytes(frame[17..25].try_into().ok()?),
        ))
    }
}

/// Correlated follower-to-leader shutdown-intent submission.
pub mod rshutdown {
    pub const TAG_REQUEST: u8 = 0x1d;
    pub const TAG_REPLY: u8 = 0x1e;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Request {
        pub session: u64,
        pub request_id: u64,
        pub caller: alloc::vec::Vec<u8>,
        pub envelope: alloc::vec::Vec<u8>,
    }

    pub fn encode_request(request: &Request) -> Option<alloc::vec::Vec<u8>> {
        if request.caller.is_empty()
            || request.caller.len() > 255
            || charlotte_launch::shutdown::decode(&request.envelope).is_none()
        {
            return None;
        }
        let mut frame = alloc::vec::Vec::with_capacity(
            17 + request.caller.len() + charlotte_launch::shutdown::ENCODED_LEN,
        );
        frame.extend_from_slice(&request.session.to_le_bytes());
        frame.extend_from_slice(&request.request_id.to_le_bytes());
        frame.push(request.caller.len() as u8);
        frame.extend_from_slice(&request.caller);
        frame.extend_from_slice(&request.envelope);
        Some(frame)
    }

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 18 || frame[0] != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let request_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = usize::from(frame[17]);
        let caller_start = 18usize;
        let envelope_start = caller_start.checked_add(caller_len)?;
        let envelope = frame.get(envelope_start..)?;
        if caller_len == 0
            || envelope.len() != charlotte_launch::shutdown::ENCODED_LEN
            || charlotte_launch::shutdown::decode(envelope).is_none()
        {
            return None;
        }
        Some(Request {
            session,
            request_id,
            caller: frame.get(caller_start..envelope_start)?.to_vec(),
            envelope: envelope.to_vec(),
        })
    }

    pub fn encode_reply(session: u64, request_id: u64, result: i64) -> alloc::vec::Vec<u8> {
        super::rdeploy::encode_reply(session, request_id, result)
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, i64)> {
        if frame.len() != 25 || frame[0] != TAG_REPLY {
            return None;
        }
        Some((
            u64::from_le_bytes(frame[1..9].try_into().ok()?),
            u64::from_le_bytes(frame[9..17].try_into().ok()?),
            i64::from_le_bytes(frame[17..25].try_into().ok()?),
        ))
    }
}

/// Correlated follower-to-leader ingress-policy submission.
pub mod ringress_policy {
    pub const TAG_REQUEST: u8 = 0x1f;
    pub const TAG_REPLY: u8 = 0x20;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Request {
        pub session: u64,
        pub request_id: u64,
        pub caller: alloc::vec::Vec<u8>,
        pub envelope: alloc::vec::Vec<u8>,
    }

    pub fn encode_request(request: &Request) -> Option<alloc::vec::Vec<u8>> {
        if request.caller.is_empty()
            || request.caller.len() > 255
            || charlotte_launch::ingress_policy::decode(&request.envelope).is_none()
        {
            return None;
        }
        let mut frame =
            alloc::vec::Vec::with_capacity(17 + request.caller.len() + request.envelope.len());
        frame.extend_from_slice(&request.session.to_le_bytes());
        frame.extend_from_slice(&request.request_id.to_le_bytes());
        frame.push(request.caller.len() as u8);
        frame.extend_from_slice(&request.caller);
        frame.extend_from_slice(&request.envelope);
        Some(frame)
    }

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 18 || frame[0] != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let request_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = usize::from(frame[17]);
        let caller_start = 18usize;
        let envelope_start = caller_start.checked_add(caller_len)?;
        let envelope = frame.get(envelope_start..)?;
        if caller_len == 0 || charlotte_launch::ingress_policy::decode(envelope).is_none() {
            return None;
        }
        Some(Request {
            session,
            request_id,
            caller: frame.get(caller_start..envelope_start)?.to_vec(),
            envelope: envelope.to_vec(),
        })
    }

    pub fn encode_reply(session: u64, request_id: u64, result: i64) -> alloc::vec::Vec<u8> {
        super::rdeploy::encode_reply(session, request_id, result)
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, i64)> {
        if frame.len() != 25 || frame[0] != TAG_REPLY {
            return None;
        }
        Some((
            u64::from_le_bytes(frame[1..9].try_into().ok()?),
            u64::from_le_bytes(frame[9..17].try_into().ok()?),
            i64::from_le_bytes(frame[17..25].try_into().ok()?),
        ))
    }
}

/// Closes request-scoped IPC attachments when dropped.
///
/// Raw service loops have many exit paths (`continue`, bad-opcode replies,
/// early returns); a guard releases the attachments on all of them. Capability
/// ids are monotonic, so closing an attachment an arm already consumed is a
/// harmless no-op. Services that deliberately retain an attachment must not
/// use this.
#[must_use]
pub struct RequestAttachments {
    memory: u64,
    connection: u64,
}

impl RequestAttachments {
    pub fn new(memory: u64, connection: u64) -> Self {
        Self {
            memory,
            connection,
        }
    }

    pub fn memory_only(memory: u64) -> Self {
        Self {
            memory,
            connection: 0,
        }
    }

    pub fn connection_only(connection: u64) -> Self {
        Self {
            memory: 0,
            connection,
        }
    }
}

impl Drop for RequestAttachments {
    fn drop(&mut self) {
        if self.memory != 0 {
            catten_syscall::memory_close(self.memory);
        }
        if self.connection != 0 {
            catten_syscall::ipc_close(self.connection);
        }
    }
}
