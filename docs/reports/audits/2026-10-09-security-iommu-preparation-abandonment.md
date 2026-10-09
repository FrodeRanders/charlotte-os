# IOMMU table and region preparation abandonment

This continues the [published stack-pair correction](2026-10-09-security-published-stack-retirement.md)
within C15/G1/G3 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md).
The ledger remains **20 corrected, six partial and four open**. This is scoped
SEC-18 progress, not completion of the finding or its QEMU milestone.

## Correction and ownership

`Tables` fallback previously released private table frames and their admission.
`PreparingRegion` fallback released its provisional extent and entered the pool.
Even eliminating those calls would have left implicit `Vec<Region>` destruction
entering the global heap allocator under unknown outer guards.

Both fallbacks now retain their original backing/admission without physical or
heap allocation release, pool/backend/table guards, callbacks or logging. Table
fallback retains its ledger storage; explicit successful release disposes that
storage. Ordinary success therefore does not leak the ledger. Reservation-only
region abandonment also retains admission and marks the exclusively borrowed
parent uncertain. That parent rejects further allocation and release/refund.
Scope, domain/unit ceiling and original pool classification are not re-resolved
through an ASID, requester or numeric domain ID.

Ordinary metadata and physical allocation rejection explicitly cancel unused
region admission. Allocated-region cancellation consumes its extent and arms the
parent fence before invoking physical adapters; rejection or interruption keeps
the whole original charge. Successful cancellation clears only its own fence
and refunds after physical completion. Table cancellation is consuming and
rejects a published owner before physical work.

`Tables::prepare_unpublished` retains a private constructor prefix through
preparation and explicitly cancels it on ordinary rejection. VT-d, AMD-Vi and
SMMUv3 constructors use that boundary for roots and unit tables/queues. SMMUv3
includes its context descriptor. VT-d and SMMUv3 explicitly cancel a rejected
private MSI prefix; VT-d also cancels its private domain when context-table
admission fails before domain publication. Private walker fixtures explicitly
cancel their owners. A known-private firmware-disable rejection can cancel the
new backing because no new base register has been written.

Publication rejects uncertain preparation. Published domain detachment, hardware
maintenance/drain, timeout handling and requester/reset fences retain their
existing typed backend contracts. No physical partial-release retry, numeric
backing adoption, generic custody registry or force-clear operation is introduced.
Unpublished rollback failure and abandonment are terminal retention, not a
recoverable receipt. Published-unit initialization failure likewise has no new
recovery adapter.

## Evidence

Serialized boot probes exercise both domain and unit scope:

- Metadata rejection occurs before physical allocation and restores unused
  admission. Physical allocation rejection, reservation-only cancellation and
  allocated contiguous-region cancellation restore their baselines.
- The shared constructor helper rejects after prefixes of one, two and three
  actual frames; each confirmed private cancellation restores frame/charge counts.
- Unpublished table abandonment and rejected published cancellation retain real
  ledger allocations while heap, physical allocator and original pool are held.
- Region abandonment covers reservation-only, allocated backing, armed transfer
  before ledger insertion and partial physical release. Partial release invokes
  two adapters, returns the first frame and rejects the second; no retry occurs.
- Every guarded probe also holds backend registries, lifecycle and both CPU
  table guards. On x86 both compiled backend registries are held without hardware
  initialization; Arm holds its SMMU registry. Fallback changes no free counts or
  charges. Uncertain/frozen parents reject allocation and repeated release before
  physical callbacks; successor owners do not discharge earlier admission.

The additional probes retain **14 original charges (seven domain), ten frames**
and any admitted ledger storage. The original published-abandonment/partial-
release probes retain five charges and three frames, giving nineteen charges and
thirteen frames across injected retention fixtures, separate from live hardware
backing. Interruption is modeled; no actual panic unwinding, physical allocator
corruption or withheld hardware acknowledgement is exercised.

Existing sparse-walker ceilings/cached-prefix reuse, map rollback pins, NVMe
domain pressure/reset, exact root ownership, real user isolation and thread
lifecycle fixtures remain enabled on all three QEMU targets.

## Reproduced retirement timeout

The initial Intel run passed the IOMMU admission/guarded abandonment probes but
failed user-isolation cleanup after its null-read fault: ASID 90, root generation
6 remained leased until the existing ten-second deadline. It finished **14
passed, one failed**. Executed kernel SHA-256 was
`2ce7eb74fbef2fe18a1ea014bd81fb315040d31e12a4fb3235d1d1b6a0e29c2b`.

Selected failure output (guest timestamps are seconds since boot):

```text
[+     1.657945] FATAL USER FAULT: ASID=90 vector=14 error=0x4 RIP=VAddr(0x20002) address=VAddr(0x0)
self-test deadline expired while waiting for user stack retirement lease
[+    11.666268] SELFTEST COMPLETE: passed=14 failed=1 pending=0 passed_bitmap=0x1bbfe failed_bitmap=0x1 pending_bitmap=0x0
```

Aggregate user-retirement observations
`[started,released,identity,detach,invalidation,physical]` changed from
`[187,185,0,0,1,1]` to `[194,192,0,0,1,1]`: seven starts/releases without additional recorded user-half
rejection. This does not correlate the individual pair or prove kernel/admission
completion. It reproduces the earlier progress concern; no scheduling or
IOMMU-causation conclusion follows from this evidence.

The retained thread node now records its reported pair outcome. A copied snapshot
of the exact staged generation includes its captured root, LP, started fence and
error, with kernel validation/detach/physical/unconfirmed errors distinguished.
The failure-node fixture checks those observations while preserving the same
owner. Timeout output prints after staging serialization leaves. An in-flight
batch can be absent, so a missing snapshot is never a completion proof.

Six independent atomic x86 observations count wrapper shootdown success, busy,
masked, exhausted, delivery rejection and timeout. They carry no operation
identity, exclude fake rendezvous helpers and cannot authorize retry. Neither
these diagnostics nor fresh-run success clear the original lease, extend its
deadline, restore abandoned backing or resolve the cause.

## Validation

| Check | Result |
| --- | --- |
| Complete host harness | Passed, including 29 slot/lease probes, four retirement-list allocation/identity tests and 13 signer tests. |
| Both custom-target kernel Clippy checks, `--locked -- -D warnings` | Passed. |
| Initial Intel VT-d execution | IOMMU probes passed; user-stack retirement timeout, 14 passed/one failed. |
| Intel VT-d with retained-pair diagnostics, two fresh executions | Each 15 passed, zero failed/pending. |
| AMD-Vi, fresh storage | 15 passed, zero failed/pending. |
| Arm SMMUv3 security suite, fresh storage | 19 passed, zero failed/pending; probe `0xffff`, publication generations 1/2 and 4,872 cancellation requests retired. |
| Formatting, whitespace and documentation | `cargo fmt --all -- --check`, `git diff --check`, 141 relative links/anchors and unchanged 18-family/seven-gate map passed. |

Runtime executions were sequential after host/Clippy work. Every run passed the
IOMMU preparation abandonment marker, including the initially failed Intel run.
Assembly permissions verified 249 x86 native entries and one Arm trampoline.
The two diagnostic Intel runs and AMD/Arm did not reproduce the timeout. These
successes validate the scoped preparation probes, not resolution of the retained
stack lease or completion of the concurrent recovery gate.

Commands (unchanged userspace/ABI; existing signed staged bundles reused):

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance iommu-preparation-intel-diagnostics-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance iommu-preparation-intel-confirm-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance iommu-preparation-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18579 CATTEN_DEPLOY_HOST_PORT=17879 scripts/run-aarch64.sh --security-test --instance iommu-preparation-arm-20261009 --fresh-storage --timeout 180
```

Executed final diagnostic kernel SHA-256:

- x86: `38ccfa6d04c32970d829137cd1d3ef8217359d9b9767083d9484b11392dd515b`.
- Arm security: `c981e88e81d1c27ca664a62efda8466c0de99d11b42f0af62e4a3c19869e25b4`.

All compile/Clippy attempts passed in this batch. The only failed runtime is the
preserved initial Intel timeout above. No forced count clear, deadline extension,
frame adoption or retirement retry was added to obtain passing runs.

## Remaining boundaries

Ordinary allocation/cancellation and successful ledger disposal still run under
backend serialization. Published maintenance/drain and physical table release
also retain that guard. G3 must retain a complete domain/command completion owner
and requester fence before releasing serialization; releasing an outer device
lock alone does not satisfy it. Command queues, completion epochs, concurrent
map/unmap/create/reset, failure reinsertion and terminal physical release need
separate execution evidence.

General registry/metadata destruction, outer IRQ/caller qualification, shared
custody, supervisor reconciliation and the combined pressure/recovery matrix
remain open. The earlier Intel user-stack timeout remains unexplained; its
original deadline, diagnostics and retained owners are unchanged. QEMU evidence
cannot satisfy physical-platform qualification.
