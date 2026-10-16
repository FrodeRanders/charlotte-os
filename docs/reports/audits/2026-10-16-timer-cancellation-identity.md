# Exact owned timer-cancellation identity

Date: 2026-10-16. Baseline `573c98c1`, following capability-backed completion
preparation. Scoped C16 identity/cleanup qualification. SEC-07/18 remain partial;
the numbered findings remain 29 corrected within scope, six partial and three
open. No new registry, owner family, custody or physical completion proof.

## Source observation and choice

Anonymous timer cancellation assigned a private `u64` from wrapping
`NEXT_TIMER_EVENT_ID.fetch_add`. Eager same-LP cancellation removed queued events
with that numeric ID. A retained handle could outlive its event; after full
counter wrap it could select a later event on the same LP. Removal scanned all
matches, while diagnostics counted a single removal. This is a source-proven
edge, not evidence of actual 64-bit exhaustion or attribution of a historical
panic.

The numeric ID had no public ABI, persisted format, diagnostic consumer or
other lookup user. It duplicated the exact charged cancellation-state owner
already shared by each event and its handle. The fix removes the counter,
state field and scalar removal API, rather than adding an exhaustion mode.
No compatibility selector or second identity allocator remains.

## Ownership and cancellation contract

`TimerEventCancelHandle` and its event retain the same `CancellationRef`, an
`Arc<TimerEventCancellation, EventAdmission>`. Eager removal borrows that strong
owner and compares the queued state's exact allocation with `Arc::ptr_eq`.
The consuming handle remains alive throughout queue lookup/removal, so the
allocation cannot be freed and reused underneath the comparison. No scalar
pointer escapes, is stored as authority or is adopted back into an owner.
Matching LP, deadline or cancelled flags do not identify the same event.

Construction still prepares the existing cancellation allocation fallibly,
using its original event admission. No new allocation, charge, registry,
lookup fallback or permanent exhaustion error is introduced. Public completion
operation IDs keep their separate checked boot-lifetime counter and syscall
sentinel rule. Diagnostic count atomics are observations, not cancellation
identities, and are unchanged.

Cancellation still sets the shared flag before inspecting owner LP. A remote
or busy queue retains the flagged node and original charge until owner
reconciliation; cancellation neither waits for a remote queue nor reenters a
borrowed local queue. Publication still updates actual owner LP before insertion
and discards an already-cancelled prepared node. Local eager removal preserves
the caller's IRQ state and existing comparator reconciliation.

Original event/node/strong/weak lifetime accounting is unchanged. A stale strong
handle keeps its predecessor state alive and cannot select a successor.
Weak-only backing remains charged to its original sponsor; failed weak upgrade
does not revive it or create a cancellation handle. Node removal or a false
removal result is not physical quiescence, root release authority, accepted-work
replay permission or a new recovery owner.

## Native controls

The existing synchronous timer-event fixture runs production construction,
publication, cancellation, local event processing and original charged owners:

- Two live events have the same maximum absolute deadline and publishing LP.
  Cancelling one removes only its node, preserves the other's flag and original
  sponsor charge, and permits the other's exact cancellation.
- An unpublished predecessor loses its prepared node, retires its sponsor and
  retains a cancellation handle plus weak backing. Across **64 successor
  cycles** with identical LP/deadline, stale cancellation removes nothing and
  leaves each successor live and charged until its own handle cancels it.
- Dropping the predecessor's final strong handle leaves its weak-only charge.
  A fresh successor stays charged to its separate sponsor while final weak
  destruction refunds only the predecessor.
- A timer actually fires while its old handle remains. A successor with the
  same expired deadline/LP is published under a short local mask. The stale
  fired handle cannot remove it; local event processing delivers the second
  real callback. Both retained handles refund only their own backing afterward.
- Two events are flagged cancelled while their real local queue is borrowed.
  Both flags/LPs/deadlines then match. Eager removal through one retained alias
  removes only that event; the other remains charged until owner reconciliation.

Private aliases clone the existing typed state owner inside this serialized
kernel fixture. They never reset a production allocator or reconstruct scalar
authority. Allocation precedes the fixture's local publication mask. The fixture
checks original node accounting and exact enclosing IRQ restoration. Existing
timer fixtures additionally cover rejected node preparation, cancellation before
enqueue, simulated preparation relocation, busy-queue retention, actual
sleep/watchdog operation, and completion cancellation/observer rejection.

These are exact-object queue controls, not a simulated full-counter wrap. The
removed counter has no exhaustion state left to exercise.

## Execution results

The host runner passes **499 Rust tests, zero failed, four ignored**, plus its
signing/tooling/Python controls. Strict kernel Clippy passes on both custom
targets with `-- -D warnings`. Workspace `cargo fmt --all -- --check` and
`git diff --check` pass.

Runtime Rust and the new native fixture were frozen before two completed
security profiles. Each uses four LPs, fresh storage, a 180-second guest budget,
denied Rust warnings, assembly-section checks and automatic all-LP failure
capture. Both return runner exit zero and diagnostic result `complete`:

| Profile | Exact-cancellation fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, security | Passed at 1.176082 s | 19 passed, zero failed/pending at 56.971618 s |
| Intel VT-d, security | Passed at 1.068690 s | 17 passed, zero failed/pending at 64.044155 s |

Together these execute 128 stale unpublished-predecessor requests during
successor churn, two fired-predecessor/real successor-notification controls,
and both exact live/busy-queue removal and original weak-only charge controls.
Each profile also completes 64 scheduled IPC close/exit episodes with exact
exit watches and two independent scoped EL0 security generations. Concurrent
cancellation traffic retires after 14,888 requests on ARM and 13,924 on Intel.
Those are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 still qualifies
only never-dispatched migration.

No failed native sample, panic or Raft persistence fail-stop was observed in
these runs. This focused cross-architecture matrix does not repeat the preceding
AMD/no-network profiles or establish historical failure causes.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CATTEN_ALLOW_SANDBOX_NETWORK=1 CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-identity-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-identity-intel-security
```

HTTP/deployment ports are 19080/19081 for ARM and 19180/19181 for Intel. Staged
service inventories match the baseline byte for byte; no service sources or
bundles change.

[Version-controlled evidence](evidence/2026-10-16-timer-cancellation-identity.json)
records reviewed Rust hashes, commands, kernel/boot-image/initial-disk/final-UART
hashes, concise observations and complete unchanged service inventories. These
inline results and the JSON are the published evidence; temporary logs and
ignored diagnostic directories are not required to read it. Per-file hashes
include the new untracked fixture omitted by dirty-diff manifests.
Documentation/evidence was finalized afterward. This is not a full-tree or
physical-platform attestation.

## Remaining boundary

This closes the private cancellation-ID wrap gap recorded by the preceding
batch. The scheduler/completion timer contracts, simplification ledger and LaTeX
completion chapter describe the current single implementation.

Queue-held node/observer destruction, arbitrary enclosing guards/IRQ contexts,
registry/backlog metadata, aggregate admission, integrated pressure/progress and
physical hardware qualification remain separate. No polling, forced IRQ
enabling, readiness source, root lease, physical release or retry policy changes.
Historical IPC exit, stack, persistence and NIC incidents remain unattributed.
