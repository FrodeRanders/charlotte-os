# Cryptographic entropy

CharlotteOS exposes two fallible cryptographic entropy paths:

- the `RandomU64` syscall uses Arm `RNDR` or x86-64 `RDRAND` when the
  architectural instruction is available; and
- the node-local `rng` service uses a modern VirtIO RNG PCI device.

Security-sensitive callers fail closed when neither path can satisfy a
request. There is no clock-, counter-, or deterministic-seed fallback.

## VirtIO RNG service

The steady-state launcher discovers PCI device `1af4:1044` (or its
transitional `1af4:1005` identity), creates a private protected-DMA domain for
that requester, and launches `rng.elf`. The driver receives only its four-page
modern VirtIO MMIO window and DMA-domain capability. It polls the queue and is
therefore not granted an interrupt capability.

The service registers the short name `rng`. `OP_FILL` accepts a requested byte
count from 1 through 4096 and returns the exact initialized length plus a moved
memory object. The driver handles short device completions by resubmitting
until the complete request has been filled.

Virtqueue memory uses `catten_rt::owned::SharedDmaMemory`. This owner keeps the
memory object, CPU mapping, and DMA mapping together; access is volatile, and
drop order removes device access before unmapping CPU access and closing the
memory capability. Setup and teardown errors retain the precise mapped or
unmapped owner so callers cannot accidentally adopt a handle twice.

## QEMU

The ordinary AArch64 runner attaches the device even when no test verifier is
selected:

```text
-object rng-random,filename=/dev/urandom,id=charlotte-rng
-device virtio-rng-pci,rng=charlotte-rng,disable-legacy=on,iommu_platform=on,addr=4
```

`iommu_platform=on` is required: CharlotteOS negotiates
`VIRTIO_F_ACCESS_PLATFORM` and maps the queue only through the device's SMMU
domain. This supplies entropy to QEMU's named `cortex-a710` model without
pretending that the CPU implements `RNDR`.

The S3 TLS service prefers architectural random words and transparently uses
the `rng` service for any remainder. Other applications should resolve `rng`
through the name service and retain the returned owned connection rather than
accessing VirtIO or DMA directly.
