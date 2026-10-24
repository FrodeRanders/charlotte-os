# Security audit: reply-token storage

Date: 2026-10-24. Reviewed baseline: `b1cb1731`.

Reply-token nodes now prepare fallibly outside IPC in the existing call owner,
after the pending node and with both original counts already reserved. Reply,
cancellation and serialized close retain detached token nodes until post-IPC
disposal. Payload and storage are freed before the original token count returns.

The [tracked evidence](evidence/2026-10-24-security-ipc-reply-storage.json)
records source/formal/artifact hashes and selected execution diagnostics. This
continues SEC-07 R7-2 and C07/C08/C09/C10/C18 owner/context work. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production
trust remains disabled.

## Preparation and publication

Previously call submission inserted a reply token into an allocating `BTreeMap`
after shared authority publication. Its count charge lived in the token payload
and returned during token removal under IPC, without a per-record storage owner.

The existing `PreparedCall` now retains both prepared nodes and their original
reservations. Invalid handle/rights/queue checks precede count/node admission;
revalidation still precedes mutation. The caller's exact-root lease protects the
unlocked preparation interval. Both counts reserve atomically before either
allocation. Failure of the second allocation leaves the pending prefix inside
the same owner for explicit cancellation after local guards. Abandonment retains
all partial fields and the original lease. No compatibility allocation path is
kept.

Publication relinks both admitted nodes. Each node owns its count charge
independently of the payload, through actual storage disposal. These are the
existing per-generation/node/ordinary count policies; they have not been widened
or turned into a general heap-byte or principal limit.

The token keeps a copy of its original reservation classification for later
claim-source preparation. Moving the charge into its storage must not make that
source infer classification from a later caller generation or promotion. The
native control verifies the real published token remains ordinary after its
caller is promoted, and that late refund returns the original ordinary share.

## Reply and cleanup

Ordinary scalar reply returns an internal `ReplyCompletion` owning the detached
token and notification batch together. Explicit finish after IPC unlock disposes
the token before notifying. Its inert fallback retains both, including observer
entries and their sponsorship. No callbacks or allocator work run on abandonment.

Claimed reply and cancellation keep detached nodes in their existing root-owning
operations until post-IPC release. Cancellation also keeps its detached pending
node there. Node disposal precedes root completion. Failed loans leave their
live token, original charge, claim fence and uncertain backing in place; a
rejected cleanup cannot publish a terminal reply or refund its token count.

Serialized call/endpoint/namespace cleanup reuses the detached nodes themselves
as one bounded retirement list. It neither allocates a teardown snapshot nor
adds a registry. The existing close operation releases that list after IPC
unlock; fallback retains the complete chain. Cursor scans remain ordered and
bounded by the existing record limits.

Producer-LP claim-source closure/delivery, returned-resource observation and
pending-call/CQ semantics are unchanged. Source/waiter allocations retain their
separate sponsorship. Metadata release supplies no hardware completion or
physical replay authority. These are the existing cleanup families; the
[rows](../../reference/cleanup-recovery.md),
[record contract](../../reference/ipc-record-budgets.md),
[lock order](../../reference/locking.md) and Markdown/LaTeX manual are updated.
Userspace code, protocols and runtime ABI are unchanged.

## Verification

Serialized native controls exercise:

- **64 second-allocation rejections**, each after a real pending-node prefix and
  both count reservations. The allocation boundary leaves IPC/lifecycle/table
  guards available; root close reports `OperationsInFlight`. Ordinary rejection
  restores count/authority baselines without publishing a call.
- **64 paired publications/detaches** with IPC and heap held. Already-admitted
  pending and reply nodes relink into local registries, then form one 64-token
  retirement batch without allocating. Disposing the pending nodes returns
  their count while every token count remains charged. Post-guard token disposal
  returns the original baseline; the call owners then finish, preserving IRQ
  state and releasing their authority/root leases.
- An actual self-call prepares ordinary counts, then its caller is promoted.
  The published token retains its original classification. Reply and call close
  restore the original total/ordinary counts independently.
- Retired-list fallback and scalar-completion fallback each retain one scalar
  token node/count under IPC/heap guards. The scalar completion also retains a
  nonempty notification batch and its one waiter entry, without invoking its
  callback or refunding that entry. These fixtures have no loans or hardware
  authority. Confirmed caller-root retirement and exact slot reuse leave the
  two original record charges attached to the old account, without crediting
  the successor. The deliberate footprint is two token nodes/counts and one
  notification entry, in addition to the earlier containing-call fixtures.
- Existing invalid-handle and first-allocation controls remain enabled, as do
  real quota, queued/delivered cancellation, failed loan/publication, returned
  memory/connection, source/epoch readiness and scheduled endpoint/namespace
  retirement checks.

Both first native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**. Each completes
two scoped-security generations and all **14 host HTTP controls**. Healthy
responses beside stalled peers take **0.297–0.387 s** on ARM and
**0.220–0.375 s** on x86. Trickled deployment bodies reject in **30.14 s** and
**30.30 s**; cancellation traffic retires after **19,424** and **18,576** requests.

Each host capture records four `ConnectionResetError` attempts around EOF,
abortive-close and idle recovery controls. Their existing bounded retries
succeed; failed attempts and adjacent successes remain separate in the evidence.
No cause is attributed and no timeout/counter was widened. Earlier initial-probe
timeouts and guest failures retain their own historical evidence; these runs do
not erase them.

All **16 shared collection host tests** pass, including late-bound admission,
actual payload/node destruction before refund, admitted-map detachment and
retaining list fallback. Strict base-kernel Clippy passes on both architectures.
Workspace formatting, whitespace and shell syntax checks pass. Embedded service
bundles are reused and the tracked manual PDF is preserved.

## Formal qualification

All **20 focused cases** pass: three safe configurations and 17 expected
counterexamples. The checked safe graphs are:

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |

Required safe actions execute and each unsafe case violates its named invariant
through the intended action. No model/configuration changed. These models
project shared authority/delivery, rollback, non-terminal timeout and exact-root
cleanup gates. The two family-node allocations, post-guard disposal, captured
classification copy and token count's actual node lifetime are native/host
qualification, not new model variables. The prior complete **190-case** run
remains the separate full-suite checkpoint; all its 235 inputs are unchanged.
This gives no executable refinement, allocator/IRQ or sustained-pressure fairness
proof.

## Remaining work

Endpoint and per-capability maps still allocate under IPC, including insertions
after authority mutation. Removing the reply-token insertion does not yet make
IPC publication fully fallible. Other waiter/vector/control-block factories and
payload destructors, broader byte/principal admission, allocator overhead and
enclosing IRQ/guard contexts remain SEC-07/18 work.

Pending/token lookup and insertion are linear; serialized cursor/close scans can
repeat that traversal. Record limits bound size, but these controls do not
qualify worst-case hold time, contention or sustained management/timer/teardown
fairness. The earlier queued-loan mapping panic and root-close timeouts remain
causally unresolved; no cleanup or replay authority follows from these passing
runs.
