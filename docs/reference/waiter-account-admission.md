# Generation-owned waiter account backing

Each root prepares one `WaitSponsor` account before registry publication.
Threads, entries and transient preparations retain that exact account rather
than resolving a reusable ASID. Entry admission remains separate from the
account's own shared backing.

| Policy field | Ordinary | Total |
| --- | ---: | ---: |
| `waiter_accounts` | 6144 | 8192 |

One slot admits a fixed allocation bundle: the account Arc and its private
`ChargedAllocator` holder. This counts allocation lifetimes, not requested heap
bytes or allocator overhead. Reservation precedes either allocation; holder or
payload allocation failure returns the original reservation. Root preparation
failure returns `MemorySponsorAllocationFailed`, without publishing an ASID.

The defaults allow twice the root-metadata pool's count-only capacity of 3072
ordinary or 4096 total complete roots. That leaves headroom for independently
retained accounts and concurrent preparations without reducing that existing
capacity. The remaining quarter is reserved for trusted preparation. This is
an independent ceiling, not preallocated RAM, a per-service entitlement or a
guarantee that all other resources can reach their ceilings simultaneously.
Sharing the entry pool would couple long-lived roots to active waits; a separate
fixed-bundle count keeps those lifetimes independent with one leaf counter.

Reusing the existing allocation owner costs one private holder allocation per
account and one extra machine word per account reference. Existing byte admission
covers the enclosing [root-account layout](root-metadata-admission.md), including
its separate timer owner; the 4/3 MiB ceilings still accommodate the count-only
complete-root capacity. No existing limit is raised. This avoids introducing
another custom reference-counting or weak-lifetime mechanism.

## Classification and release

Backing classification comes from the unpublished root's trusted translation
account, as for its registry metadata. A name, role or later ASID lookup cannot
select reserve. This does not grant permission to use platform waiter-entry
capacity: that still requires the existing explicit platform grant. Later
promotion changes future entry admission only, never the backing's original
classification or final refund.

Root retirement closes the account to new entries. Existing entries, thread
owners, allocator clones and weak references keep the original backing charged.
The payload and private holder must both be deallocated before refund; neither
root close nor final strong release suffices while weak backing remains. Exact
ASID reuse creates a fresh account and cannot refund the old generation.

Preparation and cancellation retain their existing ownership boundary. The
containing `PreparingAccount` keeps all partial fields inert on abandonment,
including the backing charge, under lifecycle/table/ledger/heap/pool guards.
Ordinary cancellation explicitly disposes unused fields outside those local
guards. Final reference disposal still needs its caller's qualified context;
this change does not qualify every scheduler, token or parent destructor.
The pool lock only changes counters and leaves before allocator entry.

## Qualification

The [versioned evidence](../reports/audits/evidence/2026-11-01-waiter-account-admission.json)
records real allocations to ordinary and total ceilings, heap-held denial before
allocation, trusted root publication at ordinary exhaustion, rollback of failed
root registration, unchanged backing classification after promotion, and recovery
by closing an existing root at total exhaustion. A retired account with a
counter-only waiter-entry charge and weak reference survives actual root close
and exact ASID reuse; final weak
release returns exactly its original slot without changing its successor.

The existing four partial-preparation rejection phases return every unused
account. Sixty-four heap/pool-held publications allocate and refund nothing;
actual partial-parent abandonment retains two account bundles. The host suite
also executes the shared allocation owner's failure, weak-reference, clone and
deallocation-order controls.
Account-specific native pressure rejects at admission; holder/payload allocation
errors are exercised by those shared host controls, rather than forced native
heap exhaustion.

Timer accounts have their own [backing admission](timer-account-admission.md).
Completion/CQ/watch accounts have separate [backing admission](completion-account-admission.md).
Capability/IPC/mailbox accounts, Waker/callback control blocks, other registry
metadata and aggregate principal/general-heap admission remain open. These
bounded fixtures do not establish sustained hostile-pressure fairness, physical
hardware behavior or a complete allocation/destructor inventory. SEC-07 remains
partial; existing C03/C16 and G1/G4 boundaries in
[cleanup/recovery](cleanup-recovery.md) remain applicable.
