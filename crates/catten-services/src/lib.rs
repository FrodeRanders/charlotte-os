//! Shared protocol definitions for the reference CharlotteOS services.
//!
//! This is the userspace half of the Phase 3 name-service architecture: the
//! kernel moves opaque capabilities, while interface ids, opcodes, names,
//! generations, and lookup policy are defined here.
#![no_std]

extern crate alloc;

/// Disk-backed Raft persistent state and log store on top of the object store.
pub mod disk_raft;

/// Persistent, cluster-scoped node identity (`{mnemonic}:{token}`).
pub mod node_identity;

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
/// (`0x11000`), launch input (`0x12000`), and heap (`0x13000..0x20000`).
pub const NAME_SCRATCH_VADDR: usize = 0x0000_0000_0010_0000;

/// Maximum memory-carried name length (fits one page with room to spare).
pub const MAX_NAME_LEN: usize = 256;

/// Name-service protocol (`charlotte-protocol-name` v1).
pub mod ns {
    /// Interface id: "NAME".
    pub const INTERFACE: u64 = super::name(b"NAME");
    pub const VERSION: u32 = 1;

    /// Register a service under a short (<= 8 byte) name. `arg0` = packed
    /// name; the call must attach a re-delegable connection
    /// (`SEND | CALL | MINT_CONNECTION`) to the service's endpoint. Reply
    /// result = new instance generation (>= 1).
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
    /// Access denied: the caller's access key did not match the service's
    /// registered key (policy gating).
    pub const ERR_ACCESS_DENIED: i64 = -4;

    /// Register under a short name with an access key.  `arg0` = packed
    /// name; the call must attach a re-delegable connection.  If a
    /// memory object is attached, its first 8 bytes are the access key
    /// (0 = public, no gating).  Reply = new generation.
    pub const OP_REGISTER_KEYED: u32 = 5;
    /// Look up a short-named service with an access key.  `arg0` = packed
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

/// Raft consensus protocol (`charlotte-protocol-raft` v1).
///
/// The opcodes are deliberately aligned with the `graft` crate's message
/// types so that the wire-level encoding stays identical across the
/// standard-library (Tokio) and CharlotteOS builds.
pub mod raft {
    pub const INTERFACE: u64 = super::name(b"RAFT");
    pub const VERSION: u32 = 1;

    pub const OP_VOTE_REQUEST: u32 = 1;
    pub const OP_APPEND_ENTRIES: u32 = 2;
    pub const OP_INSTALL_SNAPSHOT: u32 = 3;
    pub const OP_CLIENT_COMMAND: u32 = 4;
    pub const OP_CLIENT_QUERY: u32 = 5;
    pub const OP_ADD_SERVER: u32 = 6;
    pub const OP_REMOVE_SERVER: u32 = 7;
    pub const OP_STATUS: u32 = 8;

    pub const ERR_NOT_LEADER: i64 = -1;
    pub const ERR_LOG_INCONSISTENCY: i64 = -2;
    pub const ERR_STALE_TERM: i64 = -3;
    pub const ERR_NOT_FOUND: i64 = -4;
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
/// connection capability. Data payloads use memory-object transfer
/// (`Move` for send, `Move` on reply for recv).
pub mod socket {
    pub const INTERFACE: u64 = super::name(b"SKT");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"tcpip");

    pub const OP_SOCKET: u32 = 1;
    pub const OP_CONNECT: u32 = 2;
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
    /// page). Offsets are in u32 words from the base of the reply page.
    pub const STATUS_OFFSET_IP: u32 = 0;
    pub const STATUS_OFFSET_RX_FRAMES: u32 = 1;
    pub const STATUS_OFFSET_TX_SENDS: u32 = 2;
    pub const STATUS_OFFSET_SOCKETS: u32 = 3;
    pub const STATUS_OFFSET_MAGIC: u32 = 4;
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

/// Reliable Message Layer protocol (`charlotte-protocol-relmsg` v1).
///
/// Exposes sequenced, acknowledged, retransmitted message delivery.
/// Clients send messages addressed to peer service names; the RML
/// encapsulates them in `charlotte-protocol-msg` frames and delivers
/// them via the NIC driver (or directly for same-machine peers).
pub mod relmsg {
    pub const INTERFACE: u64 = super::name(b"RELMSG");
    pub const VERSION: u32 = 1;
    pub const NAME: u64 = super::name(b"relmsg");

    pub const OP_SEND: u32 = 1;
    pub const OP_RECV: u32 = 2;
    pub const OP_STATUS: u32 = 3;
    /// Internal ingress used by the isolated NIC-receive pump.
    pub const OP_FRAME: u32 = 4;
    pub const OP_SHUTDOWN: u32 = 5;

    pub const ERR_PEER_UNREACHABLE: i64 = -1;
    pub const ERR_BAD_OPCODE: i64 = -2;
    pub const ERR_UNKNOWN: i64 = -3;
    pub const ERR_BUSY: i64 = -4;

    pub const MAX_PEERS: usize = 16;
    pub const MAX_MSG: usize = 1400;
    pub const RETRANSMIT_MS: u64 = 200;
    /// Permit peers that boot at different speeds to rendezvous without
    /// making an application-level send wait forever.
    pub const MAX_RETRIES: u32 = 150;
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
    /// Internal ingress used by the frame demultiplexer (frouter): delivers
    /// one raw frame whose EtherType matched `DISCO_ETHERTYPE`. The call
    /// attaches a **moved** memory object holding the frame; the service
    /// processes it and replies 0.
    pub const OP_FRAME: u32 = 5;

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
/// connection with the local name service once committed). `OP_LOOKUP`
/// answers from the replicated catalog: local names resolve to the local
/// name service, remote names report the hosting node. `OP_CALL` is the
/// remote-invocation entry point: it resolves a name through the catalog and
/// invokes it locally (when hosted here) or forwards it to the hosting node
/// over the reliable message layer.
pub mod dns {
    pub const INTERFACE: u64 = super::name(b"DNS ");
    pub const VERSION: u32 = 1;
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

    /// The name is registered on this node (lookup returns a connection).
    pub const RESULT_LOCAL: i64 = 0;
    /// The name is registered on a remote node (lookup returns its node id).
    pub const RESULT_REMOTE: i64 = 1;
    pub const ERR_NOT_FOUND: i64 = -1;
    pub const ERR_NOT_LEADER: i64 = -2;
    pub const ERR_BAD_OPCODE: i64 = -3;
    pub const ERR_TOO_LARGE: i64 = -4;
}

/// Remote-invocation wire protocol carried over the reliable message layer.
///
/// The distributed name service relays `OP_CALL`s to the node that hosts the
/// target service. The wire carries a monotonic call id so replies can be
/// matched to their requests. The transport prepends the type tag:
/// ```text
/// request: 0x10 | call_id:u64 | name_len:u8 | name | opcode:u32 | arg:i64
/// reply:   0x11 | call_id:u64 | result:i64
/// ```
pub mod rcall {
    pub const TAG_REQUEST: u8 = 0x10;
    pub const TAG_REPLY: u8 = 0x11;

    /// Encode the request body *without* the type tag; the transport adds it.
    pub fn encode_request(call_id: u64, name: &[u8], opcode: u32, arg: i64) -> alloc::vec::Vec<u8> {
        let mut frame = alloc::vec::Vec::with_capacity(8 + 1 + name.len() + 12);
        frame.extend_from_slice(&call_id.to_le_bytes());
        frame.push(name.len().min(255) as u8);
        frame.extend_from_slice(&name[..name.len().min(255)]);
        frame.extend_from_slice(&opcode.to_le_bytes());
        frame.extend_from_slice(&arg.to_le_bytes());
        frame
    }

    pub fn decode_request(frame: &[u8]) -> Option<(u64, alloc::vec::Vec<u8>, u32, i64)> {
        if frame.len() < 1 + 8 + 1 + 4 + 8 {
            return None;
        }
        let call_id = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let name_len = frame[9] as usize;
        let name_off = 10;
        if frame.len() < name_off + name_len + 12 {
            return None;
        }
        let name = frame[name_off..name_off + name_len].to_vec();
        let op_off = name_off + name_len;
        let opcode = u32::from_le_bytes(frame[op_off..op_off + 4].try_into().ok()?);
        let arg = i64::from_le_bytes(frame[op_off + 4..op_off + 12].try_into().ok()?);
        Some((call_id, name, opcode, arg))
    }

    /// Encode the reply body *without* the type tag; the transport adds it.
    pub fn encode_reply(call_id: u64, result: i64) -> alloc::vec::Vec<u8> {
        let mut frame = alloc::vec::Vec::with_capacity(16);
        frame.extend_from_slice(&call_id.to_le_bytes());
        frame.extend_from_slice(&result.to_le_bytes());
        frame
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, i64)> {
        if frame.len() < 17 {
            return None;
        }
        let call_id = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let result = i64::from_le_bytes(frame[9..17].try_into().ok()?);
        Some((call_id, result))
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
    let timer = catten_syscall::submit_timer(milliseconds);
    if timer == u64::MAX {
        return;
    }
    catten_syscall::wait(timer);
    catten_syscall::close(timer);
}

/// Block until the kernel has registered [`charlotte_launch::BOOT_DONE_NAME`]
/// in the name service, signalling that this node has finished its boot storm.
///
/// `ns::OP_LOOKUP` defers until the name is registered, so this returns as
/// soon as the kernel publishes the marker. Returns `false` only if the call
/// itself could not be made. Network-initiating services (cluster discovery,
/// reliable-message/Raft membership clients) must call this before starting
/// to communicate with other nodes.
pub fn wait_for_boot_done(ns_conn: u64) -> bool {
    let call = catten_syscall::ipc_scalar_call(
        ns_conn,
        ns::OP_LOOKUP,
        charlotte_launch::BOOT_DONE_NAME,
    );
    if call == 0 {
        return false;
    }
    let (generation, _) = unsafe { wait_reply(call, 0) };
    generation >= 1
}
