# Analysis: DMA isolation and hostile-device security

Status: **OPEN** — CharlotteOS has a strong protected-DMA foundation, but the
items identified as production blockers in Section 8 remain to be implemented
and verified on physical hardware.

Date: 2026-08-30

---

## 1. Executive summary

CharlotteOS places each delegated PCI requester stream in a private IOMMU
domain. Drivers receive an I/O virtual address (IOVA), not a physical address,
and the device can reach only memory objects explicitly mapped into that
domain. Memory remains pinned until translation revocation is acknowledged;
when the hardware does not acknowledge invalidation or domain shutdown, the
kernel retains the mapping and frames rather than risking reuse while they may
still be DMA-reachable.

This closes the simplest and most damaging DMA attack: a device issuing reads
or writes to arbitrary core memory. It does **not** make the device or its DMA
protocol trustworthy. Every mapped buffer and descriptor ring remains a
hostile shared-memory interface, and all simultaneously mapped buffers in one
domain remain mutually visible to that device.

The most important remaining work is:

1. validate device completions as hostile input, beginning with NVMe command
   IDs;
2. make interrupt delivery subject to requester-aware remapping;
3. model the platform's true IOMMU isolation groups, including PCIe ACS,
   requester aliases, multifunction devices, and bridges;
4. explicitly disable ATS, PRI, and PASID until their complete security
   lifecycle is implemented;
5. disable bus mastering until the requester domain is ready; and
6. reduce mapping authority with range mappings or isolated bounce buffers.

The current implementation is a good defense against accidental or
straightforward DMA. It should not yet be represented as complete containment
of a deliberately malicious PCIe endpoint on arbitrary production hardware.

---

## 2. Scope and threat model

This analysis considers:

- a physically malicious PCIe or PCIe-tunneled endpoint;
- compromised device firmware;
- a device that correctly identifies sufficiently to select a CharlotteOS
  driver, then violates that device protocol;
- a compromised userspace driver exercising only its delegated MMIO,
  interrupt, DMA-domain, IPC, and memory capabilities; and
- failures or stale state during boot, reset, driver exit, and IOTLB
  invalidation.

The security objectives are:

- a device cannot read or modify kernel memory or another driver's unrelated
  memory;
- a device cannot retain access after a DMA operation or driver domain ends;
- one application's I/O cannot be confused with or completed as another
  application's I/O;
- an endpoint cannot forge interrupts outside its authority; and
- malformed device behavior is bounded and leads to faulting or quarantine,
  rather than unbounded CPU use or corruption of driver state.

This report does not attempt to protect against compromised IOMMU silicon,
physical probing of DRAM, CPU speculative-execution flaws, or denial of service
caused by physically removing a device. It also distinguishes isolation from
data authenticity: a storage controller allowed to return a block can still
return false contents, and a network adapter can still drop, replay, or alter a
frame. End-to-end authentication is required when those properties matter.

---

## 3. Current architecture and controls

### 3.1 Per-requester translation domains

`device::grant_dma_domain` resolves a PCI requester ID to the platform stream
ID, creates a domain, and delegates only a `DmaDomain` capability to the driver
(`crates/catten/src/device/mod.rs:476-490`). A stream cannot be installed in two
Charlotte domains simultaneously (`smmu.rs:514-527`, `vt_d.rs:479-530`,
`amd_vi.rs:388-419`). Translation tables and physical addresses remain in the
kernel.

The SMMUv3, VT-d, and AMD-Vi backends map potentially discontiguous physical
frames into a contiguous, domain-local IOVA range. Their initially empty page
tables are default-deny.

### 3.2 Capability and direction checks

`memory::object::pin_for_dma` verifies that the calling address space possesses
the memory capability and that the capability has the rights implied by the
device direction (`object.rs:1346-1390`). It rejects an exclusive transfer if
there is a CPU mapping, IPC lend, copy pin, or another DMA pin.

The safe runtime boundary expresses the exclusive case as ownership transfer:
`OwnedMemory::begin_dma` consumes unmapped memory and returns `DmaTransfer`.
Safe Rust references therefore cannot survive into exclusive device ownership
(`crates/catten-rt/src/owned.rs:177-196`). `SharedDmaMemory` retains the memory
object, CPU mapping, DMA mapping, and their required drop order for coherent
device rings (`owned.rs:198-229`, `401-484`).

### 3.3 Pinning, revocation, and quarantine

Every translation retains a `DmaPin` containing the memory object's physical
frames. Driver exit marks an owned object for deferred destruction while pins
remain. The final acknowledged unmap releases the pin and, when appropriate,
the frames (`object.rs:1251-1266`, `1393-1408`).

All three hardware backends invalidate translation state before unpinning. The
SMMUv3 backend keeps a mapping pinned if invalidation fails and keeps every
domain mapping pinned until an aborting stream-table entry is acknowledged
(`smmu.rs:531-616`). VT-d and AMD-Vi retain failed mappings in
`quarantined_pins` until the requester context is successfully disabled
(`vt_d.rs:534-630`, `amd_vi.rs:423-515`). This is the right fail-safe ordering:
leaking unavailable memory is preferable to reusing memory a device may still
address.

### 3.4 Boot ordering

The kernel initializes SMMUv3 or the x86 IOMMU before constructing PCI topology
and launching drivers (`crates/catten/src/main.rs:227-250`, `285-299`). Thus the
Charlotte-controlled path enables DMA against a default-deny remapper rather
than an identity domain.

This does not by itself prove that firmware disabled every endpoint's bus
mastering bit before kernel entry, especially across a warm reboot. Section 7
records the remaining boot window.

---

## 4. What IOMMU isolation does not provide

An IOMMU is comparable to an MMU: it limits the addresses an actor can reach,
but the permitted interface remains security-sensitive. The Thunderclap work
demonstrated attacks through deliberately malformed descriptor-ring behavior,
page-granularity exposure, stale translation windows, PCIe topology, and ATS
even on systems that used an IOMMU.

For CharlotteOS, the relevant distinction is:

```text
application memory capability
        │
        ▼
driver maps the complete memory object
        │
        ▼
kernel pins frames and installs IOVA → frame translations
        │
        ▼
device may access every permitted byte until acknowledged unmap
```

The IOVA is not a secret and predictability is not an access-control failure.
The page tables enforce access. However, once two objects are mapped in the
same domain, a malicious device can deliberately access either mapping; it is
not constrained to the descriptor or operation that was intended to use it.

---

## 5. Findings in device-visible memory

### DMA-01 — Device-written state is hostile protocol input

**Severity:** High for integrity and availability; potentially critical if a
driver turns a device value into an unchecked memory access.

The device controls completion entries, used-ring indices, descriptor IDs,
reported lengths, status bytes, and timing. Acquire/release fences make a
well-behaved hardware protocol coherent; they do not stop an adversarial device
from changing a value after the driver checks it.

Drivers must therefore:

- copy volatile device values into local scalar snapshots;
- validate indices before indexing any array;
- validate that a returned descriptor or command is currently in flight;
- reject duplicates, impossible queue advances, and unknown generations;
- bound work derived from a device-owned producer index; and
- never follow a device-supplied host pointer.

VirtIO net already bounds returned descriptor IDs and lengths before copying
received data (`crates/catten-services/src/bin/net.rs:243-304`). Its producer
delta and in-flight ownership rules should nevertheless be reviewed under a
malicious, rather than merely faulty, device model. The same audit is required
for E1000E, VirtIO block, AHCI, RNG, and NVMe.

### DMA-02 — NVMe completion IDs can select the wrong pending operation

**Severity:** High.

NVMe obtains `cid` directly from the device-written CQE
(`crates/catten-services/src/bin/nvme.rs:688-710`). `take_pending` converts that
untrusted value to an array index with `% MAX_PENDING`
(`nvme.rs:319-325`). A forged or duplicate CID can therefore consume another
operation's reply token, PRP list, and data mapping. The driver may reply to the
wrong client or revoke the wrong mapping.

The pending table should require an exact in-range CID and a matching active
generation. Unknown, duplicate, out-of-range, or not-in-flight completions must
not touch resource owners. They should increment a protocol-fault counter and,
after a small threshold, reset or quarantine the controller. Queue completion
processing must not decrement `outstanding` until validation succeeds.

### DMA-03 — Authority is per device domain, not per operation or client

**Severity:** High when mutually distrustful clients use the same physical
device.

Every active mapping for a driver shares one requester domain. A malicious
controller can inspect or modify the buffers of any concurrent client
operation. Guard pages detect accidental overruns but do not stop a malicious
device from scanning other valid IOVAs in its own domain.

Possible policies, in increasing isolation order, are:

1. accept that the physical device is trusted with all clients' active I/O;
2. serialize operations and expose only one client bounce buffer at a time;
3. use per-client or per-operation address spaces where the hardware supports
   scalable contexts/PASIDs and their secure lifecycle; or
4. cryptographically protect application payloads end to end so a device can
   observe traffic but cannot silently substitute authenticated content.

The selected policy must be explicit in the block, network, and future
accelerator service contracts.

### DMA-04 — Mapping granularity exceeds the requested transfer

**Severity:** Medium to high, depending on buffer contents.

The DMA syscall takes a memory capability and direction but no byte range.
`pin_for_dma` clones every frame in the memory object, and each backend maps
all of them. A request for part of a larger object therefore exposes the whole
object for the duration of the mapping.

Memory objects own whole pages, so unrelated kernel allocations do not share a
mapped page. This already avoids the classic case in which device data shares a
page with kernel metadata. It does not prevent exposure of unused bytes or
unrelated application fields within the same memory object.

A future typed API should support `offset + length`. Fully covered pages may be
mapped directly; partial first and last pages require dedicated, zeroed bounce
pages if bytes outside the range must remain confidential or immutable.

### DMA-05 — `DeviceWrite` is not receive-only confidentiality

**Severity:** Medium.

The memory-capability check distinguishes device reads from device writes, but
the current hardware leaf entries use read-only for a device-read mapping and
read/write for a device-write mapping (`vt_d.rs:149-178`,
`amd_vi.rs:129-165`). A buffer delegated so a device can fill it should
therefore be assumed readable by that device as well.

Receive buffers must be dedicated and zeroed and must not contain secrets or
control data. Documentation should describe `DeviceWrite` as granting at least
device write authority, not as enforcing write-only confidentiality on every
supported IOMMU.

---

## 6. Platform-boundary findings

### DMA-06 — Requester ID is not always the isolation granule

**Severity:** Critical on unvalidated physical topologies; low on a known
single-function QEMU topology.

CharlotteOS currently creates one domain per requester ID. Real PCIe topology
can reduce isolation through:

- a bridge without PCIe Access Control Services (ACS), permitting peer-to-peer
  redirection before transactions reach the IOMMU;
- requester-ID aliases or a PCIe-to-PCI bridge that hides downstream devices;
- multifunction hardware with an internal path between functions;
- firmware-reserved mappings; and
- more than one IOMMU or a requester path crossing remapping units.

The VT-d DMAR parser deliberately supports only `INCLUDE_PCI_ALL` or a direct,
one-hop endpoint scope (`crates/catten/src/environment/acpi/sdt/dmar.rs:1-8`,
`92-158`). That is a useful fail-closed limit, but it does not construct or
validate ACS isolation groups. AMD-Vi currently discovers the first IVHD and
then accepts the low 16 bits of a requester ID without validating IVRS device
entries (`ivrs.rs:1-6`, `amd_vi.rs:383-386`).

The kernel needs a first-class `DmaIsolationGroup`. A domain may be delegated
only for the whole group, and all group members must be disabled, owned by the
same driver domain, or proven isolated by validated ACS/requester routing.
Unsupported bridges, aliases, firmware mappings, and multi-IOMMU paths should
fail closed.

### DMA-07 — Interrupt delivery is not fully requester-remapped

**Severity:** High.

DMA remapping does not automatically constrain MSI/MSI-X messages. On x86,
CharlotteOS sends MSI directly to the LAPIC and identity-maps the selected
LAPIC MSI page in the VT-d domain (`crates/catten/src/cpu/isa/x86_64/interrupts/device_irq.rs:212-257`,
`crates/catten/src/device/vt_d.rs:120-145`). It does not yet configure the
VT-d interrupt-remapping table. AMD-Vi similarly enables translation without
configuring requester-aware interrupt remapping.

On AArch64, the GIC ITS binds a requester DeviceID and event ID to an allocated
LPI (`crates/catten/src/cpu/isa/aarch64/interrupts/gic/its.rs:270-301`) and is
the stronger path. The GICv2m fallback maps one doorbell page but takes the SPI
number from device-provided MSI data (`gic/mod.rs:490-541`), allowing a
malicious endpoint to attempt interrupt injection or an interrupt storm.

Production use with untrusted endpoints requires VT-d/AMD interrupt remapping
or an equivalent requester-aware interrupt controller. GICv2m and direct LAPIC
MSI should be classified as trusted-device compatibility paths.

### DMA-08 — ATS, PRI, and PASID are not explicitly disabled or audited

**Severity:** High if firmware or future code enables them.

PCIe Address Translation Services (ATS) lets a device cache translations and
issue requests marked as already translated. PRI and PASID add more device-side
translation and address-space state. These can be safe only when device TLB
invalidation, requester validation, reset, and teardown are implemented as one
protocol.

CharlotteOS does not currently implement these features, but its PCIe
capability code also does not enumerate and explicitly disable or reject them.
Until complete support exists, enumeration should clear their enable bits,
verify the resulting state, reject a device that cannot be put into that
state, and configure upstream ACS to reject inappropriate translated requests.

### DMA-09 — Bus mastering has a boot and launch-order window

**Severity:** Medium to high, depending on firmware and attachment model.

The kernel initializes the IOMMU before its topology-driven launch path enables
drivers. This protects normal Charlotte-controlled startup. Two windows remain:

1. firmware or a previous warm-boot kernel may leave an endpoint bus-mastering
   before CharlotteOS enables default-deny translation; and
2. `program_vector0` enables PCI memory decoding and bus mastering while
   programming MSI-X (`msix.rs:104-146`), whereas the supervisor creates the
   driver's DMA domain later (`service/supervisor.rs:805-821`).

The second window should fault safely because translation is already enabled
and the requester has no domain, but it permits fault flooding and makes the
lifecycle harder to reason about. The desired order is:

```text
disable bus mastering
→ identify complete isolation group
→ install default-deny requester context
→ create interrupt-remapping entry
→ launch driver and create required DMA mappings
→ program MSI/MSI-X
→ enable bus mastering last
```

Reset, driver crash, and hot-unplug should reverse the order: disable or reset
the requester, revoke interrupt authority, install an aborting DMA context and
wait for acknowledgement, then release pins and memory.

### DMA-10 — Faults and interrupts remain denial-of-service channels

**Severity:** Medium.

SMMUv3 records fault events and event-queue overflow
(`crates/catten/src/device/smmu.rs:618-669`). The backends favor memory safety
when invalidation fails, but there is no common policy that automatically
disables a requester after repeated translation faults, malformed completions,
or an interrupt storm. A malicious device can consume CPU, fill diagnostic
queues, keep memory quarantined, or prevent services from making progress.

Add per-requester counters, rate limits, bounded recovery attempts, and a
terminal quarantine state that clears bus mastering and refuses new maps.
Publish these events through the observability service without exposing
physical addresses or sensitive buffer information.

---

## 7. Security properties and limits

| Property | Current state |
|---|---|
| Device cannot address arbitrary RAM after protected-DMA initialization | Strong for supported, correctly described requester paths |
| Physical addresses remain kernel-private | Strong on the DMA path; owner-only compatibility queries remain |
| Frames cannot be reused before acknowledged IOTLB revocation | Strong; failures quarantine pins |
| Exclusive DMA cannot coexist with CPU/IPC authority | Enforced by memory-object state and typed ownership |
| A device can access only the intended operation's bytes | Not enforced; domain and memory-object granularity are broader |
| Completion data is safe to trust | Not a provided property; driver validation is required and incomplete |
| One PCI function equals one isolated requester | Not generally true; topology/ACS grouping is missing |
| MSI source and vector are requester-authorized | Stronger with GIC ITS; incomplete for direct LAPIC MSI, AMD-Vi, and GICv2m |
| ATS/device-TLB state cannot bypass teardown | Not established; features must be explicitly disabled |
| Malicious device cannot deny service | Not achievable in full; fault and interrupt effects can be bounded better |
| Device output is authentic and fresh | Outside IOMMU scope; requires protocol or application cryptography |

---

## 8. Recommended implementation sequence

### Phase 0 — Hostile completion handling

1. Replace NVMe's modulo pending lookup with an exact bounded CID and
   generation table.
2. Reject duplicate and not-in-flight completions without releasing resources.
3. Bound every device-controlled queue advance to at most the number of
   outstanding entries.
4. Track descriptor ownership explicitly and validate it before accepting a
   completion.
5. Add a shared device-protocol-fault abstraction leading to reset or
   quarantine.

### Phase 1 — Platform isolation

1. Disable bus mastering for every discovered endpoint before binding it.
2. Add `DmaIsolationGroup` construction from IORT, DMAR, IVRS, PCI topology,
   requester aliases, and ACS state.
3. Reject unsupported multi-IOMMU, bridge, alias, and multifunction cases.
4. Implement VT-d and AMD-Vi interrupt remapping; require GIC ITS where a
   requester-aware AArch64 MSI path is part of the threat model.
5. Enumerate and explicitly disable ATS, PRI, and PASID.

### Phase 2 — Least-authority DMA buffers

1. Add a typed range-mapping API at the runtime/kernel boundary.
2. Use dedicated zeroed bounce pages for partial-page transfers and for
   mutually distrustful clients where appropriate.
3. Separate device data, device control fields, and CPU-only metadata into
   distinct memory objects.
4. Consider unmapped IOVA guard pages for diagnostics and accidental-overrun
   detection.
5. Document whether each service trusts its physical device with all clients'
   concurrent plaintext.

### Phase 3 — Recovery and evidence

1. Add reset/quiesce protocols before domain destruction.
2. Expose per-requester mapping count, faults, invalid completions, resets,
   quarantined pins, and interrupt rate.
3. Extend the DMA TLA+ model with bus-master enable, requester grouping,
   interrupt remapping, device-TLB state, malformed completion, and quarantine.
4. Establish a physical-hardware qualification matrix for ACS, DMAR/IVRS/IORT,
   interrupt remapping, reset behavior, and warm boot.

---

## 9. Validation plan

### 9.1 Adversarial virtual-device tests

Build QEMU or vhost test devices capable of:

- DMA one byte before and after every mapped range;
- scanning other active IOVAs in the same domain;
- returning out-of-range, duplicate, stale, and mismatched NVMe CIDs;
- advancing VirtIO used indices by zero, queue size, queue size plus one, and
  `u16::MAX`;
- changing a descriptor ID or length between driver reads;
- reporting completion and continuing DMA;
- issuing arbitrary MSI address/data combinations;
- flooding unmapped IOVAs and overflowing the IOMMU event queue; and
- failing to quiesce during driver teardown.

The expected result is a bounded protocol error, translation fault, reset, or
quarantine. No test may release or reply through an unrelated pending
operation, expose a physical address, reuse a pinned frame, or livelock the
driver.

### 9.2 Kernel lifecycle tests

Retain and extend the existing DMA ownership tests to cover:

- mapping submission failure and partial-page-table rollback;
- synchronous unmap success;
- invalidation timeout and retained pins;
- driver exit with active DMA;
- domain destruction timeout;
- a completion racing unmap;
- warm-boot-style bus mastering before domain creation; and
- repeated device faults reaching automatic quarantine.

### 9.3 Physical platform tests

For every supported system, record:

- all DMA remapping units and requester coverage;
- the isolation group for every delegated endpoint;
- ACS capability and control settings on every upstream port;
- requester aliases and multifunction relationships;
- interrupt-remapping availability and enabled state;
- ATS/PRI/PASID disabled state;
- behavior across function-level reset, secondary-bus reset, driver crash,
  warm reboot, and hot-unplug; and
- proof that a test requester faults when addressing a kernel sentinel page.

---

## 10. References

- A. Theodore Markettos et al.,
  [*Thunderclap: Exploring Vulnerabilities in Operating System IOMMU Protection via DMA from Untrustworthy Peripherals*](https://thunderclap.io/wp-content/uploads/2024/01/thunderclap-paper-ndss2019.pdf),
  NDSS 2019. The most directly relevant treatment of hostile shared-memory
  device protocols, page-granularity exposure, temporal mappings, and ATS.
- Linux kernel,
  [*VFIO — Groups, Devices, and IOMMUs*](https://www.kernel.org/doc/html/v6.12/driver-api/vfio.html#groups-devices-and-iommus).
  Explains why PCI topology and ACS make the isolation group, rather than an
  individual endpoint, the minimum safe ownership unit.
- Intel,
  [*Intel Virtualization Technology for Directed I/O Architecture Specification*](https://cdrdv2-public.intel.com/831418/vt-directed-io-spec.pdf).
  Defines DMA and interrupt remapping, invalidation, device TLBs, PASIDs, and
  requester validation.
- AMD,
  [*AMD I/O Virtualization Technology (IOMMU) Specification*](https://www.amd.com/content/dam/amd/en/documents/processor-tech-docs/specifications/48882_IOMMU.pdf).
- Linux kernel,
  [generic IOMMU binding documentation](https://github.com/torvalds/linux/blob/master/Documentation/devicetree/bindings/iommu/iommu.txt).

Charlotte-specific lifecycle correspondence is maintained in
[`docs/tla/CONFORMANCE.md`](../../tla/CONFORMANCE.md#dma-and-smmuv3-lifecycle),
and the current architecture summary is in
[`docs/architecture/persistent-storage.md`](../../architecture/persistent-storage.md#36-dma).
