# Fusion ARM early-console and platform bring-up

Investigated 15 November 2026 from baseline
`375073a0a9eaedb8f4e490c996a79f4af834465b`, using Fusion 26.0.1 build 25689522
on an Apple silicon Mac. The initial appliance had an empty serial log and a
black display. Reproduction used separate VMs, copied boot media and fresh data
disks; the experiments did not edit the user's appliance or its virtual hardware.

The [retained evidence](evidence/2026-11-15-fusion-arm-console.json) records the
captured ACPI bytes, checksums, kernel/loader identities and separate outcomes.
Complete temporary logs, binaries and guest memory are not published. Selected
diagnostics below and in that JSON supply the durable record.

## First blocker: UART register-family mismatch

Fusion boots the kernel. A debugger capture of the unmodified reproduction
identified the BSP at:

```text
PC = 0xffffffff8050f804
<catten::log::serial::Pl011 as core::fmt::Write>::write_str + 540
callers: core::fmt::write + 144; bsp_main + 772
w8 = 0xffffffff
ldr w8, [uart_base, #0x18]
tbz w8, #5, transmit
```

The SPCR has a valid checksum, revision 2, interface `0x12`, SystemMemory GAS,
8-bit width, byte access, base `0x3fff03f8` and GSI 36. This describes a
16550-compatible interface with GAS, not PL011. See Microsoft's
[SPCR](https://learn.microsoft.com/en-us/windows-hardware/drivers/bringup/serial-port-console-redirection-table)
and [DBG2 subtype definitions](https://learn.microsoft.com/en-us/windows-hardware/drivers/bringup/acpi-debug-port-table).

The old discovery helper returned only the base. The logger then performed
32-bit PL011 flag reads at offset `0x18`, rather than byte 16550 line-status
reads at offset 5. The captured all-ones value kept PL011's transmit-full test
true forever. This occurred during the first kernel message, before the first
ordinary log could initialize the framebuffer. APs were still waiting in
bootloader secondary startup. It was a live kernel polling hang, not evidence
of a failed ELF or UEFI handoff.

The corrected console preserves the SPCR register family and access contract.
It accepts implemented memory interfaces, rejects invalid checksums, truncated
tables, unsupported interfaces/access widths and unsafe addresses, and does not
substitute a QEMU address after rejecting a present table. Missing SPCR retains
the existing QEMU-virt fallback. A stuck or all-ones transmitter disables the
sink after bounded polling, without enabling interrupts or blocking subsequent
framebuffer logging. The boot mapping and caller's IRQ policy remain unchanged.

## Second blocker: inaccessible Group 0 priority bank

With the UART correction, the guest prints initialization, creates the
1024-by-768 terminal, starts all four LPs and reaches GIC initialization. The
next capture records:

```text
Unsupported MSR(S3_0_C12_C8_4, 0x0): vpc=0xffffffff8007e758
debugger: each LP in ivt + 512 during GIC initialization
```

That encoding is `ICC_AP0R0_EL1`. The old CPU-interface initialization
unconditionally reset this Group 0 active-priority bank. An intermediate
Group-1-only control progressed past it. The final implementation retains
Group 0 firmware-priority reset only when the discovered distributor's
single-security-state `DS` bit grants that access; Group 1 initialization stays
active on the restricted hosted interface. No vendor-name exception or
manifest-derived platform classification was added.

The final Fusion guest has visible, scrolling framebuffer text and working
serial logging. Visual inspection confirmed ordinary boot lines, including
local-controller initialization, the ACPI catalog and storage rejection.
Panic diagnostics remain serial-only, so the framebuffer stops at the last
ordinary log message. This qualifies the observed terminal output, not a GPU
driver, input console, framebuffer retirement or complete platform boot.

## Remaining Fusion blockers

The captured XSDT contains FACP, GTDT, MCFG, APIC, SSDT, PPTT and SPCR. It has
no IORT. MADT describes GICv3 distributor `0x2c010000`, redistributors
`0x2c100000`, and a GICv2m MSI frame at `0x2c080000`.

The current MSI path accepts an ITS or the fixed QEMU-virt GICv2m geometry.
It does not consume Fusion's MADT MSI frame. The guest also discovers no usable
SMMUv3 for protected userspace DMA. With both console/GIC fixes it reports:

```text
[smmu] no supported SMMUv3 discovered; DMA isolation unavailable.
[launch] SKIP storage: no supported MSI mechanism.
Kernel panic:
local readiness requires durable storage
```

This is an explicit failed boot, despite successful console and multiprocessor
initialization. MSI discovery alone would not supply DMA isolation. No bypass
or `hvf_compat` profile was used. The VMX also leaves its NIC disabled; VMXNET3
is unsupported. HTTP and OTLP therefore remain unavailable on this appliance.
These hardware statements describe the captured VM, not every possible Fusion
version/configuration. The PL011 userspace regression driver also remains a
platform-specific fixture rather than a new 16550 userspace driver.

## Separate QEMU framebuffer observations

QEMU 11.1.2 was tested with the identical intermediate display-enabled minimal
kernel (`d2f1aa040f6f58931e24141a32909993dd54d14637ae6e0535f67d1ec61dbda6`):

| Display configuration | Observation |
| --- | --- |
| Headless | Runner passed mandatory minimal memory/storage/operational validation. |
| Existing ramfb configuration | No CharlotteOS serial output; stopped manually while the display showed TianoCore and PC was `0x5fdf74a8`, before kernel entry. The precise firmware/bootloader boundary remains unresolved. |
| VirtIO GPU substituted at PCI slot 5 | Guest services started, but Limine supplied no framebuffer. The screen reported inactive output. This is not a display pass or an adopted replacement. |

The retained [ramfb view](evidence/2026-11-15-qemu-display.png) and
[VirtIO GPU view](evidence/2026-11-15-qemu-virtio-gpu.png) preserve these different
outcomes. No flanterm renderer change or QEMU display-device replacement was
made. These captures do not establish the cause of every earlier QEMU black
screen, nor turn a serial-only boot into a framebuffer qualification.

A later four-LP repeat used the final guarded Group 0 minimal kernel
(`dfc17550d0162391979dfeb7cd5535e4f70632ae5985aa9c6164ec2edd9817d9`)
with the existing runner's `release --minimal --display --smp 4` configuration,
the same QEMU/edk2 installation and `ramfb`. It initialized an 800×600 terminal,
scrolled ordinary boot text, and displayed `BOOT COMPLETE: validation passed;
local storage ready; services submitted`. The authenticated reader request to
`/` returned HTTP 200 and CharlotteOS HTML with CA verification enabled. The
[passing framebuffer capture](evidence/2026-11-15-qemu-framebuffer-pass.png)
and separate artifact/serial record in the evidence JSON qualify this repeat.
No display-device or renderer change was needed. This pass does not explain
or close the earlier pre-kernel stall.

A separate `debug --tests --display --smp 4 --timeout 180` run used kernel
`f18ad151192ba9929fb9b919bc7676f46bf19047156aa9d7d7a4abba6f02ee21`.
It initialized the 800×600 framebuffer and completed all 17 registered deferred
self-tests with zero failures or pending tests; the runner exited successfully.

## Validation and limits

Five host controls exercise the production parser and actual volatile UART
accesses against host register backing: register family/width, receive
readiness, stuck/missing transmitters, truncated/bad-checksum tables and unsafe
GAS/address rejection. The existing C-terminal controls pass allocation/layout,
failure-prefix, rendering and boot-lifetime ownership checks.

Four-LP QEMU regression boots qualify PL011 operation and the scheduler,
interrupt, IPC and storage paths; the retained run records distinguish the
intermediate and final guarded Group 0 versions. Strict kernel Clippy, workspace
formatting and whitespace checks are part of final validation. SBSA firmware
and physical hardware were not available for a repeat in this investigation.

An initial direct build used a missing service-bundle path and the wrong host
archiver; the maintained runner rebuilt/staged services and supplied LLVM ar.
A packed-field borrow and a test-only unused assignment were corrected before
native qualification. Strict Clippy subsequently required a typed SPCR error.
Early debugger attempts lacked a surviving launcher; later captures retained
its session. The first headless comparison collided with the still-running
display guest's host port; its passing repeat used the same kernel after that
guest stopped. These setup failures are not native acceptance evidence. The
original UART hang, GIC failure, final durable-storage panic and unsuccessful
QEMU display controls remain separately recorded above.
