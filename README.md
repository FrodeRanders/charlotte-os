# The Charlotte Operating System (CharlotteOS)

This repository is a development fork of the
[CharlotteOS project](https://github.com/charlotte-os/charlotte-os). CharlotteOS,
its original architecture, and the substantial foundation on which this work
builds were created by the upstream project and its contributors.

The purpose of this fork is to explore and implement additional operating-system
mechanisms while retaining CharlotteOS's Rust, capability-oriented, and
service-based direction. It is not a replacement for upstream, and features
described below should not be assumed to be present or supported in the
upstream repository.

## Research direction: the cluster as a capability machine

This fork explores a deliberately ambitious question: what would an operating
system look like if isolation, explicit ownership, and least authority did not
stop at the process boundary, but extended through services, deployment, and a
whole cluster? The aim is a distributed computer in which nodes can discover
one another, agree on desired state, fetch signed software, and place it without
turning every application into its own miniature infrastructure platform.

The central bet is that cluster software can become easier to reason about when
authority is a first-class value. Applications should receive narrow
capabilities, not ambient network access or copied credentials. Infrastructure
connectors should own Kafka, S3, TLS, and authentication policy. Signed release
descriptors should state both what code may run and which named services it may
use. Rust ownership should then carry those constraints into ordinary program
structure, making resource leaks and accidental authority propagation harder to
express.

This opens several connected research directions: capability-safe distributed
deployment, content-addressed and provenance-aware software delivery,
replicated naming and lifecycle state, bounded asynchronous execution,
failure-domain-aware placement, and generation of Charlotte applications from
higher-level process models such as
[Durga](https://github.com/FrodeRanders/durga/tree/feature/charlotte-target). The
aspiration is an understandable substrate for long-lived service systems whose
security and failure behaviour can be inspected end to end.

Some of that path already exists as working vertical slices while other parts
remain research. The lists below distinguish implemented mechanisms from
the longer-term direction.

## Contributions in this fork

The main additions and extensions currently maintained here are:

- **AArch64 bring-up and SMP execution:** boot on QEMU's `virt` machine,
  secondary-processor startup, GICv3 and generic-timer support, PCIe ECAM
  enumeration, preemptive scheduling, and serial-first development tooling.
  The server-oriented SBSA Reference work additionally exercises UEFI/ACPI
  discovery through MADT and SPCR, high physical addresses, GICv3 ITS/LPI
  delivery, and firmware handoff from EL2 into the EL1-oriented kernel.
- **x86-64 service and cluster parity:** multi-LP ring-3 execution uses the
  shared Rust service, syscall, IPC, and launch ABIs. QEMU suites exercise
  storage, networking, discovery, distributed naming, Raft, live upgrade, and
  signed deployment behind Intel VT-d or AMD-Vi. An installable two-disk VMware
  Fusion appliance additionally validates persistent first-boot installation,
  virtual NVMe, VT-d, and the userspace E1000E path.
- **Capability-based userspace services:** EL0 (userspace) service loading,
  typed launch manifests, endpoint IPC, transferable memory objects,
  completion queues, device capabilities, service supervision, and
  stale-connection handling. The non-`Copy` owners in `catten_rt::owned`
  mirror linear kernel ownership: moved capabilities transfer exactly once,
  memory borrows outlive pending calls, and `Drop` or explicit consuming
  teardown closes compound operations across ordinary error paths.
- **Protected userspace device I/O:** reference UART, NVMe, AHCI, virtio-blk,
  virtio-net, E1000E, and VirtIO RNG drivers receive only delegated MMIO,
  interrupt, and DMA authority. Arm SMMUv3, Intel VT-d, and AMD-Vi isolate
  requester DMA; MSI/MSI-X and IRQ-to-completion-queue delivery keep interrupt
  handling outside application code, while reset and reconciliation support
  driver restart.
- **Scheduler and completion work:** interrupt-safe per-processor state,
  blocking completion waits, timer-affinity preservation, constrained runtime
  rebalancing (between cores), lifecycle cleanup, and idle operation without
  steady-state polling.
- **One shared node-local name service:** services register in a common
  registry; waitable lookup blocks until registration instead of relying on
  arbitrary boot-time spin ranges. A distributed name service (`dns`)
  replicates a `name → node` catalog across QEMU guests and provides
  linearizable leader-read lookups, bounded generation-fenced remote
  invocation, duplicate suppression, two-phase publication, and
  owner-and-generation-fenced retraction over the reliable-message layer.
- **Generic Raft service:** a transport-independent Raft core and one durable,
  network-enabled EL0 member per CharlotteOS node. Discovery supplies peer
  routes and the service performs restart-safe dynamic admission between
  machines. The distributed name service currently uses a separate instance
  of the same [Graft](https://github.com/FrodeRanders/raft) core to
  replicate its catalog.
- **TCP/IP and UTC as userspace services:** the
  [smoltcp](https://github.com/smoltcp-rs/smoltcp) stack runs through a frame
  demultiplexer (IPv4/ARP routed to the `tcpip` service) and exposes a
  TCP/connected-UDP socket API. The `time` service uses it to sample NTP,
  calibrates the observe service's monotonic counter, persists holdover state
  in the object store, and publishes Unix, calendar, and ISO 8601 UTC over
  endpoint IPC. A simple `httpd` service provides a self-refreshing dashboard
  and a full-node JSON report with scheduler and per-service telemetry over
  TCP, reachable from the host via SLIRP `hostfwd`
  (`scripts/run-aarch64.sh --http-test`, then `curl localhost:8080`).
  Both QEMU runners attach a NIC by default: ordinary boots acquire a DHCP
  lease, start discovery and cluster formation, and synchronize UTC. The
  `*-test` options only add verifiers; use `--no-network` for an intentionally
  isolated boot.
- **Reliable-message fragmentation:** wire protocol v3 uses 32-bit message
  lengths and fragment offsets. The initial 1 MiB operational ceiling is a
  resource policy, not a field-width limit; messages are split across Ethernet
  frames and reassembled at the receiver, carrying the distributed name
  service / Raft across two guests.
- **Capability-scoped data-plane clients:** the S3 service confines a managed
  object-store endpoint, bucket/prefix, TLS trust anchor, credentials, and
  operations behind endpoint capabilities. Named Kafka connectors confine an
  authorized broker pool, TLS/mTLS and SASL/SCRAM material, topic/partition
  routes, consumer-group membership, and transactional identity. Attenuated
  access points support idempotent production, bounded read-committed
  consumption, failover and rebalancing, and transactional offset commits
  through linear Rust owners. A generic `kafka_step` service owns the Kafka
  transaction while invoking separately deployed business logic. Verified TLS
  obtains fallible entropy from Arm `RNDR`, x86-64 `RDRAND`, or an isolated
  VirtIO RNG service and fails closed rather than using a deterministic
  fallback.
- **Userspace persistent-storage prototype:** NVMe, AHCI, and virtio-blk
  drivers publish one block protocol consumed by the object store, native
  hierarchical filesystem, and namespaced Raft term/vote/log/snapshot storage.
  Process-restart recovery is boot-tested; objects and files can exceed a
  single device request. Object-store v3 adds mirrored generation-selected
  directory records, device-scaled directory capacity, up to 16 extents
  per object, integrity hashes, allocation reconstruction, and copy-on-write
  replacement (individual objects use 32-bit lengths). One torn
  directory-record write is recoverable; full device-level power-loss
  guarantees still depend on truthful flush/FUA behaviour. A host-side
  inspector checks directory generations, hashes, allocation overlap,
  filesystem reachability, and raw persistent images.
- **Experimental live service upgrade:** an EL0 service manager prototype
  can spawn a replacement generation, transfer state, synchronize
  registration, and invalidate stale connections.
- **Store-backed, blessed service artifacts:** AArch64 embeds only the
  bootstrap storage path and loads the remaining service ELFs by logical name
  from an initial NVMe object-store image. x86-64 can also install its complete
  immutable signed bundle onto a blank object store during first boot, while
  retaining valid persisted upgrades on later boots. CLS2 Ed25519 notes bind
  bytes to name, class, release/rollback policy, parallel-instance permission,
  and optional provenance evidence. Deployment pins the complete artifact
  SHA-256.
- **Signed, capability-scoped cluster deployment:** CI can place immutable ELFs
  in a separately managed S3-compatible store and notify any cluster member
  with a signed `CDEPLOY4` descriptor that binds placement, per-thread stack
  pages, a maximum active-thread count, a bounded cooperative-shutdown grace
  period, and capability grants. A signed
  `CRELEASE` binds an ordered
  multi-component change and admits all desired revisions in one Raft command.
  Assigned node agents fetch and verify each artifact, launch it in a fresh
  address space, and give it only `grantctl`; the controller translates the
  descriptor's named grants into attenuated service connections without
  exposing the name service or infrastructure credentials to the application.
- **Authorization and executable safety models:** the name service hosts a
  bounded default-deny policy engine with kernel-authenticated principals,
  separately protected administration and publication roles, attenuated and
  generation-fenced connection issuance, and an audit stream. A set of TLA+
  models cover IPC, memory transfer, completions, scheduling, lifecycle,
  authorization, DMA, Raft, remote calls, and reliable messaging; CI checks the
  safe configurations and expected-counterexample regressions.
- **[Sitas](https://github.com/FrodeRanders/sitas) shard runtime at EL0:**
  the `sitas` no_std shard-per-core runtime (external crates) runs as a real
  EL0 image (`catten-user`) boot-tested by the kernel. A mailbox index demo
  shows the division of responsibility: scanner shards route entries to
  logical assemblers through typed owned messages in userspace, and the
  coordinator merges and verifies the result, while the kernel only supplies
  the address space, logical processor (core) pinned thread spawn, and the
  completion-queue wait/wake.
- **Architecture and implementation documentation:** the
  [documentation index](docs/README.md) separates living architecture,
  implementation reference, contributor guides, platform status, historical
  reports, and research context. The
  [LaTeX manual](docs/manual-v2/charlotte.pdf) provides the integrated narrative.

Automated AArch64 and x86-64 QEMU paths exercise these mechanisms, with
additional opt-in Docker fixtures for S3 and Kafka and a VMware appliance path.
This is experimental *research and development*, so we are not making
any reliability, security, or hardware-compatibility claims.

Run target-independent suites with `scripts/run-host-tests.sh`. The split
between host tests and target/QEMU tests is documented in
[`docs/guides/testing.md`](docs/guides/testing.md).

For the upstream project, its history, and its community, please visit
<https://github.com/charlotte-os/charlotte-os>. Changes from upstream are kept
as ordinary Git history so that upstream updates can continue to be merged into
this fork.

## Architectural optimisations

This specific architecture is primarily optimised for controlled, understandable
behaviour in systems composed of isolated asynchronous services rather than peak
benchmark throughput:

- **Fault containment:** Drivers and services live in separate address spaces.
  A failed component should lose its capabilities, have outstanding operations
  cancelled, and be restartable without taking down the machine.

- **Predictability and boundedness:** Queues, messages, capability tables, and
  most communication paths are explicit and bounded. Overload should produce
  visible backpressure rather than hidden memory growth.

- **Low tail latency:** Shard-local ownership avoids shared locks and cache-line
  contention. Work normally stays on its assigned LP, reducing migration, interference,
  and latency variance.

- **Efficient asynchronous workloads:** Completion queues, waitable IPC, interrupts,
  and timers allow services to block when idle and resume from events. The 0%
  steady-state CPU behaviour is an important architectural outcome.

- **Capability-oriented security:** Authority is conveyed through typed object
  capabilities rather than ambient process privileges, global device access, or
  freely chosen syscall numbers. Components receive only the resources required
  for their role.

- **Lifecycle control:** Services are expected to start, stop, crash, restart,
  and eventually upgrade. Stale connections are invalidated, device authority is
  reclaimed, and clients can rediscover a replacement through the name service.

- **Composability:** Storage, networking, consensus, naming, and drivers are
  services connected through stable protocols. Implementations can be replaced
  without moving policy into the kernel.

- **Multicore locality:** The Sitas/shard model assigns mutable state to one
  owner and uses explicit messages for cross-shard communication. It is intended
  to scale by partitioning ownership rather than increasing shared-memory locking.

- **Operational observability:** Explicit capability transfers, completion
  records, service generations, named tests, and well-defined ownership make
  failures easier to diagnose than systems built from implicit global state.

- **A small kernel trust boundary:** The kernel supplies isolation, scheduling,
  memory objects, capabilities, IPC, interrupts, and DMA protection.
  Filesystems, drivers, consensus, and higher-level policy remain outside it.

In short, it optimises for dependable service execution: isolated authority,
explicit ownership, bounded asynchronous communication, predictable multicore
locality, and rapid recovery.

## Vision: server-class clusters

The long-term direction is to scale this model from one machine to a cluster:
interchangeable server-class nodes assemble themselves on boot, software is
deployed to a named cluster rather than to named servers, and replicated policy
decides where components belong. Placement should eventually account for
replica count, node capacity and labels, failure domains, affinity,
anti-affinity, observed communication, readiness, and disruption budgets. A
node should be replaceable compute, retaining only the local state needed to
participate safely while pulling immutable software from a managed object
store and validating it against cluster-wide trust state.

A first end-to-end version of this is implemented and boot-tested on two-guest
AArch64 and x86-64 QEMU clusters (`scripts/run-aarch64.sh --deploy-test` and
`scripts/run-x86_64.sh --deploy-test`): the
deployment catalog is replicated Raft state and pins immutable digests. The
normal network service set starts a bounded deployment ingress: CI uploads
ELFs to a separately provisioned central S3 store, signs deployment descriptors
with the offline cluster key, and sends descriptors rather than executable
bytes or storage credentials to `deployd`. A request may enter through any
cluster member and be relayed to the leader. `CRELEASE` adds atomic admission
of an ordered component set; assigned node agents independently reconcile the
committed desired state, pull pinned bytes, and report exact-generation
readiness. Admission is atomic, while fetch, launch, readiness, coordinated
rollback, and rollout policy remain separate concerns.

The next deployment boundary separates development approval from operational
configuration: developers sign immutable behavior and request logical
capabilities, while operators bind those names to production Kafka and S3
profiles without exposing credentials to the application. The first bounded,
operator-signed HPKE envelope, admission-bundle tooling, role-aware public
trust, leader-verified ingress, follower relay, trusted-time expiry, and compact
replicated replay fences are implemented. The assigned node now retrieves the
digest-pinned encrypted profile through its bootstrap S3 capability; a
deployment-agent-only kernel gate re-verifies the signed release, descriptor,
artifact and envelope, opens HPKE into zeroizing memory, validates the bounded
S3/Kafka profile, and transfers it read-only to the connector without exposing
plaintext to Raft or application IPC. Production key custody, rotation,
readiness-driven cutover, and audit remain research and engineering work. See
[Deployment secrets and the development/operations boundary](docs/architecture/deployment-secrets-and-operations.md)
for the trust model, precise status, and rollout plan.

The node agent has narrowly delegated deployment authority. After verification,
the kernel starts each exact ELF in a separate address space with `grantctl` as
its sole bootstrap service. Signed descriptors determine which named
capabilities the component may acquire, and service publication is generation
fenced. `clusterctl` also retains the local upload/deploy/status path used by
the test console. Artifacts are real ELF binaries blessed and
signed in place with Ed25519: the signature lives in a standard 
`.note.charlotte-sig` ELF note (added by `tools/cluster-sign elf-sign`), 
the public key is injected at build time and committed to the replicated state 
by the key ceremony, and the EL0 loader (which refuses any unsigned or invalidly 
signed image -- the build pipeline signs every staged service ELF with a
*publicly known* version-controlled development key in
`tools/cluster-sign/dev-key.hex`) and the deploy path validate both bytes
and logical identity. Known third-party-containing services can therefore be
admitted once with an SBOM/provenance digest and traded internally without
runtime Internet dependency fetching. Bootstrap and Raft durability are still
per-node; capacity-aware replica placement, failure-domain scheduling,
rescheduling, coordinated rollback, and a richer process-level release bundle
are not yet implemented. These boundaries are called out in
Chapter 17 of [the manual](docs/manual-v2/charlotte.pdf) ("Server-Class
Cluster Vision"), which describes the vision against what already exists
(consensus, the distributed name service, the object store, and live upgrade).

That should make it particularly interesting for:
- Control systems and appliances.
- Storage or network services with strict latency requirements.
- Long-running systems that must replace or recover components.
- Multi-tenant service machines with narrowly delegated authority.
- Systems where auditable failure behaviour matters more than POSIX compatibility.
- Distributed machines whose local services participate in replicated naming or state.

---

## Programming Languages

- CharlotteOS is written primarily in the latest Edition of Rust, with architecture-specific assembly where required or advantageous.
- x86-64 assembly uses Intel syntax as implemented by `rustc`/`llvm-mc`.

---

## Platform & Firmware Requirements

CharlotteOS aims to support platforms that offer **standardized, documented, and interoperable hardware and firmware interfaces**. The focus is on systems where the operating system can rely on well-defined firmware and discoverability mechanisms, without requiring vendor-specific hacks or opaque initialization sequences.

### Supported Architectures and Their Requirements

#### x86-64

- Invariant Timestamp Counter
- Local APIC with x2APIC mode
- Always Running APIC Timer (ARAT) available on all logical processors
- Full standards conforming UEFI and ACPI firmware environment
- Intel or AMD compatible IOMMU

The QEMU `q35` development target boots on multiple logical processors, runs
the userspace service stack at ring 3, and exercises NVMe, AHCI, virtio-blk,
virtio-net, discovery, distributed DNS, smoltcp TCP/IP, HTTP reporting, signed
deployment, migration, and dynamic membership behind VT-d or AMD-Vi. See
[`docs/platforms/x86_64.md`](docs/platforms/x86_64.md) for the tested matrix,
runner commands, and remaining limitations. A generated two-disk
[VMware appliance](docs/platforms/vmware-x86_64.md) additionally boots the
storage and service stack under Fusion and uses a protected userspace E1000E
driver for VMware's emulated Intel 82574L adapter.

#### AArch64 (ARM64)

- ARMv8-A or later application processor
- Generic Interrupt Controller version 3 (GICv3)
- ARM Generic Timer
- Full standards conforming UEFI and ACPI firmware environment (ARM SystemReady
  compliant), or a Flattened Device Tree (FDT) on embedded platforms
- ARM System Memory Management Unit (SMMU) for IOMMU functionality

AArch64 support is under active development. The kernel currently boots on the
QEMU `virt` machine (GICv3): it initializes memory, brings up all secondary
processors, runs the scheduler with preemptive context switching driven by the
ARM Generic Timer, and enumerates PCIe via ECAM. See
[`docs/platforms/aarch64.md`](docs/platforms/aarch64.md) for a detailed
status report, including current limitations (notably device-tree discovery).
EL0 execution, isolated userspace services, IPC, driver restart, virtio-net
frame exchange, reliable messages, the distributed name service, and a
two-node Raft election are boot-tested under QEMU TCG. See
[`docs/guides/aarch64-network-development.md`](docs/guides/aarch64-network-development.md)
for the macOS TCG and two-VM stream-LAN workflow (`--relmsg-test`,
`--disco-test`, `--dns-test`, `--tcpip-test`, `--http-test`). Those switches
add validation workloads; the underlying network, discovery, cluster, TCP/IP,
HTTP, and time services are part of an ordinary boot.

> **HVF caveat:** Apple's Hypervisor.framework does not preserve the hardware
> ASID bits of `TTBR0_EL1`, so ASID-based TLB isolation and `mrs ttbr0_el1`
> caller attribution do not work under `--hvf`. HVF builds rely on the
> `hvf_compat` fallback (whole-TLB flush on every context switch, per-LP
> tracked caller ASID). See the "HVF and hardware ASIDs" section of
> [`docs/platforms/aarch64.md`](docs/platforms/aarch64.md) before debugging
> under HVF. HVF also provides no SMMU to the guest, so its compatibility boot
> requires `--no-network` and runs the 15-test non-storage suite; use the
> default TCG path for protected
> NVMe DMA, the object store, and persistent Raft tests.

#### *Other architectures may be supported in the future depending on contributor support and demand for their development.*

---

## Firmware Model

System firmware is required to implement the UEFI specification and version 2.0 or later of the ACPI specification.

The latest versions of both specifications can be found at <https://uefi.org/specifications>.

---

## Supported Hardware

### Memory[^1]

Embedded:

- Recommended: ≥ 128 MiB
- Minimum: 24 MiB

PC and Server:

- Recommended: ≥ 2 GiB
- Minimum: 256 MiB

### Storage[^1]

- Recommended: ≥ 64 GiB
- Minimum: 4 GiB
- Supported device classes:
  - [Prototype in this fork] NVMe (PCIe, QEMU-tested)
  - [Prototype in this fork] AHCI (SATA, x86-64 QEMU-tested)
  - [Prototype in this fork] virtio-blk (PCI, x86-64 QEMU-tested)
  - [Planned] USB Mass Storage Device Class (MSC)
  - [Planned] SDHCI (PCIe SD card reader)

### Display

- Linear framebuffer exposed via UEFI GOP

### Input Devices

- Keyboards:
  - [Planned] i8042 PS/2
  - [Planned] USB HID
  - [Planned] I²C HID

- Pointing Devices:
  - [Planned] i8042 PS/2
  - [Planned] USB HID
  - [Planned] I²C HID

### Serial Console

- [Planned] NS16550 compatible UART over PCIe
- [Planned] USB CDC-ACM (virtual serial)

### Networking

- [Planned] USB CDC-NCM (Ethernet over USB)

---

## Contributing

The CharlotteOS upstream project welcomes contributions of all forms—code,
design proposals, documentation, and testing. Please use the upstream
repository and the community links below when contributing to CharlotteOS
itself. Issues or changes specifically concerning the experimental additions
listed above may instead be discussed in this fork.

Upstream's contribution guidance requires new hardware support to include
inline documentation comments with references to publicly available hardware
documentation. This may include community reverse-engineered documentation,
along with clean, maintainable code.

---

## Licensing

The Charlotte Operating System is licensed under the GNU Affero General Public License version 3.0 (or any later version). By contributing, you agree that your work may be distributed under the AGPL version 3.0 or later.

---

## Community

Find us on:

- **Discord:** <https://discord.gg/vE7bCCKx4X>  
- **Matrix:** <https://matrix.to/#/#charlotteos:matrix.org>
- **Reddit** <https://www.reddit.com/r/charlotteos>
- **E-Mail** <charlotte-os@outlook.com>

[^1]: These requirements are estimates that may change in the course of development.
