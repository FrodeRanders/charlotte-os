# DMA-map diagnostics and restricted failure ownership

This continues SEC-18, C13/C18 and G2/G3/G7 after the
[grant cancellation readiness correction](2026-10-12-dma-grant-cancellation-readiness.md).
It corrects a shared-DMA error-owner exposure and preserves per-call map rejection.
It adds no public wait, polling/retry loop, hardware completion proof or registry.
SEC-18 remains partial and finding counts are unchanged.

## Captured failure and causal limit

The previous AMD boot used kernel SHA-256
`d4d230248cc0fa4ad0fc47bcb936ee041f77433414f009e8fb6ca45d1460a926`.
At 17.362002 s, NIC startup rejected stage 4 with `DmaDomainMap detail=0x3`.
The old driver recorded requested pages when the syscall returned zero:
this was a failed three-page map, not a queue index or kernel status. At
47.874025 s, results were 15 passed / 1 failed / 2 pending: passed bitmap
`0x1bbff`, failed `0x20000`, pending `0x1200000`. Net failed and HTTP/DHCP
remained pending; the router verifier's deadline was downstream of NIC rejection.
The runner exited 1 because host HTTP validation failed. An identical-artifact
fresh-storage repeat passed 18/0/0 at 18.379689 s.

Both observations remain valid. The old ABI discarded the map cause, so neither
this correction nor a passing repeat establishes whether that rejection was
backend contention, memory/metadata admission or hardware failure. The previous
report preserves the source deadline and matching-symbol backtrace. The new ABI
cannot recover the historical lost cause or prove cancellation caused it.

## Per-call diagnostics

Both map syscall forms now return status in register 0, IOVA in register 1,
and diagnostic stage/cause in register 2. Rejection clears IOVA; success clears
diagnostics. There is no global last-error state and no compatibility syscall.
The SDK returns `Result<u64, DmaMapError>`; its compact driver detail packs
status in bits 0..7, stage in 8..15 and backend cause in 16..23.

| Stage | Meaning |
| --- | --- |
| 1: validation | Invalid direction, before operation admission. |
| 2: public admission | Original capability/root rejection before backend work. |
| 3: backend | Original backend category, including pin/admission, walking, maintenance or rollback rejection. |
| 4: public completion | Backend success followed by rejected exact capability/root completion; no IOVA is exposed. |

Backend causes 1..13 are Unsupported, InvalidStream, StreamInUse,
InvalidDirection, Memory, OutOfIova, MapFailed, AlreadyMapped, UnknownDomain,
UnknownMapping, HardwareTimeout, OperationInFlight and WaitRegistrationFailed.
Cause 0 means no backend category. Memory remains the backend's existing pin
rejection category; it does not expose the inner memory-object error.

`DmaOperation` still retains the exact root and in-flight capability claim through
backend work. Ordinary return restores the registered domain/engine before
finishing this claim. If backend rejection and public completion both fail, the
original backend rejection takes diagnostic precedence; the failed completion
still retains the original lease/count. Abandonment retains the same owners.
Diagnostics are not rollback receipts. In particular, backend stage includes
both pre-claim admission and started maintenance; a busy-looking cause cannot
authorize another map. This does not import the unmap ABI's pre-work busy contract.

NIC startup logs the page count and direction separately, preserves `DmaMapError`
in its first startup failure detail, and keeps its terminal teardown behavior.
Other raw hardware adapters accept the checked result and retain their existing
map-failure protocol behavior; they do not add retries. Existing kernel fixture
adapters use the same owned operation and reduce diagnostics to device status
for their scoped assertions.

## Shared-DMA error ownership

Previously, `OwnedMemory::map_shared_dma` created a writable CPU mapping before
DMA setup and returned that `MappedMemory<Writable>` in its error. A backend
failure can follow partial publication and uncertain rollback with a quarantined
DMA pin. The returned mapping could then create Rust slices while the device
might still access those pages. Blocking new kernel mappings does not revoke
this already-established CPU mapping.

The error now retains that exact mapping inside `FailedSharedDmaMemory`. Its
field is private and it exposes no pointer, slice, volatile access or DMA retry.
A consuming CPU `unmap` returns the memory capability on confirmed CPU-unmap
success, or returns the same restricted owner on rejection. Kernel pins still
fence subsequent map/lend/release while DMA cleanup is uncertain. Error Drop
uses the existing best-effort CPU/capability teardown; it never submits DMA
unmap, discharges uncertain pins or refunds their original backing charges.

Success still consumes the CPU mapping into the existing `SharedDmaMemory`,
with volatile protocol access and the existing unmap progress owner. Exclusive
DMA failure preserves the original diagnostic and memory capability without
creating a CPU mapping. No new kernel owner family, custody or retry owner is
introduced. Conservative restriction also applies to unstarted rejection;
ordinary callers can explicitly CPU-unmap before attempting a fresh CPU mapping.

## Regression evidence

Host assertions cover exclusive failure diagnostics and exact memory ownership,
shared failure Drop without DMA work, rejected CPU unmap returning the restricted
owner and subsequent explicit CPU completion. A compile-fail doctest rejects
slice access through the restricted owner. The SDK's host harness is enabled
for register-result decoding; it does not execute native CharlotteOS syscalls.
AArch64 emitter coverage includes both three-result map forms.

Synthetic dispatcher probes exercise validation, unknown capability, backend
memory-pin rejection and an in-flight public claim for both coherent and
exclusive map numbers. Backend busy, timeout and mapping-failure categories
execute once through the same public owner and retain their original category.
These probes do not prove live-hardware concurrent readiness. The matrix does
not independently exercise exclusive-map success from EL0; exclusive dispatcher
rejections and injected runtime ownership cover that form's scoped changes.

The existing real sparse-map rollback fixture now enters through the dispatcher:
it verifies backend `MapFailed`, cleared IOVA, retained table charges and a data
pin that rejects memory close until confirmed domain retirement. Its injected
rollback rejection is an expected negative probe, not a guest failure. Existing
real map/unmap/reset-exclusion, staged cancellation and guarded abandonment
assertions remain in the same guest profiles and owner families.

## Failed Intel run retained separately

The first Intel run used kernel SHA-256
`11ac50b240c5bb62d81ce6f6ba2d94f79890bbd24f5b53d2a964b18540ce2351`.
All synchronous map diagnostics and real rollback/pin-retention assertions
completed. Network, HTTP and DHCP reached their passing states. At 23.429878 s,
the log recorded `abort tid=35 lp=3 asid=0`, followed by a panic at
`crates/catten/src/memory/object/close/tests.rs:231:5`:

```text
assertion failed: crate::completion::wait_timeout(SCHEDULED_CLIENT, watched, 5_000).unwrap()
```

This existing fixture had observed its memory closer Blocked, retained the
exact root and dropped the transfer to produce the wake, then waited for its
exact-generation thread-exit completion. The capture establishes failure to
observe that completion within five seconds, not whether source wake,
scheduling, close work or exit observation caused it. The adjacent abort diagnostic
alone does not resolve that distinction. It is different from the previously
corrected missing-completion-namespace fixture failure.

The last waiting observation at 344.364723 s was 17 passed / 1 pending,
passed bitmap `0x123abff`, pending `0x1000` (cq-wait). No authoritative terminal
result was produced; do not interpret its zero failed bitmap as a passing run.
The runner exited 1, `guest HTTP keyhole was not validated from the host`.
Deployment's initial status control passed, then a partial-body request
exhausted its bounded transport retries with `TimeoutError: timed out`.
Subsequent node/cluster controls timed out. Their common cause with the fixture
panic is not established.

Matching immutable symbols resolve the raw backtrace as follows:

| Return address | Resolved symbol |
| --- | --- |
| `0xffffffff8027972e` | `__rustc::rust_begin_unwind+0x4e` |
| `0xffffffff802d0ffb` | `core::panicking::assert_failed::<usize, usize>` |
| `0xffffffff802d16e1` | `core::panicking::panic+0x11` (nearest preceding symbol) |
| `0xffffffff80120997` | `catten::self_test::cq_wait::cq_driver+0x6567` |
| `0xffffffff8000674c` | `kernel_thread_trampoline+0x8` |

An identical-artifact fresh-storage Intel repeat completed 18/0/0 at
18.185136 s and passed host ingress controls, runner 0. Its scheduled close
wake and later cancellation/IPC readiness assertions completed. This repeat
does not supersede the failed sample or establish reliability/causality.
No speculative scheduler or timeout extension was made for that capture.

## Validation and artifacts

Both signed service bundles were rebuilt. Final host suite: **393 passed,
0 failed, 4 ignored documentation examples**. This includes the SDK decoding
unit test, two additional runtime failure-owner tests and the compile-fail access
test. Strict `-D warnings` Clippy passed for services and both kernel profiles.
Formatting, diff, AArch64 emitter coverage and changed Markdown links passed.

| Profile | Result / last observation | Bitmap | Completion time | Runner |
| --- | --- | --- | --- | --- |
| ARM SMMUv3 security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 57.258947 s | 0 |
| Intel VT-d first run | No terminal result; 17 passed / 1 pending | passed `0x123abff`, pending `0x1000` | last observation 344.364723 s | 1 |
| Intel VT-d identical-artifact repeat | 18 / 0 / 0 | `0x123bbff` | 18.185136 s | 0 |
| AMD-Vi HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 24.980099 s | 0 |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.506540 s | 0 |

The four passing runs validated host ingress controls; shutdown requested PSCI
power-off at 14.493276 s. Existing bounded transport retries were used by ingress
probes. The failed Intel sample remains separate from its passing repeat.
Synchronous assertions supplement the deferred result counts. No passing guest
reproduced the old AMD map rejection; that is not proof of its resolution.

Commands used `CATTEN_SKIP_EMBED_BUILD=1` after bundle builds:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance dma-map-arm
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-map-intel
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-map-intel-repeat
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance dma-map-amd
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance dma-map-shutdown
```

HTTP/deployment ports were 19349/18649, 19350/18650, 19353/18653,
19351/18651 and 19352/18652. Intel repeat and AMD used
`CATTEN_SKIP_KERNEL_BUILD=1` only after the current x86 ELF matched the original
Intel boot hash. Immutable per-boot hashes precede later profile rebuilds:

| Artifact | SHA-256 |
| --- | --- |
| ARM combined kernel | `e2a8e06da115beb00ad13678d5b66f779afe2ab53b4a43ddb4b4a85c1a5297c6` |
| Failed Intel / Intel repeat / AMD kernel | `11ac50b240c5bb62d81ce6f6ba2d94f79890bbd24f5b53d2a964b18540ce2351` |
| ARM shutdown kernel | `57db7954f854fd9f369dd259d04aa3330845e4ac2d80ed0bf6b1f48cce1d2dd7` |
| ARM bundled service ELFs | `db645c0e1b527a6521fdfcd79dd7e498637b1a650024f8fbbeb82cb3c8b30f7a` |
| x86 bundled service ELFs | `0a4a369f13576fbddd304e76b0facc268ea31da31e9d50cd6707ced075bd9bb2` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256. Results, relevant failures, matching symbols and hashes are inline;
temporary logs and ignored artifacts are not report dependencies.

## Remaining work

Public map/unmap/close and whole-domain device cleanup still need exact owning
readiness admission and caller IRQ qualification. Never force-enable IRQs under
an unknown entry mask or replay map/cleanup after an ambiguous failure. Captured
AMD causality, the new Intel scheduled-close/ingress failure and the older Intel
grant failure remain unresolved. Live-hardware
contention, outstanding-I/O pressure, broader metadata/outer-context qualification
and custody remain separate SEC-18 acceptance work. Fresh-run success is not a
reliability or physical-platform claim.
