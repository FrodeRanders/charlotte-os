# VMware x86-64 appliance

CharlotteOS can be built as a two-disk VMware appliance. It has been qualified
with VMware Fusion on an Intel macOS host using UEFI, four virtual CPUs, a
virtual NVMe controller, and VMware's guest-visible VT-d implementation.

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
[store] bootstrap seed complete: retained=0 written=26
[nvme] DMA domain completed the transfer without translation faults
SELFTEST COMPLETE: passed=7 failed=0 pending=0
```

Later boots validate every existing artifact against its logical name and
signature. Valid stored artifacts are retained, including newer versions
installed through `clusterctl`; only missing or invalid entries are restored
from the boot bundle. A normal second boot therefore reports:

```text
[store] bootstrap seed complete: retained=26 written=0
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
- one PCIe root-port group, sufficient for the two storage controllers.

Do not replace the NVMe data controller with SCSI or SATA: the current VMware
appliance path starts the userspace NVMe driver. Do not disable virtual VT-d:
CharlotteOS refuses to delegate unrestricted DMA when no usable IOMMU exists.

The VMDKs use VMware's sparse monolithic format. Fusion and Workstation can
open the generated VMX directly. For ESXi, upload/import the disks with the
normal datastore tooling and reproduce the VMX settings above; that workflow
has not yet been qualified by this repository.

## Current limitation: VMware networking

Storage and node-local services work in this appliance, but networking does
not. The x86-64 network service currently drives virtio-net PCI devices, while
VMware exposes VMXNET3, E1000E, or E1000 adapters. The supplied VMX therefore
has no virtual NIC.

Consequently discovery, distributed DNS, smoltcp TCP/IP, HTTP, and multi-node
cluster operations remain qualified on QEMU but are unavailable inside VMware
until CharlotteOS gains a VMXNET3 or E1000E userspace driver. Adding a VMware
NIC in the UI does not make it usable by the current kernel.

## Qualification evidence

The Fusion qualification exercised both a blank first boot and a clean
power-off/start of the same data VMDK. Both runs passed all seven registered
x86-64 tests. The second retained all 26 service artifacts, completed the
NVMe and persistent Raft restart tests, and recorded no VT-d translation
faults. QEMU's corresponding blank-disk and retained-disk runs also pass.
