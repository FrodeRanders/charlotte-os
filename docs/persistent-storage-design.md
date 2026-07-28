# CharlotteOS Persistent Storage Architecture

**Status:** Implemented prototype; power-loss atomicity remains unvalidated\
**Audience:** CharlotteOS contributors\
**Purpose:** Define the persistent storage subsystem: block device protocol,
NVMe driver, and integration with Raft consensus.

---

## 1. Executive Summary

CharlotteOS now has a userspace NVMe driver, block protocol, object-store
prototype, and disk-backed implementations of Raft's `PersistentStateStore`
and `LogStore`. A QEMU boot test starts a single-voter Raft process, stops and
tears it down, then starts the same node identity again and verifies that its
term advances from the value recovered through the NVMe-backed object store.
This proves process-restart persistence in the test environment; it does not
yet prove power-loss atomicity or multi-machine recovery.

The persistent storage subsystem addresses this in layers:

1. **Block device protocol** — a crate defining the endpoint IPC interface
   between block device consumers (filesystems, Raft log stores) and block
   device drivers. Specifying this first keeps the driver and consumer
   implementations independent.

2. **NVMe driver** — a userspace service that discovers an NVMe controller
   via PCI, initializes the admin and I/O queues, and exposes the block
   device endpoint. NVMe is the natural first target: well-specified, PCIe,
   DMA-capable, and supported by QEMU.

3. **Disk-backed Raft state** — implementations of `PersistentStateStore`
   and `LogStore` use stable, namespaced object IDs and flush mutations before
   returning, giving a Raft node state that survives process restart.

4. **Filesystem service** (future) — a userspace server that consumes the
   block device protocol and exports file-level operations through its own
   endpoint.

---

## 2. Block Device Protocol

### 2.1 Interface identity

```
Interface: "BLOCK"  (packed as little-endian u64)
Version: 1
Service name: "blk0", "blk1", ... per device instance
```

### 2.2 Operations

| Opcode | Name | Description |
|--------|------|-------------|
| 1 | `OP_INFO` | Query block size, total block count, device identity |
| 2 | `OP_READ` | Read one or more blocks into a caller-provided memory object |
| 3 | `OP_WRITE` | Write one or more blocks from a caller-provided memory object |
| 4 | `OP_FLUSH` | Flush any write cache to durable media |
| 5 | `OP_TRIM` | Discard/deallocate a block range (optional, NVMe Dataset Management) |

### 2.3 Message patterns

**OP_INFO** — scalar call/reply. The reply scalar encodes block size (u32, low
32 bits) and total blocks (u32, high 32 bits). No memory object.

**OP_READ** — memory call with BorrowWrite attachment. The caller provides a
memory object large enough for the requested blocks. The driver writes
directly into it (DMA on NVMe). On reply, the borrow is revoked and the
caller reads the data.

**OP_WRITE** — memory call with BorrowRead attachment. The caller provides a
memory object containing the data to write. The driver reads from it (DMA on
NVMe). On reply, the borrow is revoked. The caller may then reuse or drop the
buffer.

**OP_FLUSH** — scalar call/reply. The driver ensures all previously written
data is durably stored. The reply scalar carries a status code.

**OP_TRIM** — scalar call with the start block and block count packed into
the argument u64. No memory object.

### 2.4 Error codes

| Code | Name | Description |
|------|------|-------------|
| 0 | OK | Success |
| 1 | IO_ERROR | Hardware or transport error |
| 2 | INVALID_RANGE | Requested blocks out of range |
| 3 | UNALIGNED | Buffer size not a multiple of block size |
| 4 | DEVICE_GONE | Device was removed or reset |

### 2.5 Why BorrowRead/BorrowWrite, not Move

READ uses BorrowWrite: the caller provides an empty buffer, the driver fills
it, the caller reads it. Move would require the driver to allocate a buffer,
fill it, and transfer ownership — but the caller already knows how much data
it wants. BorrowWrite is the natural "read into my buffer" semantic.

WRITE uses BorrowRead: the caller provides a buffer containing data. The
driver reads it for DMA. The caller retains ownership and can reuse the
buffer after the write completes.

Both are tied to the IPC call: revocation happens on reply, cancellation, or
driver death. No indefinite borrowing, no forgotten mappings.

---

## 3. NVMe Driver Design

### 3.1 Discovery and grant

The kernel PCI topology scanner discovers NVMe controllers by class code
(0x01, 0x08, 0x02). For each controller found:

1. Read BAR0 physical base (the NVMe controller registers, 64-bit BAR).
2. Read the MSI-X or INTx interrupt line.
3. Construct a `DriverGrant { mmio_phys_base, mmio_pages, intid }`.
4. Spawn the NVMe driver EL0 service with the grant.

The driver receives an MmioRegion capability covering the controller
registers and an Interrupt capability for completion queue doorbell
interrupts.

### 3.2 Initialisation sequence

Following the NVMe 1.4 specification (NVM Express Base Specification):

1. **Controller reset:** write 0 to CC.EN, wait for CSTS.RDY = 0.
2. **Configure controller:** set CC with the desired I/O queue entry size,
   enable the controller (CC.EN = 1), wait for CSTS.RDY = 1.
3. **Set up admin queue:** allocate memory for the Admin Submission Queue
   and Admin Completion Queue (two contiguous page-aligned regions). Write
   the ASQ and ACQ base addresses to the controller registers. The admin
   queue uses a fixed size (e.g., 32 entries).
4. **Identify controller:** submit an Identify command (CNS=1) via the
   admin SQ. Read the Identify Controller data structure to determine the
   number of namespaces and the maximum I/O queue entries.
5. **Create I/O completion queue:** allocate memory. Submit a Create I/O
   Completion Queue command via the admin SQ.
6. **Create I/O submission queue:** allocate memory. Submit a Create I/O
   Submission Queue command via the admin SQ.
7. **Identify namespace:** submit an Identify command (CNS=0) for namespace
   1. Read the Identify Namespace data structure to determine the block
   size (LBAF) and total block count (NSZE).

### 3.3 I/O path

**READ:** the driver receives an `OP_READ` call with a BorrowWrite memory
object. It:

1. Calculates the target physical address from the borrowed memory object
   (via `memory_get_phys` — the kernel returns the physical address of the
   mapped pages).
2. Constructs an NVM Read command with the starting LBA, block count, and
   a PRP (Physical Region Page) list pointing to the caller's physical
   pages.
3. Submits the command to the I/O Submission Queue (ring doorbell).
4. Retains the reply token.
5. When the I/O Completion Queue entry arrives (signalled by interrupt →
   CQ wake), the driver completes the reply.

**WRITE:** analogous, with an NVM Write command and a BorrowRead memory
object.

The implementation accepts transfers up to 512 pages (2 MiB with 4 KiB
pages). `memory_get_phys_page` exposes each frame authorized and retained by
the borrowed memory-object capability; the driver uses PRP1 for the first
frame, direct PRP2 for a two-page request, and a temporary one-page PRP list
for larger scatter/gather requests. The PRP-list capability is retained until
the matching command completion. Larger requests are rejected rather than
requiring chained PRP lists.

**FLUSH:** submits an NVM Flush command to the I/O SQ. Completes the reply
when the CQ entry arrives.

### 3.4 Completion model

The intended steady-state design binds an MSI/MSI-X interrupt capability to
the driver's CQ. When an I/O completion arrives:

1. The MSI-X interrupt fires → GIC/APIC interrupt → kernel defers a wake
   to the driver's CQ.
2. The driver's shard wakes from `cq_wait`.
3. The driver reads the I/O Completion Queue head doorbell to determine
   which completions arrived.
4. For each completion, it finds the corresponding retained reply token
   and completes it with the status.

The QEMU AArch64 bring-up path uses the GICv2m MSI frame exposed by
`virt,gic-version=3,msi=gicv2m`. The kernel allocates an SPI from that frame,
programs MSI-X table entry zero through the BAR named by the PCI capability,
and delegates only the resulting interrupt capability to the driver. The NVMe
I/O CQ enables interrupt vector zero. A bounded `cq_wait_timeout` remains as a
compatibility fallback for platforms where MSI-X setup is unavailable.

### 3.5 Memory allocation for queues

NVMe queue pages are page-aligned and currently fit in one page each. The
driver allocates them using `memory_alloc`, then maps them through its
delegated `DmaDomain` capability. The returned IOVA range is contiguous even
when the backing frames are not, so the driver builds PRP entries without
learning physical addresses.

### 3.6 DMA

The NVMe controller performs DMA directly to/from the caller's memory
objects (for READ/WRITE) and to/from the driver's queue memory (for SQ/CQ
entries). On the AArch64 ACPI path, the supervisor grants a `DmaDomain`
alongside MMIO and IRQ capabilities. `dma_map` validates the memory
capability's direction, pins every frame, installs SMMUv3 translations, and
returns an IOVA. `dma_unmap` synchronously invalidates the IOTLB before
unpinning. The device stream can otherwise address no RAM; its one additional
mapping is the page containing its MSI-X doorbell.

The current implementation targets coherent SMMUv3 systems described by ACPI
IORT. Non-coherent table walks, ATS/PRI, and multi-SMMU topologies are not yet
supported.

---

## 4. Disk-Backed Raft Persistent State

### 4.1 Existing interface (catten-graft)

The existring crate defines two traits:

```rust
trait LogStore {
    fn append(&mut self, entry: LogEntry) -> Result<u64, StoreError>;
    fn entry(&self, index: u64) -> Option<LogEntry>;
    fn last_index(&self) -> u64;
    fn last_term(&self) -> u64;
    fn truncate(&mut self, from_index: u64);
    fn snapshot_index(&self) -> u64;
    // ...
}

trait PersistentStateStore {
    fn current_term(&self) -> u64;
    fn set_current_term(&mut self, term: u64);
    fn voted_for(&self) -> Option<String>;
    fn set_voted_for(&mut self, candidate_id: Option<String>);
}
```

Both have in-memory implementations for storage-independent local testing and
object-store-backed implementations for durable deployments.

### 4.2 Disk-backed implementation

`DiskPersistentStateStore` and `DiskLogStore` connect to the `"obj"` service.
The cluster and node identities are hashed into a stable namespace containing
four object IDs: term/vote state, snapshot metadata, snapshot data, and log
entries. Stable creation (`OP_CREATE_AT`) means a replacement process opens
the same objects instead of allocating new ones. Each mutation writes its
serialized object and calls `FLUSH` before returning.

The Raft launch manifest selects the backend:

- `storage = 0` (or omitted): in-memory, used by the ordinary local multicore
  two-node election test;
- `storage = 1`: try persistent storage, with an explicit in-memory fallback;
- `storage = 2`: require persistent storage and wait for the object-store
  service through the name service.

Node ID, peer list, election timing, and storage policy remain supervisor-owned
launch configuration. They are not copied into the Raft log. The durable store
contains Raft protocol state and log/snapshot data, not the deployment
manifest.

On-disk format v3 has two checksummed superblock slots, a device-sized
allocation bitmap, and two mirrored banks of atomic object locators. Directory
capacity scales with the device (each bank uses about 1/2048 of its blocks,
bounded to 4,096 blocks). Each record has its own generation and checksum;
mount selects the newest valid copy. Versioned tombstones ensure that an older
live record cannot reappear after deletion. Every allocation begins with a
versioned header containing `header_len`, `header_blocks`, object generation,
exact and allocated lengths, data offset, up to 16 extent descriptors, and a
mandatory FNV-1a content hash. The reserved header space permits compatible
metadata growth.

Replacement is copy-on-write: the new data, header, and allocation state are
flushed before the directory locator changes; the old allocation is released
afterward. Mount validates reachable headers and reconstructs the bitmap,
which reclaims allocations abandoned before commit. Whole-object transfers
are split into NVMe requests of at most 512 KiB, so the NVMe request ceiling
is not a file, object, Raft-log, or snapshot-size limit. The service API
currently supplies lengths as 32-bit values, limiting one object to 4 GiB and
available disk space. The mirrored per-record directory resolves a torn write
by retaining the other valid generation; full device-level sudden-power-loss
guarantees still depend on truthful flush/FUA behaviour below the service. The
content hash detects accidental corruption; it does not authenticate
executable objects or other data.

---

## 5. Implementation Sequence

### Phase 1 — Protocol and skeleton (complete)

1. Create `charlotte-protocol-block` crate with the interface constants,
   opcodes, error codes, and helper functions.
2. Add `block` module to `catten-services/src/lib.rs` with protocol
   constants.
3. Create NVMe driver binary `catten-services/src/bin/nvme.rs` as a
   skeleton that registers with the name service and enters the unified
   shard wait — no NVMe hardware interaction yet.
4. Add the NVMe PCI device class matching and topology lookup to the
   kernel.
5. Wire up the driver spawn in kernel init.
6. Build and verify.

### Phase 2 — NVMe functional bringup (complete for QEMU prototype)

1. Implement controller reset and admin queue setup.
2. Implement Identify (controller + namespace).
3. Implement I/O queue creation.
4. Implement READ and WRITE I/O submission and completion handling.
5. Implement FLUSH.
6. Boot-validate in QEMU (`-device nvme,serial=...`).

### Phase 3 — Raft integration (complete for process-restart prototype)

1. Implement object-store-backed `DiskPersistentStateStore`.
2. Implement object-store-backed `DiskLogStore`.
3. Select memory, optional-disk, or required-disk storage through the launch
   manifest.
4. Boot-validate durable term recovery across teardown and restart of one
   Raft process.

This test is deliberately separate from the local two-node election test. The
generic Raft service can be exercised on a multicore local machine without
NVMe. A name service replicated among several physical or virtual machines is
a later consumer of that generic service and still requires a cross-machine
transport and multi-node validation.

### Phase 4 — Filesystem service (future)

1. Implement a simple filesystem (e.g., FAT32 or a custom minimal
   filesystem) as a userspace service that consumes the block device
   protocol and exports file operations through its own endpoint.

---

## 6. Why This Architecture

**Userspace driver, not kernel driver.** A bug in the NVMe driver corrupts
the driver's address space, not the kernel. The driver restarts; the system
stays up.

**Capability-based authority.** The NVMe driver receives exactly the BAR0
MMIO window and MSI-X interrupt it needs. It cannot access other PCI
devices. It cannot receive arbitrary interrupts. The block device protocol
definition gives it authority only over the namespace it serves.

**Memory objects, not copies.** READ writes directly into the caller's
pages via DMA (with BorrowWrite). WRITE reads directly from the caller's
pages via DMA (with BorrowRead). No intermediate kernel buffer, no copy.

**The unified shard wait.** The NVMe driver's main loop blocks on one
`cq_wait` that releases for both endpoint IPC (client requests) and
device interrupts (I/O completions). This is the same pattern as the
UART and virtio-net drivers — proven and tested.

**Disk-backed Raft through the same block protocol.** The Raft log store
speaks the block device endpoint protocol. It does not know the storage
is NVMe — it could be AHCI, USB MSC, a ramdisk for testing, or a network
block device. The block protocol is the abstraction boundary.
