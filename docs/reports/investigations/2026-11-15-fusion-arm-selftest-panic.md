# Fusion ARM self-test fixture panics

Investigated 15 November 2026 from `a551159f`. The rebuilt test appliance
reached kernel tests on Fusion 26.0.1, build 25689522, then panicked. This is
separate from the earlier [console and platform investigation](2026-11-15-fusion-arm-console.md).
Diagnostic VMs used separate immutable boot copies and fresh 64 MiB data disks;
the user's running appliance was not modified or stopped.

## Captured failures and cause

The kernel extracted from the user's boot VMDK had SHA-256
`deba44f5e3378b64a88d5324c4deae88ec1c565fe59b19fd7bda7aeb189a0bc1`.
Its serial log reported `CHARLOTTE_BOOT_MODE=tests`, successful framebuffer
initialization and no supported SMMUv3, followed by:

```text
[+     2.956444] [device node admission] ... passed
Kernel panic:
panicked at crates/catten/src/device/registry/tests.rs:156:35:
called `Result::unwrap()` on an `Err` value: DmaInvalid
```

The registry fixture published a synthetic DMA domain (`u64::MAX`, explicitly
without hardware backing), then called the public close adapter. That adapter
selected the real SMMU destruction backend. QEMU's installed SMMU accepts an
absent domain as already gone; Fusion has no installed supported SMMU and
returns an error before domain lookup. Unwrapping that error panicked. This
was a synthetic test/backend mismatch, not evidence of failing physical DMA
maintenance or a permission to bypass DMA isolation.

Correcting that fixture alone produced kernel
`a4c1ada62fa367874a83174d1ca45de29ea663f558c08d4740da22225016149d`.
Fusion then passed `[device registry ownership]`, but exposed the same
cleanup mismatch in `device/mapping/tests.rs:233:31`, again `DmaInvalid`.
The mapping fixture had already exercised injected close rejection and retained
authority successfully; its final successful cleanup selected the real backend
for the synthetic domain.

With both DMA fixture corrections, kernel
`e1ab76fd3f8bf6a3d401dc975ec0b7d8fb334180a1c91f66cbc8d2465b7e8508`
passed the registry, DMA close/mapping/creation and device-retirement controls
on Fusion. A separate QEMU four-LP framebuffer run of that exact kernel passed
all 17 deferred tests:

```text
[+    42.982776] SELFTEST COMPLETE: passed=17 failed=0 pending=0 passed_bitmap=0x1ffff failed_bitmap=0x0 pending_bitmap=0x0
>>> All registered deferred self-tests passed.
```

The Fusion repeat then exposed another fixture assumption:

```text
[+     4.573652] [launch] SKIP storage: no supported MSI mechanism.
[+     4.601197] Testing EL0 userspace UART driver (delegated MMIO + interrupt)...
panicked at crates/catten/src/service/supervisor.rs:1232:13:
[supervisor] MMIO region grant failed: NotPageAligned (owner=139, base=0x3fff03f8, pages=1)
```

The PL011-only userspace test still treated SPCR's base as a PL011 window.
Fusion advertises a byte-MMIO 16550, whose base is not page aligned. Rounding
that grant would not make the PL011 driver compatible with the device.

## Fixture contracts

Synthetic DMA controls now pair their invented IDs with fixture destruction
through the real containing close owner. Root/capability claims, original
charges and post-completion metadata disposal remain exercised. The production
IOMMU backend and its failure semantics are unchanged.

The userspace UART fixture now checks the validated SPCR register family and
page alignment before preparing any client namespace or hardware grants.
Unsupported hardware explicitly fails the registered UART test; it does not
claim a pass or silently shrink the expected suite. A supported PL011 base is
captured with the fixture state and reused for driver restart.

These fixture corrections do not add MADT-driven GICv2m discovery, a supported
Fusion SMMU/IORT contract, or a VMXNET3 driver. Full Fusion operation and a
passing complete self-test suite remain unqualified.

## Final qualification

Final test kernel SHA-256:
`d85a086efb390544215706f38aa15de55bd03ed57db9877fa21d90b16fd72a0e`.
The separate Fusion boot passed the corrected DMA registry/close controls and
reported:

```text
[+     4.114962] [launch] SKIP storage: no supported MSI mechanism.
[+     4.135911] [uart] fixture unavailable: firmware console is not a supported page-aligned PL011
[+     4.136610] Testing EL0 sitas (Rust-compiled knytos-user binary)...
```

No subsequent serial progress or complete self-test summary was observed before
manual termination. Source inspection shows that this application is loaded
through the unavailable object store; this is consistent with the remaining
storage blocker, not a complete Fusion acceptance result. The UART failure bit
is set before returning from its unsupported fixture. No registered test is
silently marked passed or removed.

The final exact kernel ran under QEMU 11.1.2 with four LPs, the existing ramfb
display, a supported SMMU and the maintained runner:

```text
flanterm: framebuffer terminal initialised successfully (800x600, scale=1, heap=230056)
[+    20.169774] SELFTEST COMPLETE: passed=17 failed=0 pending=0 passed_bitmap=0x1ffff failed_bitmap=0x0 pending_bitmap=0x0
>>> All registered deferred self-tests passed.
```

The runner exited zero. Strict AArch64 kernel Clippy with
`acpi,display,virtio_gpu,kernel_tests`, workspace formatting, whitespace checks
and the five host UART/parser controls passed. An initial UART-guard build
exposed a second base lookup in driver restart; the final fixture retains its
validated base for that restart. Earlier failing native kernels remain separate
above. No x86 repeat or physical-hardware qualification was performed.
