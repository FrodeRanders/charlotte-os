# RNG descriptor ownership and terminal failure — 2026-10-26

This follows `5a16e903` and the
[server-readiness correction](2026-10-26-security-rng-readiness.md). The old
driver returned `false` after a device timeout without fencing its descriptor
position. A later request could publish/notify that position again, and ordinary
field destruction could reset/unmap a backend with an uncertain operation.
The replacement retains the complete device owner and refuses further hardware
calls after failure. This corrects that software reuse/destructor path; it is
not a hardware recovery or physical-quiescence implementation.

The [tracked evidence](evidence/2026-10-26-security-rng-descriptor-fence.json)
contains source/executable hashes, complete signed-bundle maps, kernel,
boot-image and initial-disk hashes, commands, regression diagnostics and
separate preliminary/final results. No temporary log is an evidence dependency.

## Owner and completion boundary

`rng_device::Device` consumes one initialized, empty backend. The concrete
`VirtioRng` adapter owns MMIO, queue/bounce CPU mappings, DMA mappings and their
original capabilities/charges. One captured attempt records previous/expected
indices, slot, requested length and already copied prefix. Terminal retention
is armed before the first descriptor mutation, including publication that
partially succeeds or a backend callback that unwinds.

One submission publishes/notifies once. Polling accepts only the exact next
wrapping used index; another nonprevious index rejects. A complete used entry
must identify descriptor zero and a positive length no greater than requested.
Only after those checks can the private entropy bytes be copied and the
completed position advance. Confirmed short completions continue the remaining
prefix. Host tests cover the `u16` sequence wrap; they do not qualify a real
hardware rollover.

Preparation, publication, index/entry access, timeout, unexpected index/ID,
invalid length and copy failures retain the same backend and attempt. Checked
completion remains recorded separately from subsequent copy failure or unwind.
Later `fill` calls return the original typed failure before any backend method,
even if a late completion has appeared. There is no extraction, reset, rearm or
retry API. Invalid caller lengths are unstarted rejection and leave a healthy
device usable. Partial output is usable only on complete success.

The retained fallback runs no backend or nested field destructor: no reset,
unmap, close, allocator work or refund occurs there. Healthy fallback delegates
to the existing adapter cleanup policy; this batch does not qualify that
ordinary reset/teardown path. No new kernel cleanup receipt, custody registry,
root/account classification, quota or physical retry authority is introduced.

`ServingRng` contains device and endpoint together. On a first returned retained failure
the service records diagnostic `0xe008` plus the typed stage/attempt; it logs no
entropy bytes. Valid subsequent requests get the existing `ERR_DEVICE` without
backend access. The caller-funded CPU buffer is never a DMA target, and the
existing owning reply transaction cannot publish a partial successful count.
The signed reply fixture now drives this rejection through the shared device
owner with public-data fault callbacks.

A fenced shutdown consumes device, endpoint and original `ShutdownRequest`
into terminal retention. It drains CQ hints and parks event-only without
receiving more requests, resetting hardware, destroying mappings or publishing
device-quiesced status. The existing node device coordinator requires an
acknowledged exit before teardown and refuses power-off when a driver does not
quiesce. That control flow was inspected; a real fenced-hardware shutdown was
not injected. Forced termination, partial setup and healthy-shutdown proof
remain separate boundaries.

## Configuration and structure

Hardware policy lives in `rng_device.rs`: `Limits::DEFAULT` permits
**1000000 completion-index reads per descriptor**, and `Limits::new(n)` accepts
only a positive bound before adoption. The bundled driver selects this default.
It is an iteration ceiling, not milliseconds, a total request deadline or retry
permission. Every confirmed short completion may need another bounded step.

`QUEUE_SIZE = 8` is a compile-time power-of-two split-ring dimension. The
adapter derives page-rounded ring backing from its layout (currently two
pages); bounce storage follows the existing entropy protocol buffer. Larger
queues do not add outstanding descriptors. These changes retain the previous
poll ceiling and actual storage sizes. Rebuild/requalify layout/device capacity,
CPU cost, short completions, timeout/late completion, retention and original
caller budgets together. There is no manifest/boot-resource/live override.

The driver now contains hardware access, while the shared module owns sequence,
validation and terminal policy. The [entropy reference](../../reference/entropy.md),
[Configuration guide](../../guides/configuration.md), secret-lifetime inventory,
API ledger, cleanup family C18 and both manual chapters describe these same
boundaries. Protected bounce storage may retain entropy while DMA is uncertain;
no erasure guarantee is added or attempted beneath that uncertainty.

## Executed controls

Debug and release each passed **382 runtime tests plus one doctest**: runtime
53, services 252, reconnect example 3 and transport 74. Two existing
documentation examples remain explicitly ignored; no runtime test is ignored.
Eight new descriptor tests cover:

- positive-only limits and invalid requests without backend access;
- ordinary/short success, continued progress and normal field destruction;
- a three-poll timeout followed by a late completion, with no second backend
  call or destruction;
- preparation, partially successful publication, index/entry failure,
  unexpected index/descriptor, zero/oversized completion and postcompletion
  copy failure, retaining exact stage/context and every field;
- a confirmed prefix followed by timeout, without replay or prefix success;
- sequence wrap and unwind before publication or after checked completion.

`scripts/tests/test-rng-device-fence.py` passed three controls. It compiles the
actual owner/contract into an optimized isolated host harness. The unchanged
owner passes eight tests. Removing the retained-state guard fails the late-
completion test; enabling unconditional backend destruction fails the field-
retention test. Both regression executables fail with status 101 as expected.
These are isolated unsafe mutations, not old production binaries, and neither
is installed in a service. Exact mutations and diagnostics are in the evidence.

Both target libraries/all service binaries passed strict Clippy. Forty
AArch64 and thirty-seven x86-64 service images were rebuilt/staged and signed;
the separate AArch64 Sitas image remains unchanged. Formatting and whitespace
checks pass. Dependency inputs, RNG wire version 2 and runtime ABI are unchanged.
The advisory gate and other unchanged signing/GCM suites were not represented
as fresh scans or reruns here.

| Frozen final run, four LPs / fresh storage | Deferred pass / fail / pending | HTTP checks | Shared descriptor-control executions |
| --- | --- | --- | --- |
| AArch64 S3 + security + HTTP, SMMUv3 | 22 / 0 / 0 | 14 | 2 |
| x86-64 security + HTTP, AMD IOMMU | 18 / 0 / 0 | 14 | 2 |

Both final manifests match complete current signed-bundle maps and archived
kernel hashes. Each signed probe runs the shared production owner with
public-data fault callbacks, never installed as cryptographic RNG. It also
places real owned endpoint/memory fields inside that backend: normal Drop
signals endpoint death; retained Drop does not, while the field counters remain
unchanged. The CPU-only fixture resources are reclaimed by ordinary exact-root
namespace retirement, not an uncertain DMA cleanup. Existing caller-retention
and sponsor-close/refund controls pass afterward. Both publication generations
retain `0xffff` on both architectures.

The AArch64 Cortex-A710 has no RNDR, so the healthy real VirtIO adapter supplies
TCP/IP and S3/TLS. S3 verified the 33-byte round trip. Real RNG readiness,
32-page caller accounting and original-sponsor retirement pass twice. The x86
runner has no VirtIO RNG device; its native result qualifies the shared owner
and capability-field controls, not real x86 hardware timeout behavior.

## Preserved preliminary failure and verification limits

The preliminary AArch64 run failed **21 / 1 / 0**, bitmap `0x20000000`:

```text
panicked at crates/catten/src/self_test/el0_security.rs:117:18:
self-test deadline expired while waiting for adversarial application grant checks
```

It completed the new descriptor-control marker at guest time 44.504984, the
existing controller RPC marker at 69.714966, and reported the aggregate failure
at 73.573464. Its S3 oracle passed at 39.354478. Those observations and any HTTP
success do not turn it into a pass. A preliminary AMD run passed 18 / 0 / 0.
Both preliminary runs overlapped final descriptor-context hardening/staging;
their archived kernels, captured maps, source hashes and results are preserved,
but they are not used as final source/artifact-binding qualification.

The final cold runs followed frozen builds, with no concurrent builds, and
passed without changing the existing 45-second aggregate probe budget, inner
deadlines, retry policies or capacities. The final ARM publication checkpoints
passed at 45.391144 and 78.065234; its terminal result was 78.234124. These are
later, different-artifact results and do not establish the preliminary failure's
cause. The earlier deadline observation remains explicit. HTTP transport retry
diagnostics are retained separately alongside final passing outcomes.

The fault callbacks exercise the real software owner, not real late DMA,
IOMMU teardown or reset failures. No actual hardware timeout, fenced-driver
shutdown, setup failure or forced-termination injection occurred. The scalar
ring helpers still use existing bytewise volatile access; real-device atomicity,
cache ordering and rollover qualification remain separate. Neither checked
descriptor metadata nor a late completion proves global hardware quiescence.
No Intel/Kafka native run or formal-model change is claimed in this batch.

The manual built in two passes: 252 pages, no undefined references and the
prior 189 global overfull warnings. Contents page 14 and affected/adjacent pages
117–120 and 198–202 were rendered and visually checked; the rest was not
re-audited. The software reuse/fallback gap is corrected in this scope;
SEC-06/07/15/18 and broader hardware recovery, erasure and flood fairness remain
partial. Totals stay **29 corrected, 6 partial, 3 open**; production trust stays
disabled.
