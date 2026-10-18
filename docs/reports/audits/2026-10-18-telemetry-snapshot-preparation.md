# Telemetry snapshot preparation and status-page lifetime

Date: 2026-10-18. Baseline `b31a5b3f`, after the documentation sweep commit.
This continues SEC-07 R7-1/R7-2 and SEC-18 C02/C18/G1/G4. The numbered finding
dispositions remain **29 corrected in scope, six partial and three open**.

## Source-confirmed gaps

`THREAD_STATISTICS` previously collected thread rows under `MASTER_THREAD_TABLE`
and domain rows under `DOMAIN_USAGE` with infallible vector growth. It then
allocated an output object before infallibly allocating its encoding vector.
Source count and arithmetic had no explicit request scratch limit. The local
capture paths therefore could not establish graceful rejection under actual
allocator exhaustion. This review did not execute a real heap-exhaustion exploit.

Heap sampling copied a physical status-frame address out of `DOMAIN_USAGE`,
released that guard and read the frame without retaining its owning root.
System snapshots also reduced captured domain identity to a reusable numeric
ASID before the later heap lookup. Root retirement/reuse could therefore race
that physical read or mix a predecessor's usage row with a successor's heap
sample. This is a source-confirmed lifetime/identity gap, not a reproduced
concurrent use-after-free or attribution for a historical crash.

## Correction

The shared `snapshot::Budget` admits at most **1 MiB of vector scratch per
request**, across thread capture, domain capture and encoded output. Checked
size arithmetic precedes `try_reserve_exact`; actual capacity consumes the
budget. Each source is counted under its local guard, then storage prepares
outside that guard. Capture fills a fixed logical capacity without growing it.
If the source outgrew preparation, the entire request rejects. A shorter source
produces a complete smaller section. There is no clamped prefix or retry loop.

Encoding scratch is also prepared fallibly before output-object allocation.
Only complete encoding reaches the existing memory-object allocation/write
boundary. Rejection returns the existing `(0, 0)` unavailable result. A failed
write closes its unreturned output through that boundary; no new scalar cleanup
ladder or physical-release mechanism is introduced. Kernel diagnostic callers
now retain a typed snapshot error rather than an infallible vector.

Domain rows retain their captured `AddressSpaceHandle`. Heap sampling admits
that exact generation's `AddressSpaceOperation` outside source guards, then
verifies the registered frame against the fixed status mapping while holding
the address-space table. The bounded volatile read exposes no pointer, and
ordinary completion explicitly releases its operation. Stale, closing, absent
or mismatched backing becomes unavailable heap data. It never falls back to a
successor resolved by scalar ASID.

Final root sampling borrows the existing `ClosingAddressSpace` before metadata
detachment. It uses the same generation/mapping checks while lifecycle remains
held; sealed close does not reopen ordinary admission. This composes C02's
existing root owner and C18's private plain-value scratch. It adds no owner
family, custody registry, retry receipt, public capability or wire-format change.
Thread-statistics version 8 and application authorization remain unchanged.

Sources: [bounded scratch](../../../crates/catten/src/klib/collections/snapshot.rs),
[syscall encoder](../../../crates/catten/src/syscall/statistics.rs),
[thread capture](../../../crates/catten/src/cpu/scheduler/threads/mod.rs),
[domain capture/read](../../../crates/catten/src/memory/usage.rs),
[closing-root sampling](../../../crates/catten/src/memory/mod.rs).
The [living contract](../../reference/telemetry-snapshots.md) and
[cleanup/recovery rows](../../reference/cleanup-recovery.md) describe the boundary.

## Controls and limits

Five host tests exercise pre-allocation byte/arithmetic rejection, allocation
failure without budget mutation, growth without reallocation or prefix overwrite,
one shared budget across buffers, and smaller/empty source capture.

Each native boot runs 64 repetitions of four syscall controls: reject before
thread preparation, after thread capture, before encoding preparation, and after
output allocation instead of writing it. All 256 return `(0, 0)` and preserve
the caller's capability count and free physical-frame count. These are explicit
rejection adapters, not real global-allocator or hardware faults. Separate
heap-held capture controls exercise production thread/domain fills and reject
an underprepared domain destination without allocating. Checked length overflow
also rejects. The existing ordinary/invalid-observer/system-observer snapshot
controls now execute on both architectures.

A signed, unstarted NS image provides real loader-owned runtime pages for heap
read controls. The fixture checks a valid record with the allocator held, a
wrong registered frame, closing-root admission, borrowed closing sampling,
completed retirement, ASID reuse with a new generation and missing status data.
Both roots close; no physical root or operation is intentionally abandoned.
The fixture clears its synthetic peak before retirement so it does not seed
the real NS principal's adaptive heap history.

The per-request scratch limit excludes the separately admitted output object's
physical backing and all unconverted downstream metadata. It is not aggregate
heap/principal admission or a guarantee of essential-service progress. Source
scans and allocator hold-time behavior under hostile concurrent pressure remain
unqualified. The thread/domain sections and workload-published counter words
are independent observations, not an atomic or authenticated machine-wide
resource ledger. General outer syscall IRQ/guard contexts, destructor inventory,
abandoned-operation recovery and physical-platform validation remain open.

## Validation and evidence

Host validation passed **517 Rust tests, zero failures and four ignored tests**
across 59 summaries, plus the signing/tooling controls and cluster-sign self-test.
Five new tests cover the shared scratch primitive. Strict Clippy passed on both
custom kernel targets; formatting and changed-file whitespace checks passed.
The AArch64 and x86 service bundles were rebuilt through their helpers. Their
41- and 37-image inventories stayed identical throughout the native series.

All guests use four LPs, fresh storage and a 180-second fixture deadline:

| Source phase | Profile | Deferred pass/fail/pending | Completion, guest seconds |
| --- | --- | --- | --- |
| Initial controls | AArch64 / SMMUv3 | 17 / 0 / 0 | 13.611274 |
| Initial controls | x86 / Intel VT-d | 15 / 0 / 0 | 18.722619 |
| Initial controls | x86 / AMD-Vi | 15 / 0 / 0 | 14.212363 |
| Initial controls | AArch64 / SMMUv3 HTTP | 19 / 0 / 0 | 15.789835 |
| Initial controls | x86 / Intel VT-d HTTP | 17 / 0 / 0 | 18.412262 |
| Final fixture isolation | AArch64 / SMMUv3 | 17 / 0 / 0 | 12.132098 |
| Final fixture isolation | x86 / Intel VT-d | 15 / 0 / 0 | 15.652730 |

All seven runners exited zero with diagnostic result `complete`; no native guest
failed. Across them, 1,792 injected syscall rejections preserved capability and
frame counts. The first five runs preceded the fixture-only peak reset described
above. Production Rust was unchanged; the final two runs recheck both architectures
after that fixture-isolation refinement. The AMD and HTTP profiles were not
repeated after it. The earlier untracked fixture source was not separately
frozen, so its executed artifacts are identified by kernel/manifest hashes
rather than claimed as final-source runs.

Both HTTP profiles passed metrics, EOF/abortive/idle-peer recovery, deployment
status, malformed/oversized framing, invalid-body and trickled-body controls.
Each printed **five `ConnectionResetError` retry lines** during recovery probes
before passing; the exact lines and successful controls are retained in the
evidence. Their cause is not attributed to this change, and passing retries do
not resolve the historical HTTP/reset or fairness observations. The trickled
body checks rejected in 30.09 seconds on Arm and 30.18 seconds on Intel.

The version-controlled [evidence](evidence/2026-10-18-telemetry-snapshot-preparation.json)
contains the ten final Rust hashes, commands, unchanged tooling and bundle
inventories, seven kernel/boot/disk/manifest/UART hashes, concise execution markers
and HTTP diagnostics. It does not depend on temporary logs or ignored artifacts
remaining available. Historical intermittent IPC, storage, NIC and shutdown
incidents remain separate; no causal closure, production trust qualification,
real backing-pressure matrix or new numbered disposition follows from this batch.

Concurrent manual heading and PDF edits appeared during this work. They were
preserved and are outside the audit source/evidence scope.
