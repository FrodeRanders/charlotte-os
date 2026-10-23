# Security audit: memory-map rejection before maintenance

Date: 2026-10-23. Reviewed baseline: `6ca90dd2`.

Rejected memory-object mappings now skip cross-LP maintenance when they returned
before mapping-pin admission. Previously, a rejected `map` or `map_any` could
still consume shootdown admission; a maintenance failure could replace its
original error and retain unused scratch. The admission fix removes that work
from the pre-mutation branch. Admitted mappings and rolled-back prefixes retain
their existing invalidation requirement.

Kernel errors also preserve the actual invalidation cause per call. The
[tracked evidence](evidence/2026-10-23-security-map-admission.json) records source
and artifact identities, native controls and validation results. The ledger
remains **29 corrected / 6 partial / 3 open**. SEC-07 and SEC-18 remain partial;
production trust remains disabled.

## Why the branch is safe

The existing mapper checks capability rights, loan/copy/DMA restrictions,
existing mappings and frame-vector preparation before installing mapping
metadata. It then acquires the backing pin in the same object-registry hold,
with no ordinary rejection between insertion and pin acquisition. Table walking
starts after that hold leaves.

A returned rejection with no pin therefore changed no mapping metadata or table
in that call. The common `finish_mapping` path preserves the original error and
cancels unused scratch without requesting invalidation. A different operation's
existing mapping supplies no cleanup authority to this call.

Once a pin exists, even rejected installation may have published a prefix or
changed translation-table branches. Such a call still invalidates. Only confirmed
maintenance and any required scratch completion permit consuming its original
pin. Failed rollback preserves its installed-prefix record; failed invalidation
retains backing, scratch, metadata and the original charge. Public unmap keeps
the same one-shot retention policy.

These remain the C05 owner family under the same exact-root operation lease.
The [cleanup row](../../reference/cleanup-recovery.md),
[memory-object contract](../../reference/memory-object-retirement.md),
[scratch contract](../../reference/scratch-admission.md) and LaTeX manual are
updated. No root/pin, registry, recovery custody, retry owner, deadline increase
or physical-release permission is added. Existing guard-free invalidation and
outer-context qualification requirements remain.

## Per-call diagnosis

`MemoryObjectError::InvalidationFailed(cause)` now distinguishes invalidation
rejection from the existing mapping/rollback errors. x86 retains busy admission,
masked context, exhausted epochs, delivery failure or timeout. Timeout records
its owned epoch and a sampled first missing acknowledgement. The sample may be
empty if an ACK arrives after the completion/deadline check. It is diagnostic,
never completion, quiescence or permission to replay. AArch64 retains missing-root
rejection. If installation/detachment also failed, invalidation failure takes
precedence; it does not prove the earlier step succeeded.

The scheduled IPC fixture now prints its captured caller/server generations,
loan capability/index, write intent and actual error if mapping rejects. The
syscall still returns uncertain status **9**, without exporting the detailed
cause to userspace. Application/runtime cleanup and replay policy are unchanged.

## Verification

Serialized kernel controls exercise the same lease, scratch, mapper and
completion helpers as production:

- **256 copy-fenced denials** across fixed/scratch mapping reject before pin
  admission. Their maintenance callback must never run. Every unused scratch
  extent is reusable, with unchanged frames and original charges.
- Ordinary successful mapping invalidates once outside object/table guards,
  retains its pin during maintenance and releases it only afterward.
- A real two-page collision rolls back its installed prefix and still invalidates
  once before pin release. Its foreign leaf remains mapped and owned.
- Each architecture's invalidation errors pass through both mapping forms.
  x86 covers five causes, including an epoch-bearing timeout; ARM covers its
  missing-root cause. Real mappings, pins and charges remain retained after
  injected rejection. Close, unmap and another mapping cannot consume or replay
  the uncertain operation. No fixture re-adopts or fabricates completion for it.
- Existing scheduled x86 delivery/timeout retirement controls still run against
  actual remote IPI behavior. Their exact-root retention/recovery policy is
  unchanged; the map-pipeline error adapters above are synthetic failures.

Both first combined native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**.
Each completes two scoped security generations and all **14 host HTTP controls**.
Healthy replies beside stalled peers complete in **0.257–0.377 s** on ARM and
**0.273–0.358 s** on x86. No guest repeat or deadline change was used in this
batch. The evidence retains HTTP transport retries within the existing host
control policy.

All **31 lifecycle library tests** pass with the pinned `nightly-2026-07-27`
toolchain from outside the repository's freestanding Cargo configuration. Six
cover shootdown serialization, stale/duplicate ACKs, exhaustion and the new
owned-epoch sample. Strict base-kernel Clippy passes on both targets; workspace
formatting and whitespace checks pass. Userspace sources are unchanged and the
embedded bundles were reused. The checked-in manual PDF is preserved.

Two host command setup failures are retained separately: the first used the
unqualified host nightly and failed on its changed `AllocatorClone` requirement;
a later invocation from the repository picked up `build-std` and failed with a
duplicate `core` lang item. The pinned invocation from `/private/tmp` passes.
Neither failure led to a source workaround or an allocator-policy change.

No TLA+ model or configuration changes. The mapper's pin/scratch pre-mutation
branch is outside the current focused projections; host/native controls qualify
this batch. The preceding 183-case formal result remains historical evidence,
not a newly executed formal check or executable refinement of this path.

## Historical failure remains open

The [thread-metadata report](2026-10-23-security-thread-metadata.md) preserves
an x86 `UnmapFailed` panic while mapping a valid queued loan, followed by a
same-kernel pass. Its original return value did not retain the invalidation
cause; the capture cannot recover it retrospectively.

That admitted operation is different from the pin-free denial corrected here.
These passing runs do not establish whether its failure was maintenance
admission, delivery, acknowledgement timing or mapping rollback. They do not
establish a relationship to the preceding metadata changes. Exact typed errors
will narrow a future capture; reproduction and controlled scheduling/load
qualification remain open.

General kernel metadata/principal admission, sustained concurrent ordinary
pressure, shootdown readiness/fairness and broader destructor/IRQ contexts remain
SEC-07/18 work. The syscall's detailed-diagnostic boundary and failed-mapping
recovery ownership also remain separate; a retained pin or error code is not a
recovery transaction.
