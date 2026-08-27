//! Shared protocol definitions for the reference CharlotteOS services.
//!
//! This is the userspace half of the Phase 3 name-service architecture: the
//! kernel moves opaque capabilities, while interface ids, opcodes, names,
//! generations, and lookup policy are defined here.
#![no_std]

extern crate alloc;

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

/// Owned application-side wrapper for the capability-grant controller.
pub mod grant_client;
/// Owned application-side wrappers for the Kafka service protocol.
pub mod kafka_client;
/// Owned application-side wrappers for the S3 service protocol.
pub mod s3_client;
/// Verified, owned TLS client transport shared by network services.
pub mod tls_client;

/// Disk-backed Raft persistent state and log store on top of the object store.
pub mod disk_raft;

/// Persistent, cluster-scoped node identity (`{mnemonic}:{token}`).
pub mod node_identity;

pub mod broker;
/// Replicated name catalog: the Raft state machine for the distributed name
/// service.
pub mod name_catalog;

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
            || rights & !charlotte_launch::deployment::CLIENT_RIGHTS != 0
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
            || rights & !charlotte_launch::deployment::CLIENT_RIGHTS != 0
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

/// Name-service protocol (`charlotte-protocol-name` v1).
pub mod ns {
    /// Interface id: "NAME".
    pub const INTERFACE: u64 = super::name(b"NAME");
    pub const VERSION: u32 = 1;

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
    /// Access denied: the caller's interim bearer key did not match the
    /// service's registered key. This gate is not principal-based policy.
    pub const ERR_ACCESS_DENIED: i64 = -4;

    /// Register under a short name with an interim bearer key.  `arg0` = packed
    /// name; the call must attach a re-delegable connection.  If a
    /// memory object is attached, its first 8 bytes are the access key
    /// (0 = public, no gating).  Reply = new generation.
    pub const OP_REGISTER_KEYED: u32 = 5;
    /// Look up a short-named service with an interim bearer key.  `arg0` = packed
    /// name.  If a memory object is attached, its first 8 bytes are the
    /// access key; the stored registration's key must match (or be 0 for
    /// a public service).  Reply = generation + attenuated connection.
    pub const OP_LOOKUP_KEYED: u32 = 6;
    /// Best-effort short-name lookup for optional dependencies. Unlike
    /// [`OP_LOOKUP`], this replies with [`ERR_NOT_FOUND`] immediately rather
    /// than retaining the call until a future registration.
    pub const OP_TRY_LOOKUP: u32 = 7;
    /// Unpublish a short-named service while retaining its generation
    /// tombstone. `arg0` = packed name. Subsequent lookups defer until the
    /// replacement registers, at which point the generation advances
    /// normally. Existing connections retain their normal endpoint lifetime.
    pub const OP_UNREGISTER: u32 = 8;

    /// Reply moves a page snapshot of the registry: three little-endian u32
    /// words (`STATUS_OFFSET_MAGIC/REGISTERED/PENDING`), then one record per
    /// registered service of `[len:u8][name bytes]`. The scalar result is the
    /// snapshot byte length.
    pub const OP_STATUS: u32 = 9;
    /// Unpublish only the exact short-name generation supplied in the first
    /// eight bytes of an attached memory object. A stale request returns
    /// [`ERR_NOT_FOUND`] and leaves a replacement untouched.
    pub const OP_UNREGISTER_GENERATION: u32 = 10;

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

    pub const STATUS_OFFSET_MAGIC: u32 = 0;
    pub const STATUS_OFFSET_REGISTERED: u32 = 1;
    pub const STATUS_OFFSET_PENDING: u32 = 2;
    pub const STATUS_HEADER_BYTES: usize = 3 * core::mem::size_of::<u32>();
    /// `"NSST"` LE.
    pub const STATUS_MAGIC: u32 = 0x5453_534e;
    /// `"AUA1"` little-endian, leading an authorization-audit snapshot.
    pub const AUDIT_MAGIC: u32 = 0x3141_5541;

    /// Read a u64 access key from a memory object, or 0 if none.
    /// Consumes (unmaps and closes) the memory cap on success.
    /// # Safety
    ///
    /// `memory_cap` must name a mapped request object whose first word follows
    /// the service protocol's access-key layout.
    pub unsafe fn read_access_key(memory_cap: u64) -> u64 {
        if memory_cap == 0 {
            return 0;
        }
        if catten_syscall::memory_map(memory_cap, super::NAME_SCRATCH_VADDR, false) != 0 {
            catten_syscall::memory_close(memory_cap);
            return 0;
        }
        let key = unsafe { core::ptr::read_volatile(super::NAME_SCRATCH_VADDR as *const u64) };
        catten_syscall::memory_unmap(memory_cap);
        catten_syscall::memory_close(memory_cap);
        key
    }
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
    /// teardown (architecture doc §13, success criterion 9). Sent, not
    /// called: there is no reply.
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

    /// Direct-network EtherType retained for the isolated generic Raft
    /// fixture. The operational DNS-owned node uses relmsg and does not
    /// install this route in the frame demultiplexer.
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
/// `Move` on reply for recv).
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
    pub const OP_ACCEPT: u32 = 5;
    pub const OP_SEND: u32 = 6;
    pub const OP_RECV: u32 = 7;
    pub const OP_CLOSE: u32 = 8;
    pub const OP_FRAME: u32 = 9;
    /// Reply moves a page holding the packed `TcpipStatus` snapshot
    /// (`crate::socket::STATUS_*` layout).
    pub const OP_STATUS: u32 = 10;

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
    pub const STATUS_WORDS: usize = 9;
    pub const STATUS_MAGIC: u32 = 0x5443_5053;

    pub const ERR_TOO_MANY_SOCKETS: i64 = -1;
    pub const ERR_BAD_SOCKET: i64 = -2;
    pub const ERR_CONNECTION_REFUSED: i64 = -3;
    pub const ERR_NOT_CONNECTED: i64 = -4;
    pub const ERR_WOULD_BLOCK: i64 = -5;
    pub const ERR_BAD_DOMAIN: i64 = -6;
    pub const ERR_BAD_OPCODE: i64 = -7;

    pub const DOMAIN_TCP: u64 = 1;
    pub const DOMAIN_UDP: u64 = 2;

    pub const MAX_SOCKETS: usize = 16;

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

        /// Wait for and own one receive page from tcpip.
        pub fn receive(&self) -> Result<Option<ReceivedChunk>, SocketError> {
            let result = self.service.call(OP_RECV, self.id())?.wait().map_err(SocketError::Ipc)?;
            Self::decode_receive(result)
        }

        /// Poll for one receive page, returning `RetryExhausted` after a
        /// bounded wait. Dropping the pending call cancels it; dropping or
        /// closing this socket then releases tcpip's retained receive slot.
        pub fn receive_timeout(
            &self,
            attempts: usize,
            retry_ms: u64,
        ) -> Result<Option<ReceivedChunk>, SocketError> {
            let mut pending = self.service.call(OP_RECV, self.id())?;
            for _ in 0..attempts {
                if let Some(result) = pending.poll().map_err(SocketError::Ipc)? {
                    return Self::decode_receive(result);
                }
                super::sleep_ms(retry_ms);
            }
            Err(SocketError::RetryExhausted)
        }

        fn decode_receive(result: CallResult) -> Result<Option<ReceivedChunk>, SocketError> {
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
            let call = match self.service.call(OP_CLOSE, id) {
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

    pub const ERR_UNAVAILABLE: i64 = -1;
    pub const ERR_BAD_OPCODE: i64 = -2;
}

/// Frame demultiplexer status protocol.
pub mod frouter {
    pub const INTERFACE: u64 = super::name(b"FROUTER");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"frouter");

    pub const OP_STATUS: u32 = 1;

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

    pub const ERR_OK: i64 = 0;
    pub const ERR_NOT_FOUND: i64 = 1;
    pub const ERR_NO_SPACE: i64 = 2;
    pub const ERR_INVALID_ID: i64 = 3;
    pub const ERR_IO_ERROR: i64 = 4;
    pub const ERR_EXISTS: i64 = 5;
    pub const ERR_TOO_LARGE: i64 = 6;
    pub const EXECUTABLE_ECHO_ID: u64 = 0xffff_0000_0000_0001;
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

/// Reliable Message Layer protocol (`charlotte-protocol-relmsg` v2).
///
/// Exposes sequenced, acknowledged, retransmitted message delivery.
/// Clients send messages addressed to peer service names; the RML
/// encapsulates them in `charlotte-protocol-msg` frames and delivers
/// them via the NIC driver (or directly for same-machine peers).
pub mod relmsg {
    pub const INTERFACE: u64 = super::name(b"RELMSG");
    pub const VERSION: u32 = 2;
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
    /// Maximum application message payload in bytes. Messages larger than one
    /// frame are fragmented across multiple `charlotte-protocol-msg` frames
    /// (the frame payload limit is `MAX_PAYLOAD_SIZE`); 65535 is the u16
    /// message-length ceiling in the address/length packing.
    pub const MAX_MSG: usize = 65535;
    pub const RETRANSMIT_MS: u64 = 200;
    /// Bound one stop-and-wait transmission to two seconds. Raft retries
    /// heartbeats and replication at its own layer; retaining a lost
    /// heartbeat for 30 seconds would monopolize the peer's only in-flight
    /// slot beyond DNS's five-second remote-call deadline.
    pub const MAX_RETRIES: u32 = 10;
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
}

/// Distributed name service (`charlotte-protocol-dns` v1).
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
    pub const INTERFACE: u64 = super::name(b"DNS ");
    pub const VERSION: u32 = 3;
    pub const NAME: u64 = super::name(b"dns");

    pub const OP_REGISTER: u32 = 1;
    pub const OP_LOOKUP: u32 = 2;
    pub const OP_STATUS: u32 = 3;
    pub const OP_SHUTDOWN: u32 = 4;
    /// Remote invocation. `arg0` is the packed target service name; the
    /// attached memory object holds `[target_opcode:u32 LE][arg:i64 LE]`. The
    /// reply is the target service's scalar result (deferred until the remote
    /// call completes).
    pub const OP_CALL: u32 = 5;
    /// Dump the replicated catalog. Reply moves a page snapshot: one u32
    /// count, then per entry
    /// `[name_len:u8][name][node_len:u8][node][generation:u64 LE]`. The
    /// scalar result is the snapshot byte length.
    pub const OP_CATALOG: u32 = 6;
    pub const CATALOG_HEADER_BYTES: usize = core::mem::size_of::<u32>();
    /// Replicate an exact-generation tombstone for a locally hosted service.
    /// `arg0` is the packed name and the first eight bytes of the attached
    /// memory object contain the expected distributed generation. The reply
    /// is that generation on success.
    pub const OP_UNREGISTER: u32 = 7;
    /// Replicate a deployment record. `arg0` is the packed artifact name; the
    /// attached memory object holds
    /// `[object_id:u64 LE][node_key:u64 LE][artifact_sha256:32]`. The digest
    /// pins this generation to one immutable, blessed ELF; the reply is the
    /// committed deployment generation.
    pub const OP_DEPLOY: u32 = 8;
    /// Query the deployment record for `arg0` (packed artifact name). The
    /// reply moves a page holding
    /// `[generation:u64 LE][object_id:u64 LE][node_key:u64 LE]
    /// [artifact_sha256:32]`,
    /// or is `ERR_NOT_FOUND` when the artifact has never been deployed.
    /// Answered from locally applied cluster state (the caller polls).
    pub const OP_DEPLOY_QUERY: u32 = 9;
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
    /// The request may have executed remotely, but no authoritative reply
    /// arrived before its deadline. Callers must not blindly retry a
    /// non-idempotent operation.
    pub const ERR_UNCERTAIN: i64 = -5;
    pub const ERR_BUSY: i64 = -6;
    pub const ERR_STALE_GENERATION: i64 = -7;
    pub const ERR_UNTRUSTED_KEY: i64 = -8;
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
    pub const VERSION: u32 = 1;
    /// The service's short name (packed LE).
    pub const NAME: u64 = super::name(b"ctl");

    /// Upload an artifact. `arg0` is the packed artifact name; the attached
    /// memory object holds `[artifact_len:u64 LE][artifact]`, where the
    /// artifact is a note-signed ELF (`.note.charlotte-sig`) produced
    /// off-cluster with `tools/cluster-sign elf-sign` and the cluster's
    /// private key. The service stores it as-is at the artifact's derived
    /// id; nodes validate the signature against the cluster public key at
    /// pickup. The reply is the object id.
    pub const OP_UPLOAD: u32 = 1;
    /// Deploy an artifact to a node. `arg0` is the packed artifact name; the
    /// attached memory object holds `[node_key:u64 LE]`. The service derives
    /// the object id and submits the assignment through the local dns; the
    /// reply is the committed manifest generation (deferred until it has
    /// replicated).
    pub const OP_DEPLOY: u32 = 2;
    /// Query the deployment manifest. `arg0` is the packed artifact name; the
    /// reply moves the 56-byte deployment record
    /// `[generation][object_id][node_key][artifact_sha256]`, or is
    /// `ERR_NOT_FOUND`.
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
    /// Join the cluster on the local network segment. The service asks the
    /// local discovery service for the cluster's leader (or a follower that
    /// redirects towards it), then asks that leader's DNS-owned Raft
    /// administration endpoint to admit this node. The reply is the committed JOIN log index, or a
    /// negative error (`ERR_NO_CLUSTER` when no peer on the segment reports a
    /// cluster).
    pub const OP_JOIN: u32 = 7;

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
}

/// Remote-invocation wire protocol carried over the reliable message layer.
///
/// The distributed name service relays `OP_CALL`s to the node that hosts the
/// target service. Calls are identified by caller node, caller DNS session,
/// and monotonic call id. The transport prepends the type tag:
/// ```text
/// request: 0x10 | session:u64 | call_id:u64 | caller_len:u8 | caller |
///          name_len:u8 | name | target_generation:u64 | opcode:u32 | arg:i64
/// reply:   0x11 | session:u64 | call_id:u64 | target_generation:u64 | result:i64
/// ```
pub mod rcall {
    pub const TAG_REQUEST: u8 = 0x10;
    pub const TAG_REPLY: u8 = 0x11;

    /// Encode the request body *without* the type tag; the transport adds it.
    pub fn encode_request(
        session: u64,
        call_id: u64,
        caller: &[u8],
        name: &[u8],
        target_generation: u64,
        opcode: u32,
        arg: i64,
    ) -> alloc::vec::Vec<u8> {
        let caller_len = caller.len().min(255);
        let name_len = name.len().min(255);
        let mut frame = alloc::vec::Vec::with_capacity(18 + caller_len + name_len + 20);
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&call_id.to_le_bytes());
        frame.push(caller_len as u8);
        frame.extend_from_slice(&caller[..caller_len]);
        frame.push(name_len as u8);
        frame.extend_from_slice(&name[..name_len]);
        frame.extend_from_slice(&target_generation.to_le_bytes());
        frame.extend_from_slice(&opcode.to_le_bytes());
        frame.extend_from_slice(&arg.to_le_bytes());
        frame
    }

    pub type Request = (u64, u64, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>, u64, u32, i64);

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 1 + 8 + 8 + 1 + 1 + 8 + 4 + 8 {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let call_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = frame[17] as usize;
        let caller_off = 18;
        if frame.len() < caller_off + caller_len + 1 {
            return None;
        }
        let caller = frame[caller_off..caller_off + caller_len].to_vec();
        let name_len_off = caller_off + caller_len;
        let name_len = frame[name_len_off] as usize;
        let name_off = name_len_off + 1;
        if frame.len() < name_off + name_len + 20 {
            return None;
        }
        let name = frame[name_off..name_off + name_len].to_vec();
        let op_off = name_off + name_len;
        let target_generation = u64::from_le_bytes(frame[op_off..op_off + 8].try_into().ok()?);
        let opcode = u32::from_le_bytes(frame[op_off + 8..op_off + 12].try_into().ok()?);
        let arg = i64::from_le_bytes(frame[op_off + 12..op_off + 20].try_into().ok()?);
        Some((session, call_id, caller, name, target_generation, opcode, arg))
    }

    /// Encode the reply body *without* the type tag; the transport adds it.
    pub fn encode_reply(
        session: u64,
        call_id: u64,
        target_generation: u64,
        result: i64,
    ) -> alloc::vec::Vec<u8> {
        let mut frame = alloc::vec::Vec::with_capacity(32);
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&call_id.to_le_bytes());
        frame.extend_from_slice(&target_generation.to_le_bytes());
        frame.extend_from_slice(&result.to_le_bytes());
        frame
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, u64, i64)> {
        if frame.len() < 33 {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let call_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let target_generation = u64::from_le_bytes(frame[17..25].try_into().ok()?);
        let result = i64::from_le_bytes(frame[25..33].try_into().ok()?);
        Some((session, call_id, target_generation, result))
    }
}

/// Correlated follower-to-leader catalog queries. Followers must not answer
/// `dns::OP_LOOKUP` or choose an `OP_CALL` target from their local applied
/// state, because that state can be stale. The leader replies only after its
/// Graft read barrier admits a linearizable query.
pub mod rquery {
    pub const TAG_REQUEST: u8 = 0x12;
    pub const TAG_REPLY: u8 = 0x13;

    pub fn encode_request(
        session: u64,
        query_id: u64,
        caller: &[u8],
        name: &[u8],
    ) -> alloc::vec::Vec<u8> {
        let caller_len = caller.len().min(255);
        let name_len = name.len().min(255);
        let mut frame = alloc::vec::Vec::with_capacity(18 + caller_len + name_len);
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&query_id.to_le_bytes());
        frame.push(caller_len as u8);
        frame.extend_from_slice(&caller[..caller_len]);
        frame.push(name_len as u8);
        frame.extend_from_slice(&name[..name_len]);
        frame
    }

    pub type Request = (u64, u64, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>);

    pub fn decode_request(frame: &[u8]) -> Option<Request> {
        if frame.len() < 19 {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let query_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let caller_len = frame[17] as usize;
        let caller_off = 18;
        if frame.len() < caller_off + caller_len + 1 {
            return None;
        }
        let caller = frame[caller_off..caller_off + caller_len].to_vec();
        let name_len_off = caller_off + caller_len;
        let name_len = frame[name_len_off] as usize;
        let name_off = name_len_off + 1;
        if frame.len() != name_off + name_len {
            return None;
        }
        Some((session, query_id, caller, frame[name_off..].to_vec()))
    }

    pub fn encode_reply(
        session: u64,
        query_id: u64,
        status: i64,
        generation: u64,
        node: &[u8],
    ) -> alloc::vec::Vec<u8> {
        let node_len = node.len().min(255);
        let mut frame = alloc::vec::Vec::with_capacity(33 + node_len);
        frame.extend_from_slice(&session.to_le_bytes());
        frame.extend_from_slice(&query_id.to_le_bytes());
        frame.extend_from_slice(&status.to_le_bytes());
        frame.extend_from_slice(&generation.to_le_bytes());
        frame.push(node_len as u8);
        frame.extend_from_slice(&node[..node_len]);
        frame
    }

    pub type Reply = (u64, u64, i64, u64, alloc::vec::Vec<u8>);

    pub fn decode_reply(frame: &[u8]) -> Option<Reply> {
        if frame.len() < 34 {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let query_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        let status = i64::from_le_bytes(frame[17..25].try_into().ok()?);
        let generation = u64::from_le_bytes(frame[25..33].try_into().ok()?);
        let node_len = frame[33] as usize;
        if frame.len() != 34 + node_len {
            return None;
        }
        Some((session, query_id, status, generation, frame[34..].to_vec()))
    }
}

/// Block until a pending call completes, returning
/// `(result, returned_connection_cap)`.
///
/// `max_spins` is retained for source compatibility; reply waiting is now
/// scheduler-backed and has no arbitrary boot-time deadline.
/// # Safety
///
/// `call` must be a live pending-call capability owned by the caller.
pub unsafe fn wait_reply(call: u64, _max_spins: u64) -> (i64, u64) {
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

    let (generation, connection) = unsafe { wait_reply(call, 0) };
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

/// Block the calling userspace thread between low-frequency reply polls.
pub fn sleep_ms(milliseconds: u64) {
    if let Ok(timer) = catten_rt::owned::Completion::timer(milliseconds) {
        let _ = timer.wait();
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

/// Wait for the local boot-ready marker using ownership-aware IPC.
pub fn wait_for_local_ready_owned(ns_connection: catten_rt::owned::ConnectionRef<'_>) -> bool {
    wait_for_registered_name_owned(ns_connection, charlotte_launch::LOCAL_READY_NAME).is_some()
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
pub mod runregister {
    pub const TAG_REQUEST: u8 = 0x14;

    pub fn encode_request(owner: &[u8], name: &[u8], generation: u64) -> alloc::vec::Vec<u8> {
        let owner_len = owner.len().min(255);
        let name_len = name.len().min(255);
        let mut frame = alloc::vec::Vec::with_capacity(2 + owner_len + name_len + 8);
        frame.push(owner_len as u8);
        frame.extend_from_slice(&owner[..owner_len]);
        frame.push(name_len as u8);
        frame.extend_from_slice(&name[..name_len]);
        frame.extend_from_slice(&generation.to_le_bytes());
        frame
    }

    pub fn decode_request(frame: &[u8]) -> Option<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>, u64)> {
        if frame.len() < 12 || frame[0] != TAG_REQUEST {
            return None;
        }
        let owner_len = frame[1] as usize;
        let owner_off = 2;
        if frame.len() < owner_off + owner_len + 1 + 8 {
            return None;
        }
        let owner = frame[owner_off..owner_off + owner_len].to_vec();
        let name_len_off = owner_off + owner_len;
        let name_len = frame[name_len_off] as usize;
        let name_off = name_len_off + 1;
        if frame.len() != name_off + name_len + 8 {
            return None;
        }
        let name = frame[name_off..name_off + name_len].to_vec();
        let generation = u64::from_le_bytes(frame[name_off + name_len..].try_into().ok()?);
        Some((owner, name, generation))
    }
}

/// Remote-registration wire protocol carried over the reliable message layer.
///
/// A service can be hosted on any node, but only the Raft leader may commit
/// catalog entries. When a follower's dns receives `OP_REGISTER` for a
/// service hosted on its own node, it relays a register request to the
/// leader; the leader commits the two-phase register/activate pair naming the
/// follower's node as owner, and replies with the committed generation. The
/// frame carries the owner (hosting node) and the service name; the reply
/// adds the generation. The transport prepends the type tag:
/// ```text
/// request: 0x15 | owner_len:u8 | owner | name_len:u8 | name
/// reply:   0x16 | owner_len:u8 | owner | name_len:u8 | name | generation:u64
/// ```
pub mod rregister {
    pub const TAG_REQUEST: u8 = 0x15;
    pub const TAG_REPLY: u8 = 0x16;

    pub fn encode_request(owner: &[u8], name: &[u8]) -> alloc::vec::Vec<u8> {
        let owner_len = owner.len().min(255);
        let name_len = name.len().min(255);
        let mut frame = alloc::vec::Vec::with_capacity(1 + owner_len + name_len);
        frame.push(owner_len as u8);
        frame.extend_from_slice(&owner[..owner_len]);
        frame.push(name_len as u8);
        frame.extend_from_slice(&name[..name_len]);
        frame
    }

    pub fn decode_request(frame: &[u8]) -> Option<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)> {
        if frame.len() < 3 || frame[0] != TAG_REQUEST {
            return None;
        }
        let owner_len = frame[1] as usize;
        let owner_off = 2;
        if frame.len() < owner_off + owner_len + 1 {
            return None;
        }
        let owner = frame[owner_off..owner_off + owner_len].to_vec();
        let name_len_off = owner_off + owner_len;
        let name_len = frame[name_len_off] as usize;
        let name_off = name_len_off + 1;
        if frame.len() != name_off + name_len {
            return None;
        }
        let name = frame[name_off..].to_vec();
        Some((owner, name))
    }

    pub fn encode_reply(owner: &[u8], name: &[u8], generation: u64) -> alloc::vec::Vec<u8> {
        let owner_len = owner.len().min(255);
        let name_len = name.len().min(255);
        let mut frame = alloc::vec::Vec::with_capacity(1 + owner_len + name_len + 8);
        frame.push(owner_len as u8);
        frame.extend_from_slice(&owner[..owner_len]);
        frame.push(name_len as u8);
        frame.extend_from_slice(&name[..name_len]);
        frame.extend_from_slice(&generation.to_le_bytes());
        frame
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>, u64)> {
        if frame.len() < 11 || frame[0] != TAG_REPLY {
            return None;
        }
        let owner_len = frame[1] as usize;
        let owner_off = 2;
        if frame.len() < owner_off + owner_len + 1 + 8 {
            return None;
        }
        let owner = frame[owner_off..owner_off + owner_len].to_vec();
        let name_len_off = owner_off + owner_len;
        let name_len = frame[name_len_off] as usize;
        let name_off = name_len_off + 1;
        if frame.len() != name_off + name_len + 8 {
            return None;
        }
        let name = frame[name_off..name_off + name_len].to_vec();
        let generation = u64::from_le_bytes(frame[name_off + name_len..].try_into().ok()?);
        Some((owner, name, generation))
    }
}
