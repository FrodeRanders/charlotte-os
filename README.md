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
  arbitrary boot-time spin ranges. Replication of this registry across
  computers remains future work.
- **Generic Raft service:** a transport-independent Raft core and EL0 service,
  with a local two-node election test that can run on a multicore development
  machine. This is distinct from the planned use of Raft to replicate the name
  service across several physical or virtual machines.
- **Userspace persistent-storage prototype:** an NVMe block driver using DMA
  and MSI-X, a block protocol, an object store, and namespaced Raft
  term/vote/log/snapshot storage. Process-restart recovery is boot-tested;
  power-loss atomicity and unrestricted object sizes are not yet provided.
- **Experimental live service upgrade:** an EL0 service manager can spawn a
  replacement generation, transfer state, synchronize registration, and
  invalidate stale connections. This remains prototype work rather than a
  production upgrade framework.
- **Architecture and implementation documentation:** Markdown design notes and
  the LaTeX manual in [`docs/manual-v2`](docs/manual-v2) distinguish implemented
  behavior from intended architecture and record known limitations.

The automated AArch64 boot path exercises these mechanisms under QEMU, but this
is an experimental research and development system. A successful self-test is
evidence for the tested configuration, not a general reliability, security, or
hardware-compatibility claim.

For the upstream project, its history, and its community, please visit
<https://github.com/charlotte-os/charlotte-os>. Changes from upstream are kept
as ordinary Git history so that upstream updates can continue to be merged into
this fork.

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
[`docs/aarch64-port-status.md`](docs/aarch64-port-status.md) for a detailed
status report, including current limitations (device-tree discovery and
Linux KVM NIC validation). EL0 execution, isolated userspace services, IPC,
driver restart, and a two-node Raft election are boot-tested under QEMU TCG.

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
