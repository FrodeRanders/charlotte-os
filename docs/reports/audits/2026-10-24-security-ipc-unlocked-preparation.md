# Security audit: unlocked call and endpoint preparation

Date: 2026-10-24. Reviewed baseline: `1f333a60`.

Call and endpoint creation now prepare shared-authority metadata and capability
payload storage outside IPC while their existing exact-root owners remain live.
Call preparation also constructs its pending waiter list outside that guard.
Shared count admission still precedes payload/list allocation, with namespace
revalidation under IPC. Ordinary unused metadata disposes after guards, root last.

The [tracked evidence](evidence/2026-10-24-security-ipc-unlocked-preparation.json)
contains source/formal/artifact hashes and selected execution results. This
continues SEC-07 R7-2 and the existing C18 owner boundary. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production
trust remains disabled.

## Captured preparation

The preceding capability-storage batch removed infallible map insertion after
authority mutation. Its shared-authority and payload factories still ran under
IPC. Call staging also allocated its pending waiter list there.

`PreparingCapability` now owns the captured namespace identity and the existing
`capability::PreparedReservation` alongside its admitted reservation and payload
node. Call and endpoint parents retain their existing exact root across three
phases:

1. Prepare shared-authority metadata outside IPC. This owns fallible storage,
   with no admitted capability identity yet.
2. Reacquire IPC, check the captured namespace and retirement fence, and reserve
   the shared count/identity by relinking prepared authority metadata.
3. Leave IPC before capability payload allocation. Calls then prepare their
   pending waiter list, using original call-record classification. Final
   publication revalidates the containing operation again.

Shared quota rejection precedes payload and call waiter-list allocation. Family
record/slot reservations still precede their own allocations. Earlier private
metadata may therefore exist when shared count admission rejects; it stays in
the same complete owner for post-guard disposal. Existing count limits and byte
qualification boundaries have not changed.

Call staging now revalidates completed metadata before attachment work. Endpoint
publication borrows its already-admitted reservation and relinks prepared
endpoint and capability nodes. No authority is published during the unlocked
factories. A retired or substituted namespace rejects; no fallback rebuilds it
from the numeric ASID.

Ordinary failure or success explicitly finishes unused payload, admitted
authority and unused shared metadata after IPC unlock, releasing the existing
root last. In particular, a rejected shared reservation can retain its unused
node and provisional charge until this finish. Abandonment retains every new
field without allocator, counter, registry or callback work. Kernel roots stay
immortal; synthetic boot fixtures retain their explicit namespace path.

This adds no lifecycle-under-IPC edge, IRQ enable, root/pin mechanism, registry,
recovery custody or physical retry. Borrowed loans, hidden delivery, returned
resource observation and exact-generation cleanup proofs remain unchanged. The
[cleanup row](../../reference/cleanup-recovery.md),
[capability contract](../../reference/ipc-capability-storage.md),
[record contract](../../reference/ipc-record-budgets.md) and Markdown/LaTeX manual
record this scope. Userspace code, protocols and runtime ABI are unchanged.

The lock table also corrects an older description of failed revocation: uncertain
cleanup retains its Revoking/pin/backing fence; only unstarted receipts cancel.
This documents the existing implementation and grants no retry authority.

## Remaining allocation contexts

Direct/delegated connection grants, receiver reply authority and returned
connections still call the guarded adapter to these same preparation helpers.
Their factories remain under IPC, with their early retirement rejection
preserved before those factories. Moving their work outside needs their own
containing-owner and revalidation qualification; this batch adds no unlocked
interval to those paths.

The joint memory/authority helper, `commit_transfers_with_delivery`, also still
allocates its publication vector under IPC, including scalar-only calls. The
heap-held call test below covers staging revalidation, not the entire commit.
The endpoint test does execute complete endpoint publication with heap held.
Other vector/control/list preparation, field destructors and enclosing
IRQ/guard contexts remain open, alongside byte/principal admission and
allocator overhead.

All IPC maps use admitted storage with linear lookup/insertion. Reference and
close scans can repeat traversals. Count bounds and these serialized controls do
not qualify worst-case hold time, contention or sustained management/teardown
progress under hostile pressure.

## Verification

Native serialized controls exercise:

- **320 call-stage rejections** across five boundaries: before shared metadata,
  after that metadata, after shared admission, after private payload allocation
  and after pending waiter-list construction. Every boundary observes
  IPC/lifecycle/table/heap availability and root close rejecting with
  `OperationsInFlight`. Shared and family count baselines reconcile after
  ordinary whole-owner finish.
- **64 shared-authority factory rejections** through the existing actual record
  factory rejection hook, after real family-record prefixes. No authority or
  family count escapes; all root leases finish ordinarily.
- **64 heap-held call staging revalidations** after complete unlocked
  preparation, with original IRQ state preserved. No factory runs in staging.
- Actual shared namespace saturation rejects before private payload and pending
  waiter-list factories. Retirement after metadata preparation rejects captured
  admission; the unused prefix returns normally. These use existing accounts
  without rewriting live limits or sponsors.
- Endpoint controls expand from six to ten boundaries: **640 preparation-stage
  rejections**, alongside the existing **96 growth rejections**. New boundaries
  cover shared metadata preparation, admitted identity and private payload. The
  same exact root remains live throughout each factory.
- **64 complete endpoint publications with heap held**, including shared authority
  publication and both payload installations. Explicit preparation finish and
  endpoint close restore original counters and IRQ state. A separate retirement
  interleaving between shared preparation and admission rejects without a
  published endpoint.
- Existing heap-held local relinking/detachment, FIFO growth replacement, source
  classification, hidden final-reference cleanup, guarded abandonment, shared
  quota, transfer/mapping failure, loan cancellation, returned resources and
  scheduled endpoint/namespace retirement controls remain enabled.

Stage controls inject errors around real admitted allocations; they do not
exhaust the global allocator or prove arbitrary concurrent schedules. No new
abandoned fixtures are added. The existing unpublished-call and endpoint
abandonment controls now also retain their newly prepared authority/payloads:
two additional staged capability pairs and one pending waiter-list allocation
across those existing roots. Earlier deliberately retained records, roots,
backing and charges remain accounted for; no budget reset conceals them.

Both final-source native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**. Each completes
two scoped-security generations and all **14 host HTTP controls**. Healthy
responses beside stalled peers take **0.257–0.377 s** on ARM and
**0.332–0.416 s** on x86. Trickled deployment bodies reject in **30.20 s** and
**30.61 s**; cancellation traffic retires after **20,432** and **15,340** requests.

Each host capture records four `ConnectionResetError` attempts around EOF,
abortive-close and idle recovery, plus an initial metrics-request `TimeoutError`. Existing bounded retries succeed; failures and adjacent
successes remain separate in the evidence. Their causes are not isolated and
no timeout or count ceiling was widened. Earlier native failures retain their
own checkpoints; these passing runs do not retrospectively explain them.

The earlier first images also passed. Final review preserved the guarded
adapter's original early retirement check before its metadata factory, in
addition to captured reservation revalidation. Both architectures were rerun
for that final source. Earlier passing images and their exact source/artifact
hashes remain separate in the evidence; there were no native failures in this
batch. No other production code changed between those images and the final runs.

All **16 shared collection host tests** pass. Strict base-kernel Clippy passes
on both architectures, as do workspace formatting, whitespace, shell syntax and
local reference checks. Native assembly-section checks stay enabled. Bundled
services are reused and the tracked manual PDF is preserved. Runtime sources
are unchanged after the native builds; later edits are documentation/evidence.

## Formal qualification

All **20 focused cases** pass: three safe configurations and 17 expected
counterexamples. Required safe actions execute and unsafe cases violate their
named invariants through the intended actions.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |

No model/configuration changed. Joint authority/delivery, rollback, non-terminal
timeout and active exact-root gates retain their existing projections. The
three implementation phases, actual allocator/guard boundaries, per-family
metadata and original-class lifetimes are native/host qualification. The prior
full **190-case** run remains a separate checkpoint; all its **235 inputs** are
unchanged. These results provide no executable refinement, allocator/IRQ or
sustained-pressure fairness proof.
