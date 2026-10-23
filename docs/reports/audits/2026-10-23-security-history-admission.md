# Security audit: bounded principal-history storage

Date: 2026-10-23. Reviewed baseline: `fcb5df09`.

Published principal-history records and unpublished root candidates now share
**1 MiB / 4,096 nodes** of requested allocation admission. Ordinary preparation
is limited to **768 KiB / 3,072 nodes**. Previously these fallibly prepared
allocations could grow without a family byte/count ceiling, including records
retained for the boot lifetime.

This closes that SEC-07 R7-2/R7-3 storage gap. It does not finish general kernel
heap admission or protected-service fairness. The ledger remains **29 corrected /
6 partial / 3 open**; SEC-07 and SEC-18 remain partial and production trust remains
disabled. The [tracked evidence](evidence/2026-10-23-security-history-admission.json)
contains source, model and artifact hashes with selected results.

## Ownership and policy

`PreparingHistory` reserves the exact charged-node layout before its fallible
allocation. Signed launch carries its existing trusted classification into
pre-root preparation after signature verification. Root registration captures
classification from the unpublished root's translation account before moving
that root into the registration owner. Names, roles, artifact classes and
manifest fields supply no reserve claim.

The shared `AdmittedMap` now accepts the retirement list's typed charge parameter;
default callers retain their existing uncharged layout. A new principal publishes
the same candidate node in the existing table. A duplicate candidate stays in its
root's authority slot. Both keep their original allocation classification;
record reuse or later promotion cannot refund ordinary admission.

Published records stay charged for the boot lifetime, preserving aggregate
sponsors and stack/heap history through domain exit and restart. Losing
candidates follow their containing closing/final-root receipt through rejected
invalidation. Explicit unused cancellation or confirmed final metadata disposal
deallocates the node before its charge returns. Candidate/table abandonment
retains the complete allocation and original charge without locks or allocator
work. The charge never re-resolves a reusable ASID.

These are the existing C02/C03 contexts and owners. No registry, root/pin,
recovery family, custody, physical proof or retry owner is added. Counter admission
leaves its independent leaf guard before allocation/publication. History lookup,
update and node relinking do not enter that counter. Ordinary cancellation and
final disposal retain their existing post-guard requirements. See the
[policy](../../reference/principal-history-admission.md),
[cleanup rows](../../reference/cleanup-recovery.md) and
[lock order](../../reference/locking.md).

The finite pool may eventually reject new preparation after historical principals
have exited. No published record is evicted, reset or transferred to free space.
Candidates consume the same allowance, so existing principals have no guaranteed
restart slot at exhaustion. Other metadata/family limits can reject before the
history ceiling; reserved headroom does not guarantee a complete platform launch.

## Native and host verification

Both first combined native runs pass: AArch64 **20 / 0 / 0**, x86 **18 / 0 / 0**.
Each completes both scoped security generations and all **14 host HTTP controls**.
Healthy replies beside stalled peers take **0.239–0.387 s** on ARM and
**0.224–0.428 s** on x86. No deadline or existing allowance was widened and no
guest repeat was needed.

Serialized controls use real candidate nodes and actual root registration/close:

- **64 allocation rejections** restore original admission. Publication and
  duplicate reuse run with heap/admission guards held and allocate or refund
  nothing. Cancelling a trusted duplicate leaves the original ordinary record
  and its charge unchanged.
- ARM admits **3,047 ordinary + 1,020 reserved** fixture candidates; x86 admits
  **3,039 + 1,020**. Existing occupancy differs by architecture. Final ordinary
  and total counters reach their actual **3,072 / 4,096-node** ceilings.
- At ordinary exhaustion, **64 further candidates reject before their allocator**.
  Ordinary root registration returns `DomainMetadataAllocationFailed` without
  publishing an ASID or retaining new physical frames. A root from the trusted
  architecture constructor still registers and closes from reserved headroom.
- At total exhaustion, even trusted candidate allocation rejects. An existing
  root still closes using its current metadata; the returned admission permits another
  ordinary root. Disposing the fixture nodes restores the complete starting
  byte/count reservations.
- A detached real root retains its duplicate candidate and charge after rejected
  final invalidation. Confirmed release returns exactly that candidate's original
  bytes/count, with an intervening root still charged and captured ASID reuse
  checks preserved.
- Abandonment of an unused candidate and a published private fixture table runs
  under history, admission, lifecycle/root, heap and frame-allocator guards.
  Their two nodes remain charged without frame return. They are not recovered
  through fixture-only adoption or counter resets.

All **15 production collection host tests** pass. The additional charged-map
case traces actual payload disposal and node deallocation before charge refund,
while relinking/detachment allocate nothing. Existing interruption, list/entry
abandonment and default-map controls remain enabled. Strict base-kernel Clippy
passes on both targets; workspace formatting, shell syntax and whitespace checks
pass. No userspace source changed; embedded service bundles were reused. The
checked-in manual PDF is preserved, with Markdown/LaTeX contracts updated together.

## Focused formal verification

`CharlottePrincipalHistory` models three nodes, shared total/ordinary ceilings,
new-record/duplicate decisions, attachment, logical exit, confirmed disposal and
abandonment. Two candidates can resolve to one principal with different captured
classes. Its safe graph has **2,539 distinct / 5,410 generated states**, depth
**19**, with all eight required actions exercised.

Six expected counterexamples bypass ordinary admission, refund logical exit,
refund abandonment, reclassify a charge, evict published history or release an
attached node before confirmation. All **seven focused cases** pass through:

```sh
docs/tla/check.sh /path/to/tla2tools.jar CharlottePrincipalHistory
```

The harness now accepts an optional module selector; unknown modules reject
without running configurations. CI's unfiltered invocation is unchanged. The
registered suite now contains **47 safe / 143 expected-negative configurations
across 44 modules: 190 cases and 235 inputs**. The expanded full suite was **not
rerun** in this batch; the preceding complete 183-case result remains historical
evidence. No full-suite result is inferred from the focused checks.

This is a fixed-node count projection. Exact requested bytes, allocator free,
root invalidation/generation proofs, authority authentication, IRQ context and
pressure fairness are separate inputs or omissions. It is not an executable
refinement or a physical-completion proof.

## Remaining work

The pool covers history/candidate nodes, including their key, counters, link and
charge. Other root policy/usage/authority nodes, control blocks, vector capacity,
allocator overhead/rounding and the complete physical heap ledger remain separate
admission work. Count limits bind first in the exercised layouts. Published
principal churn, lookup/IRQ hold-time pressure and sustained concurrent EL0
management/timer/teardown fairness remain unqualified by these serialized tests.

The earlier x86 queued-loan mapping panic is still causally unresolved in the
[mapping report](2026-10-23-security-map-admission.md). These passing runs did not
reproduce it and do not establish whether it involved invalidation admission,
delivery, timing or rollback. The new storage pool supplies no retrospective
attribution or recovery authority.
