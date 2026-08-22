# VMware x86-64 appliance

CharlotteOS can be built as a two-disk VMware appliance. It has been qualified
with VMware Fusion on an Intel macOS host using UEFI, four virtual CPUs, a
virtual NVMe controller, an E1000E adapter, and VMware's guest-visible VT-d
implementation.

## Build and open the appliance

Install the normal x86-64 build dependencies plus `qemu-img`. On macOS they
are provided by Homebrew's `mtools` and `qemu` packages:

```sh
brew install mtools qemu
scripts/build-vmware-x86_64.sh release
```

The default persistent disk is 1 GiB. For a smaller development appliance:

```sh
scripts/build-vmware-x86_64.sh release --data-size-mib 64
```

Open the generated file in Fusion or Workstation:

```text
os-images/vmware/CharlotteOS.vmwarevm/CharlotteOS.vmx
```

The builder refuses to overwrite an existing appliance because its data VMDK
may contain installed or upgraded services. Move that bundle somewhere safe,
or pass `--replace` when discarding it is intentional.

## Disk and installation model

The appliance deliberately has two disks:

- `charlotte-boot.vmdk` is a small FAT boot image on a virtual SATA controller.
  It contains `BOOTX64.EFI`, Limine configuration, and the kernel. VMware opens
  it in independent nonpersistent mode.
- `charlotte-data.vmdk` is a blank persistent virtual NVMe disk. CharlotteOS
  owns it from LBA zero; it is not partitioned or FAT-formatted by the host.

On first boot, the object-store service recognizes an unformatted data disk,
creates the COBJSTR3 layout, and the kernel installs missing signed service
artifacts from its immutable x86-64 installation bundle. A successful first
boot includes output like:

```text
[store] bootstrap seed complete: retained=0 written=27
[nvme] DMA domain completed the transfer without translation faults
SELFTEST COMPLETE: passed=17 failed=0 pending=0
```

The shared 4 MiB userspace heap accommodates the object store's allocation
bitmap, directory, and mirrored-directory mount buffer for this 1 GiB disk.
Reducing that arena without also changing the store's metadata representation
will make the service abort while formatting or mounting larger disks.

Later boots validate every existing artifact against its logical name and
signature. Valid stored artifacts are retained, including newer versions
installed through `clusterctl`; only missing or invalid entries are restored
from the boot bundle. A normal second boot therefore reports:

```text
[store] bootstrap seed complete: retained=27 written=0
```

This replaces the host-side preseeding used by the default QEMU raw disk.
Copying or backing up the data VMDK preserves the installed object store.
Rebuilding with `--replace` creates a new blank data disk and loses that state.

## Required virtual hardware

The supplied VMX configures the qualified combination:

- x86-64 UEFI firmware with Secure Boot disabled;
- four virtual CPUs and 1 GiB RAM;
- one SATA boot disk and one NVMe data disk;
- `vvtd.enable = "TRUE"` for protected userspace DMA;
- COM1 redirected to `charlotte-serial.log`, appending across boots; and
- an E1000E adapter attached to VMware NAT; and
- one PCIe root-port group, sufficient for the storage controllers and NIC.

Do not replace the NVMe data controller with SCSI or SATA: the current VMware
appliance path starts the userspace NVMe driver. Do not disable virtual VT-d:
CharlotteOS refuses to delegate unrestricted DMA when no usable IOMMU exists.

The VMDKs use VMware's sparse monolithic format. Fusion and Workstation can
open the generated VMX directly. For ESXi, upload/import the disks with the
normal datastore tooling and reproduce the VMX settings above; that workflow
has not yet been qualified by this repository.

## Networking

The supplied VMX attaches VMware's emulated Intel 82574L (`e1000e`) adapter to
NAT. CharlotteOS discovers controllers behind every function of VMware's
multifunction PCIe root ports, delegates the adapter's BAR, MSI-X interrupt,
and requester-specific VT-d domain to the userspace E1000E driver, waits for
link negotiation without busy waiting, and then publishes the hardware-neutral
`net0` service.

Serial output makes that sequence visible even though CharlotteOS does not
need console input:

```text
[e1000e] found Intel 82574L at 04:00.0 (...)
[net] selected E1000E at BAR0=... (...)
[net] started E1000E userspace driver ...
[net] SUCCESS: E1000E is online, link up, MAC ..., hardware TX completions=1.
```

Discovery, distributed DNS, smoltcp TCP/IP, HTTP, and cluster code consume
`net0` and therefore require no E1000E-specific changes. Those layers have
been exercised between two QEMU E1000E guests. Paired VMware guests and
bridged or host-only VMware networking have not yet been qualified. VMXNET3
and the older E1000 model remain unsupported.

## Qualification evidence

The Fusion qualification exercised a blank first boot and a restart using the
same data VMDK. Both boots passed all 17 registered tests without VT-d
translation faults. The first boot formatted the 1 GiB store and installed 27
artifacts; the second retained all 27, remounted persistent metadata, and
advanced the persisted Raft term. The network run found the adapter behind a
multifunction root port, brought link up, and completed a hardware transmit.
Two synchronized QEMU E1000E guests also pass cluster discovery and the smoltcp
TCP exchange.
