# Security audit: IPC capability payload storage

Date: 2026-10-24. Reviewed baseline: `2cfa25d6`.

IPC capability payloads now prepare fallibly before authority mutation and
publish by relinking prepared nodes. This removes the infallible per-capability
`BTreeMap` insertion from endpoint, call, direct/delegated connection, returned
connection and receiver reply-capability publication. Removal keeps detached
payload and shared authority metadata together through disposal outside IPC.

The [tracked evidence](evidence/2026-10-24-security-ipc-capability-storage.json)
records source/formal/artifact hashes, failed runs and selected execution
observations. This advances SEC-07 R7-2 and C07/C08/C09/C10/C18. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production
trust remains disabled.

## Publication boundary

Previously each path reserved shared authority, published it, then inserted a
payload into an allocating `BTreeMap`. Exhaustion at that last insertion could
panic after authority or memory mutation. Its connection-count charge lived in
the payload and returned on removal under IPC, before a detached node lifetime
could be represented.

`PreparingCapability` now keeps authority and payload storage in one inert owner.
Shared count admission precedes fallible payload-node allocation. Both remain
owned when allocation rejects after authority admission. Parent endpoint, call,
connection, receive and reply operations keep partial preparation through guard
exit; ordinary finish explicitly disposes it there. Existing root-owning parents
finish their roots last. Abandonment retains all new fields and original charges
without allocator, lock or callback work.

Memory and authority still publish jointly under IPC. Payload installation then
checks live target authority and only relinks its prepared node. Queued and
unobserved returned connections remain hidden until dequeue or first result
observation. Direct grants become visible in that same IPC hold. Readiness alone
observes no returned resources.

This changes allocation failure ordering, not every allocator context. Payload
and shared-authority factories **still run under IPC**, before mutation. Shared
count limits have not changed, and private payload bytes/allocator overhead have
no separate comprehensive byte or principal admission. Other vector/list/control
factories and outer masks/guards remain separate work.

## Retirement and containing owners

Capability removal detaches both its IPC payload node and shared `RetiredRecord`
authority under the same IPC hold. The authority receipt moves into the detached
payload. Existing temporary cleanup lists reuse these nodes without allocating a
snapshot or adding a registry. Public/serialized close, queued and unobserved
result cancellation, endpoint close and namespace drain all carry the pair.

Explicit disposal leaves IPC, frees shared authority metadata, then frees the
payload/node before returning the original connection-count charge. Fallback
retains both metadata nodes and their original accounts. Root retirement and
ASID reuse cannot redirect a late refund to a successor.

Receive retains preparation and speculative retirement in one containing owner.
Result-page failure detaches only its speculative reply capability; queue, token,
loans and result bytes stay intact. Scalar and vector receive share this path;
one-way receive needs no reply capability. Ordinary disposal leaves IPC.

Scalar reply keeps returned-connection preparation, detached capability metadata
and notifications with its existing completion owner. Claimed replies keep them
in their existing root-owning operation, explicitly disposing them before roots
finish. Ordinary failure refunds unused grants after IPC unlock; abandonment
retains their preparation with the reply claim. The existing unstarted
returned-memory escrow fallback remains separate and unchanged.

Failed loans still retain their claims and uncertain backing. Logical absence,
wakes, timeout and metadata detachment grant no physical release or replay
permission. No new root/pin mechanism, recovery custody, registry or family is
introduced. The [cleanup rows](../../reference/cleanup-recovery.md),
[storage contract](../../reference/ipc-capability-storage.md),
[locking contract](../../reference/locking.md) and Markdown/LaTeX manual reflect
this boundary. Userspace protocols, code and runtime ABI are unchanged.

## Verification

Serialized native controls exercise:

- **128 stage rejections**, before payload allocation and after a real node
  prefix. Shared admission remains in the owner until explicit post-guard
  finish; counts return to baseline only then. Allocation boundaries leave
  heap/lifecycle/table guards available, while the enclosing IPC guard remains
  held as the documented limitation.
- **64 heap-held joint publications/detaches**, with prepared authority and
  payload storage. Hidden connection lookup rejects before delivery. Both shared
  and connection counts remain charged after logical removal and preparation
  finish; only post-guard metadata disposal restores their baselines. IRQ state
  is preserved.
- Failure after a real containing-call prefix and shared capability reservation.
  Exact-root close rejects with `OperationsInFlight`; explicit whole-owner finish
  returns call/reply and capability counts. Retirement after full capability
  preparation rejects publication without inserting a payload.
- Guarded abandonment of a complete staged call and a detached connection
  payload/authority pair. The added deliberate footprint is **one root/lease,
  two call/reply records and one staged capability payload/authority**, plus
  **one detached connection payload/authority pair and its connection count**.
  The detached pair retains original charges through confirmed root retirement
  and exact slot reuse. These new controls carry no loans or hardware authority.
- Existing quota, scalar/vector receive result-page rejection, copy/move/loan
  submission failure, returned memory/connection, claim/source readiness and
  scheduled endpoint/namespace retirement checks remain enabled. These include
  public cleanup paths beyond the local publication controls.

Stage rejection surrounds real admitted allocations; it does not exhaust the
global allocator. Count saturation fixtures retain their prior scope.

The first run on each architecture reached the same deterministic boot-fixture
assertion at `ipc/reply/tests.rs:331`: shared capability count **4**, old expected
count **3**. The new storage and earlier quota/delivery/returned-resource controls
had passed. That abandonment fixture still expected automatic refund of the
unpublished returned connection. The new inert owner deliberately keeps that
preparation with its abandoned reply claim and roots.

The fixture now requires the additional shared and connection counts, captures
the destination and confirms it has no live authority. Production code is
unchanged between those failed runs and repeats; both failures, kernel/artifact
hashes, diagnostics and the original fixture hash remain separate in the
tracked evidence. The pre-existing returned-memory escrow fallback, root/loan
fences and caller backing checks remain enabled.

Both second native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**. Each
completes two scoped-security generations and all **14 host HTTP controls**.
Healthy replies beside stalled peers take **0.302–0.387 s** on ARM and
**0.360–0.431 s** on x86. Trickled deployment bodies reject in **30.11 s** and
**30.14 s**; cancellation traffic retires after **20,180** and **18,092** requests.

Each host capture records four `ConnectionResetError` attempts around EOF,
abortive-close and idle recovery. ARM also records an initial metrics-request
`TimeoutError`. Existing bounded retries succeed. Failed host attempts and
adjacent successful controls remain separate; their causes are not isolated.
No timeout or count ceiling was widened.

All **16 shared collection host tests** pass, including admitted-map detachment,
actual payload/node disposal before charge refund and retaining list fallback.
Strict base-kernel Clippy, formatting, whitespace, shell syntax and local
reference checks are recorded with the final captures. Bundled services are
reused, native assembly-section checks remain enabled and the tracked manual
PDF is preserved.

## Formal qualification

All **20 focused cases** pass: three safe configurations and 17 expected
counterexamples. Required safe actions execute and unsafe controls violate their
named invariants through the intended actions.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |

No model/configuration changed. These models retain their joint authority and
hidden delivery, rollback, non-terminal timeout and exact-root projections.
Private IPC payload allocation, shared-count prefixes, paired metadata disposal,
field-drop contexts and linear scans remain native/host qualification. The prior
full **190-case** checkpoint remains separate; all **235 inputs** are unchanged.
There is no executable refinement, allocator/IRQ or sustained-pressure fairness
proof.

## Remaining work

All IPC registry maps now use prepared admitted nodes. Their factories and wider
containing operations still need guard/IRQ qualification. Payload and shared
authority factories under IPC, other control/vector/list allocation and weak-only
retention remain open, as do aggregate byte/principal admission and allocator
bookkeeping.

Lookup and sorted insertion are linear; endpoint reference checks and close
scans can repeat those traversals. Count bounds alone do not qualify worst-case
hold time, contention, sustained pressure fairness or management progress.
Historical queued-loan mapping panics, root-close timeouts and initial HTTP
probe failures remain at their own evidence checkpoints. The fixture diagnosis
here does not attribute or close those earlier failures.
