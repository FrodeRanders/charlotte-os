# Cross-category cleanup and recovery review

Date: 2026-10-08. Source baseline: `ec8a5383`.

## Purpose and result

The previous remediations introduced typed owners category by category. This
review establishes their shared lifecycle and a finite coverage map before
extending recovery custody. The new living
[cleanup/recovery strategy](../../reference/cleanup-recovery.md) records 18
owner families, current continuation boundaries, checked entry contexts, seven
cross-category gaps and an ordered implementation plan.

This is documentation and source review. No kernel behavior, wire format,
recovery permission or production security gate changes. **SEC-07 and SEC-18
remain partial; the ledger stays 20 corrected, six partial and four open.**
The full production call-chain/destructor inventory and common controller are
not declared complete.

## Findings from the source review

The common strategy is exact linear ownership, fenced admission, unlocked
completion work and explicit proof before reuse/refund. Domain teardown composes
device, IPC and memory completion before sealing cleanup admission and detaching
the final root. Recovery therefore belongs to an owned operation; a cancellation
with several loans cannot be decomposed into independently retried scalar IDs.

Existing continuation contracts differ materially:

- Staged close returns its owner on Pending; final-root invalidation can transfer
  its complete owner into bounded custody. A kernel-range owner can retry its
  failed barrier before physical release starts.
- Several loan/mapping/namespace completion APIs consume a failed receipt and
  retain fences/pins/counts without returning complete retry ownership. This is
  safe retention, not eligibility for a generic retry controller.
- Ordinary explicit failure and abandonment are different. Direct loan and IPC
  error paths can explicitly release leases after retaining failed backing;
  abandoning their owners retains counts/claims. The inventory records those
  distinctions instead of treating all errors as one recovery state.
- Supervisor teardown caches terminal errors. Successful final-root retry does
  not reconcile that cached result or authorize restart/reassignment.

The review also identifies explicit R18-1 work, rather than assuming every
owning destructor satisfies the target policy:

- `Stacks::drop` drives user/kernel physical cleanup; thread/context destruction
  reaches it through reaping and preparation failures. ARM reaping occurs before
  the enclosing yield path restores incoming IRQ state.
- CPU table/user/stack preparation and IOMMU unpublished/region rollback
  destructors can release physical backing. `PreparingDomain` and
  `PreparedDmaDomain` can invoke whole-domain/backend teardown. These need
  complete outer-context and implicit-field qualification.
- `RetiredKernelRange::drop` still emits an early log even though it retains
  backing; that diagnostic context is part of the inventory.
- All three IOMMU `destroy_domain` implementations perform maintenance and
  physical table release inside `with_unit`/`with_smmu` serialization. Moving
  namespace finish outside lifecycle/device guards did not remove these inner
  guards. Domain/source/command ownership must precede backend phase separation.
- Transfer/source rollback and IPC queue/record cleanup can acquire registries
  or destroy metadata. Existing logical quotas and successful scoped tests do
  not prove global heap bounds or universally safe destruction context.

These observations refine the existing partial findings. They are not new
numbered findings, newly reproduced corruption, or claims that all those paths
currently deadlock. G1–G4 name the required proof/change and regression evidence.

## Shared machinery decision

The common layer should own bounded admission, nonwrapping identity, exclusive
attempt claims, terminal states, diagnostics and policy. Typed adapters retain
their actual payloads and category-specific completion evidence. No universal
destructive Drop, object-ID adoption or unconditional retry interface is proposed.

A second complete eligible owner must demonstrate reuse before extracting the
root registry's custody primitives. Borrowed namespace receipts require an owned
containing transaction; they cannot enter static storage by erasing their
lifetimes. Kernel ranges are a candidate only with their enclosing stack/admission
dependencies intact and context qualification completed.

Trusted local policy and supervisor reconciliation precede any external mutation
API. Read-only `SystemObserver` diagnostics must not become recovery authority.
External operator control depends on SEC-08/10 authenticated identity/transport,
freshness, scoped target/action authorization and bounded submission/outcomes.
There is no force-clear or partial-physical-release reclamation fallback.

## Validation and limits

Implementation anchors and continuation/Drop paths were read for all 18 families,
including the three IOMMU destroy wrappers and ARM/x86 reaper entry paths. Relative
documentation/source links and coverage/gap references were checked: 144 relative
links/anchors, 18 unique family rows, seven gap gates and 38 receipt declarations
matched their implementation anchors. The three backend anchors were checked and
`git diff --check` passed. No Rust files changed, so no new host or QEMU execution
was needed; the [preceding root-recovery report](2026-10-08-security-root-recovery.md)
retains its runtime evidence without being presented as tests of this design.

This review does not exhaustively enumerate every production caller, allocation
or field destructor, prove pressure fairness, exercise active outstanding device
I/O, implement operator recovery, or qualify physical hardware. Those remain
explicit acceptance gates rather than implicit exclusions. Future changes must
update the living map and evidence, so adding an instance cannot extend the work
list indefinitely.
