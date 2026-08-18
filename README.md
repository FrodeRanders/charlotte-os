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

## Contributions in this fork

The main additions and extensions currently maintained here are:

- **AArch64 QEMU bring-up and SMP execution:** boot on QEMU's `virt` machine,
  secondary-processor startup, GICv3 and generic-timer support, PCIe ECAM
  enumeration, preemptive scheduling, and serial-first development tooling.
- **Capability-based userspace services:** EL0 service loading, typed launch
  manifests, endpoint IPC, transferable memory objects, completion queues,
  device capabilities, service supervision, and stale-connection handling.
- **Scheduler and completion work:** interrupt-safe per-processor state,
  blocking completion waits, timer-affinity preservation, constrained runtime
  rebalancing, lifecycle cleanup, and idle operation without steady-state
  polling.
- **One shared node-local name service:** services register in a common
  registry; waitable lookup blocks until registration instead of relying on
  arbitrary boot-time spin ranges. A distributed name service (`dns`)
  replicates a `name → node` catalog across two QEMU guests over the
  reliable-message layer.
- **Generic Raft service:** a transport-independent Raft core and EL0 service,
  with a local two-node election test that can run on a multicore development
  machine. The distributed name service uses the same Graft core to replicate
  the catalog across several virtual machines.
- **TCP/IP as a userspace service:** the smoltcp stack runs through a frame
  demultiplexer (IPv4/ARP routed to the `tcpip` service) and exposes a
  socket-API protocol for clients; an `httpd` keyhole serves a full-node JSON
  report over it, reachable from the host via SLIRP `hostfwd`
  (`scripts/run-aarch64.sh --http-test`, then `curl localhost:8080`).
- **Reliable-message fragmentation:** messages up to 64 KiB are split across
  Ethernet frames and reassembled at the receiver, carrying the distributed
  name service / Raft across two guests.
- **Userspace persistent-storage prototype:** an NVMe block driver using DMA
  and MSI-X, a block protocol, an object store, and namespaced Raft
  term/vote/log/snapshot storage. Process-restart recovery is boot-tested;
  objects and files can exceed a single NVMe request. Object-store v3 adds
  mirrored generation-selected directory records, device-scaled directory
  capacity, up to 16 extents per object, integrity hashes, allocation
  reconstruction, and copy-on-write replacement (individual objects use
  32-bit lengths). One torn directory-record write is recoverable; full
  device-level power-loss guarantees still depend on truthful flush/FUA
  behaviour.
- **Experimental live service upgrade:** an EL0 service manager can spawn a
  replacement generation, transfer state, synchronize registration, and
  invalidate stale connections. This remains prototype work rather than a
  production upgrade framework.
- **Store-backed, blessed service artifacts:** the kernel embeds only the
  bootstrap storage path; the remaining AArch64 service ELFs are loaded by
  logical name from an initial NVMe object-store image. CLS2 Ed25519 notes bind
  bytes to name, class, release/rollback policy, parallel-instance permission,
  and optional provenance evidence. The runner refreshes stale seed images,
  while deployment pins the complete artifact SHA-256.
- **Sitas shard runtime at EL0:** the `sitas` no_std shard-per-core runtime
  (external crates) runs as a real EL0 image (`catten-user`) boot-tested by
  the kernel. A mailbox index demo shows the division of responsibility:
  scanner shards route entries to logical assemblers through typed owned
  messages in userspace, and the coordinator merges and verifies the result,
  while the kernel only supplies the address space, LP-pinned thread spawn,
  and the completion-queue wait/wake.
- **Architecture and implementation documentation:** the
  [documentation index](docs/README.md) separates living architecture,
  implementation reference, contributor guides, platform status, historical
  reports, and research context. The LaTeX manual in
  [`docs/manual-v2`](docs/manual-v2) provides the integrated narrative.

The automated AArch64 boot path exercises these mechanisms under QEMU, but this
is an experimental research and development system. A successful self-test is
evidence for the tested configuration, not a general reliability, security, or
hardware-compatibility claim.

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
interchangeable server-class ARM nodes assemble themselves into clusters on
boot, software is deployed to a named cluster rather than to named servers,
and the cluster decides placement -- initially from declared component
affinity, eventually from observed inter-dependency, with cross-node
migration of running components. Placement policy distinguishes replica count,
per-node instance capacity, affinity/co-location, and anti-affinity. Multiple
instances are legal only when the signed artifact policy blesses parallel
execution. Nodes are "dumb" compute over a shared
object store, validating signed software against a cluster-wide key held in
replicated state.

A first end-to-end slice of this is implemented and boot-tested on the
two-guest QEMU cluster (`scripts/run-aarch64.sh --deploy-test`): the
deployment manifest is replicated Raft state and pins an immutable digest. An
agent with narrowly delegated deployment authority picks up and verifies the
artifact; the kernel starts that exact ELF in a separate address space, and
the service serves across the network and reassign it between nodes without 
losing its name; `clusterctl` (plus a serial admin console) provides the outside 
upload/deploy/status interface. Artifacts are real ELF binaries blessed and 
signed in place with Ed25519: the signature lives in a standard 
`.note.charlotte-sig` ELF note (added by `tools/cluster-sign elf-sign`), 
the public key is injected at build time and committed to the replicated state 
by the key ceremony, and the EL0 loader (which refuses any unsigned or invalidly 
signed image -- the build pipeline signs every staged service ELF with the 
version-controlled development key in `tools/cluster-sign/dev-key.hex`) and 
the deploy path validate both bytes and logical identity. Known 
third-party-containing services can therefore be admitted once with an 
SBOM/provenance digest and traded internally without runtime Internet 
dependency fetching. The object store is still per-node, replica-set placement 
is not implemented, and the mutation endpoint still needs a separately 
delegated administrator capability. These boundaries are called out in 
Chapter 17 of [the manual](docs/manual-v2) ("Server-Class Cluster Vision"), 
which describes the vision against what already exists (consensus, the
distributed name service, the object store, and live upgrade).

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
`--disco-test`, `--dns-test`, `--tcpip-test`, `--http-test`).

> **HVF caveat:** Apple's Hypervisor.framework does not preserve the hardware
> ASID bits of `TTBR0_EL1`, so ASID-based TLB isolation and `mrs ttbr0_el1`
> caller attribution do not work under `--hvf`. HVF builds rely on the
> `hvf_compat` fallback (whole-TLB flush on every context switch, per-LP
> tracked caller ASID). See the "HVF and hardware ASIDs" section of
> [`docs/platforms/aarch64.md`](docs/platforms/aarch64.md) before debugging
> under HVF. HVF also provides no SMMU to the guest, so its compatibility boot
> runs the 15-test non-storage suite; use the default TCG path for protected
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
  - [Planned] USB Mass Storage Device Class (MSC)
  - [Planned] AHCI (SATA)
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
