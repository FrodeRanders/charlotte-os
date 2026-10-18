# Coherent-DMA CPU remap fence — 18 October 2026

## Finding and correction

Source comparison for the [DMA operation model](2026-10-18-dma-operation-model.md)
found a missing fence after restricted shared-DMA failure cleanup. Consuming
`FailedSharedDmaMemory::unmap` returns `OwnedMemory` after CPU unmap. An uncertain
kernel DMA result can still retain a coherent pin, but the memory-object map
gate checked only `exclusive_dma_pins`. A fresh mapping could therefore expose
safe Rust references while device access remained possible. Read-only CPU
access also needs exclusion because the device may write.

[`map_locked_with_cleanup`](../../../crates/catten/src/memory/object.rs) now
rejects every fresh CPU mapping while `dma_pins != 0`, returning `LendingActive`.
Shared DMA still permits its existing CPU mapping, installed before pinning and
used through the adapter's volatile protocol. No cleanup receipt, replay
permission, owner family or ABI is added. The pin and original backing charge
remain until genuine completion permits their release.

The [12 October failure-owner report](2026-10-12-dma-map-failure-ownership.md)
described retained pins as fencing remap. Its historical execution evidence did
not cover coherent fresh remapping. This report corrects that verification gap;
the historical report is preserved.

## Controlled failure and passing execution

The new [kernel regression](../../../crates/catten/src/memory/object/retirement_tests.rs)
creates a CPU mapping, installs two real coherent DMA pins, unmaps the CPU, then
requests fresh read-only and writable CPU mappings. It covers device-read,
device-write and bidirectional pins. Both mappings must reject before and after
the first unpin, without allocating frames or refunding the original charge.
After the last unpin, both mappings must succeed. Closing the object returns
its exact root's memory budget to zero before root close.

Before changing the gate, an ARM four-LP/SMMUv3 run failed at the first remap:

```text
panicked at crates/catten/src/memory/object/retirement_tests.rs:75:13:
assertion `left == right` failed
  left: Ok(VAddr(0x40000000))
 right: Err(LendingActive)
```

That runner exited 1. After the gate correction, separate ARM/SMMUv3 and
x86/Intel VT-d four-LP runs exited 0. Both printed:

```text
[DMA CPU remap fence] existing shared mapping, read/write remap rejection, three directions, two coherent pins, last-unpin reuse and original charge cleanup passed
```

ARM completed 17 deferred tests; Intel completed 15. Both reported zero failed
and zero pending tests. These were debug, fresh-storage, no-network QEMU runs
with `CARGO_BUILD_RUSTFLAGS='-D warnings'`. ARM services were built through
`scripts/build-catten-services.sh`; its passing repeat reused that bundle.

Reproduction commands (the first run used the pre-fix gate):

```sh
CARGO_BUILD_RUSTFLAGS='-D warnings' ./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 120 --instance dma-remap-red
CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings' ./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance dma-remap-arm
CARGO_BUILD_RUSTFLAGS='-D warnings' ./scripts/run-x86_64.sh debug --iommu intel --smp 4 --no-network --fresh-storage --timeout 180 --instance dma-remap-intel
```

| Run | Kernel SHA-256 |
|---|---|
| ARM before fix, failed | `4662ab1c98ab408b8ebfaddc06b78a21dd3ea9360898c8bdbcd825e55b4be9c6` |
| ARM after fix, passed | `40c5a3ade0d154ab107d16fc9dd5f590cdcd15776e2497a1639f121310ebfba4` |
| Intel after fix, passed | `0405bb9e677599975a5f48eb10091126a36e6742603162cc3ae04462d632aa21` |

The [version-controlled evidence](evidence/2026-10-18-coherent-dma-remap-fence.json)
retains failed and passing observations separately, serial hashes, kernel and
boot-image hashes, service/tool hashes, QEMU arguments, source hashes and check
results. Baseline revision is `1c6963a2887d7d69472af9179374f81ecd0750d4` with the
regression added; passing kernels also include the map gate correction.

## Other checks and limits

`cargo fmt --all -- --check`, strict kernel Clippy on both targets, and all
517 host tests passed. Both DMA models now require no retained DMA pin before
fresh CPU mapping. The expanded suite passed 28 safe configurations and all 64
required counterexamples, including `UnsafeCoherentRemap` violating
`CpuOwnerNeedsNoDma`.

The native control uses real kernel pins and mappings directly. It does not
inject an uncertain IOMMU hardware acknowledgement or execute the complete
failed-wrapper path from EL0. Existing volatile shared access still requires
its protocol's device/CPU synchronization. Wider IRQ/guard/destructor contexts,
quarantine recovery and SEC-07/18 remain partial. This closes a concrete C18
remap path without qualifying those broader boundaries.
