# VMware Fusion AArch64 appliance

`scripts/build-vmware-aarch64.sh` packages an experimental ARM appliance for
VMware Fusion on Apple silicon. Image generation is separate from platform
qualification: a generated VM is not evidence that Knytos boots or runs
its services under Fusion ARM. The qualified Intel Fusion appliance is
documented [separately](vmware-x86_64.md).

## Build and open

Install the normal AArch64 build prerequisites, including the sibling Sitas
checkout for test builds, plus `qemu` (for `qemu-img`) and `mtools`:

```sh
brew install qemu mtools
./scripts/build-vmware-aarch64.sh release --minimal
```

Open this file in Fusion:

```text
os-images/vmware/Knytos-AArch64-Minimal.vmwarevm/Knytos.vmx
```

The default mode is `--tests`, matching the x86 builder. It includes the native
regression suite and writes `Knytos-AArch64.vmwarevm` instead. `debug` and
`release` select optimization independently of the boot mode. `--clean` rebuilds
cached AArch64 inputs. The builder only creates files; it does not start a VM.

The persistent disk defaults to 1 GiB; `--data-size-mib 64` selects a smaller
disk. Existing appliances are preserved unless `--replace` explicitly discards
their data, NVRAM and other VM state. Shut down and close the VM first; the
builder refuses a locked bundle. It prepares new images before removing an
existing bundle.

## Packaged hardware and disks

The ARM-specific VMX selects `arm-other-64`, virtual hardware version 21, UEFI,
four virtual CPUs and 1 GiB RAM. Secure Boot is disabled for the development
Limine image. Broadcom's
[ARM compatibility guidance](https://knowledge.broadcom.com/external/article/315602/)
requires ARM guests and UEFI on Apple silicon and lists SATA and NVMe disks.

- `knytos-boot.vmdk` contains `EFI/BOOT/BOOTAA64.EFI`, Limine configuration
  and the AArch64 kernel. It is attached through SATA in nonpersistent mode.
- `knytos-data.vmdk` is a blank persistent NVMe disk. If protected storage
  bring-up succeeds, the ordinary object-store startup formats it and installs
  signed services from the kernel's installation bundle. The QEMU runner's
  preseeded disk is not copied into this appliance.
- The kernel includes the framebuffer console for the firmware-provided
  display. The captured Fusion SPCR selects an 8-bit MMIO 16550 at `0x3fff03f8`
  (interface `0x12`, GSI 36); the kernel selects that register contract and writes
  boot diagnostics to `knytos-serial.log`.
- Networking is disabled. Broadcom recommends VMXNET3 for ARM guests;
  Knytos has no VMXNET3 driver, and its AArch64 service bundle does not
  include the x86 appliance's E1000E driver.

The image builder provisions development management HTTPS and the HTML dashboard
independently of test selection. Generated enrolled client certificates/keys and
the server CA are retained in `management/` beside the VMX, with restricted host
permissions and two-day validity. A caller-supplied resource retains its own
enrollment/storage. This prepares the service configuration; Fusion ARM still
needs working protected storage and a supported NIC before HTTP or outbound OTLP
can run. See [management TLS](../reference/management-tls.md).

## Bring-up limits

The [Fusion 26.0.1 investigation](../reports/investigations/2026-11-15-fusion-arm-console.md)
reproduced the initial silent boot in a separate VM. The kernel had interpreted
Fusion's 16550 as PL011 and spun on an all-ones status word before framebuffer
initialization. SPCR-aware serial selection fixes that hang. Group 1 GIC startup
also no longer unconditionally writes the Group 0 active-priority register that
Fusion rejects; single-security-state controllers retain their priority reset.
With both fixes the four-LP minimal guest initializes serial, a 1024-by-768
framebuffer terminal, GIC and services, then stops at durable-storage readiness.
The final diagnostic's visible, scrolling terminal output was also inspected.
Panic diagnostics currently remain serial-only. In test builds, the PL011-only
userspace UART fixture reports failure before granting hardware when SPCR
advertises Fusion's 16550. Synthetic DMA fixtures pair creation/destruction
backends rather than relying on a platform IOMMU for invented domain IDs.

The captured MADT describes GICv3 distributor `0x2c010000`, redistributors
`0x2c100000`, and a GICv2m MSI frame at `0x2c080000`. The current MSI allocator
only supports the QEMU-virt fixed GICv2m geometry or an ITS; it rejects this
frame and skips storage. MADT-driven GICv2m discovery remains required.

Protected userspace NVMe requires a discoverable, supported SMMUv3/IORT mapping.
The captured XSDT has no IORT and the guest reports no supported SMMUv3.
The x86 VMX's `vvtd.enable` requests Intel VT-d and is deliberately absent here;
it cannot establish an ARM IOMMU contract. If Fusion does not expose a usable
SMMU and interrupt path, storage/service startup will remain unavailable.
The builder neither disables DMA isolation nor selects QEMU's `hvf_compat`
fixture to bypass hardware checks.

Full Fusion ARM boot, first-boot installation, persistent restart, self-tests and
networking remain unqualified. The current terminal result is `local readiness
requires durable storage`, not a passing boot. Keep `vmware.log`, serial output and a
framebuffer capture from attempted boots when investigating the next platform
boundary. QEMU ARM results do not qualify this machine.
