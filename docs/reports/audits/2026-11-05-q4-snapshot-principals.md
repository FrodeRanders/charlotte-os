# Q4 snapshot-family principal admission

Baseline `e4738466`. This batch funds the remaining production requested-byte
paths in `snapshot_buffers` from the original signed principal: primitive
syscall input/plaintext and CPU/copy/DMA frame snapshots. Diagnostics already
used that counter. **Q4 remains in progress** for wider metadata/principal
coverage, allocator/control overhead and coherent joint heap capacity. The
[qualification evidence](evidence/2026-11-05-q4-snapshot-principals.json) records
source/artifact identities, controls and preserved preflight diagnostics.

## Boundary and resource tradeoff

All three snapshot consumers share the existing boot-lifetime principal-history
record and its **16 MiB kernel-buffer requested-byte ceiling**, also used by root
heap/image frame catalogs. There is no new registry, pool, eager backing or limit
change. Snapshot family ceilings remain 16/12 MiB total/ordinary and 8192/6144
frame buffers. These are independent limits, not a joint heap guarantee.
Per-principal frame-count shares and fairness are not implemented by this byte
ceiling; the family count can reject earlier. Inline parent fields, allocator
excess and unrelated metadata require their own accounting.

One capture helper checks the exact root generation and borrows trusted class
and verified principal under the same root-table hold. Existing operations keep
their root ownership; the captured token grants no root or teardown authority.
Signed construction prebinds the account. Unidentified kernel/raw fixture roots
retain family admission; application input cannot select that exception.
Promotion, later workers, root close, sibling instances and ASID reuse cannot
replace an allocation's original sponsor or original ordinary charge.

Primitive vectors use the existing sponsor-aware byte allocator: principal,
family, then Global allocation, with separate leaf holds. Failure returns only
the unused prefix; complete growth overlap stays funded. The private input
owners, checked source-copy boundary, public failure statuses and consumed input
capability rules stay in force. Plaintext still allocates once and wipes full
capacity before free and both refunds. No parser, signature or launch contract
changes.

Fixed frame snapshots keep their existing typed retaining owner and Global
vector. A reusable linear `ByteCharge` pairs original-principal funding with the
existing family byte/count reservation. Both precede allocation. Preparation
rejects an allocator adapter returning nonempty or excess-capacity storage before
publication. Explicit finish frees the vector before either refund; guarded
abandonment retains allocation and both counters without taking locks. Empty
escrow pins allocate nothing and retain their existing independent source fence.

This shares funding mechanics without replacing CPU, copy or DMA completion
proofs. Ordinary unpin frees snapshot metadata before the independent backing
pin. A metadata refund does not authorize physical release, hardware replay or
retry. Consumed failures do not gain recovery custody.

The existing maxima remain supported: actual private storage for two maximum
pickup/plaintext pairs plus 3 MiB mixed allowance peaks at **11,815,160 bytes**,
within both ordinary family bytes and the principal byte ceiling in isolation.
This allows sharing, not guaranteed concurrency with every other buffer. Raising
a limit would increase possible retention and require the joint capacity and
latency review in the [capacity baseline](2026-11-05-q4-capacity-baseline.md).
No evidence in this batch requires such an increase or reducing legal inputs.

## Controls and qualification

The host control checks linear fixed-byte funding, movement without duplicate
reservation, principal denial, family-rejection prefix rollback and zero-byte
handling. Existing real allocator controls still trace growth and actual free.

Serialized native controls use actual principal-bound roots and history records.
They enter at the trusted constructor boundary; they do not themselves verify
artifact signatures. The separate security campaign verifies signed launch.
Private fixture principals use 16-byte frame and 96-byte input ceilings (95 bytes
for descriptor-prefix rejection, zero for public-handler denial); replacing a
fixture limit preserves every live charge. The simultaneous maximum-input case
uses the unchanged default ceiling. These controls do not claim real concurrent
EL0 saturation or fairness. Existing actual family byte/count pressure controls
remain unchanged and use unidentified fixture roots to isolate those dimensions.
Controls cover:

- Sibling-root input denial, shared diagnostic/frame rejection, healthy independent
  sponsors, promotion without reclassification and descriptor-prefix rollback.
- All four affected public handlers on invalid content, family denial and principal
  denial, retaining existing input consumption, status and zero-receipt rules.
- Successful copy and unused DMA pins, repeated copy/DMA/map rejection before pin
  publication, 64 funded allocator rejections, and malformed capacity rollback.
- Retained copy-pin metadata across root close and same-slot generation reuse,
  followed by refund to the old principal; independent healthy charges survive.
- Actual buffer-free-before-both-refunds observation with accounting locks free.
  Guarded complete snapshot abandonment retains one 8-byte metadata buffer and
  its family count/principal charge, including after root close. That deliberate
  retention is separate from cases asserting exact baseline recovery. No active
  DMA mapping or hardware completion is fabricated by this metadata-only case.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting and full host suite | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19, unchanged 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each, unchanged 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed, unchanged 90-second budgets |

All five campaigns use the exact signed bundle identities qualified by the
preceding frame-catalog batch. No userspace source changed. Frozen source/input
hashes remained unchanged through final qualification. These results qualify
the named byte owners and controls; they accept no aggregate security gate or
production/physical-platform isolation claim.

## Preserved failures and limits

The first ARM/x86 compile preflights rejected a test-only `PAddr::from(usize)`;
the fixture now uses the checked `PAddr::try_from` conversion. The diagnostics
remain separate from corrected-source qualification. Host negative controls
that intentionally panic remain distinguished from a failing test result.

The ARM display second-grant-probe, Intel retirement-pin and ARM exit-watch
incidents remain unresolved; headless passes do not explain or close them. The
previous frame-catalog AMD reuse-fixture failure and phase correction remain in
[their original record](2026-11-05-q4-frame-tracking.md). General caller/IRQ/implicit
Drop review remains Q5; sustained management/teardown progress remains Q7/Q8.
Finding dispositions, aggregate gates and production trust are unchanged.
