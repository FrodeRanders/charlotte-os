# Complete timer cancellation handback

Date: 2026-10-16. Baseline `ebbf3ee`, following unpublished timer ownership.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gap and scope

Timer cancellation already used exact retained allocation identity. Removed
events left the local queue through complete retirement owners, and completed
unpublished nodes had inert abandonment. The cancellation handle still owned
an ordinary Arc field. Its final release could free cancellation backing and
refund original admission on an implicit drop in an unknown context.

`cancel_event` also returned only a Boolean and disposed its handle before
returning, including the busy-queue and remote branches. For a stale handle
whose event had already disappeared, busy-queue cancellation could therefore
free its final allocation beneath the caller's still-live outer queue borrow.
The function's internal guard exit did not qualify that containing borrow.
These are source findings, not attribution of any historical panic, IPC exit,
stack, persistence or NIC incident.

## Exact reference and complete result ownership

`TimerEventCancelHandle` now retains its exact reference behind `ManuallyDrop`
and is marked must-use. Abandonment neither flags cancellation nor destroys
backing or refunds admission. Explicit consuming `release(self)` releases only
that reference after local guards; it does not cancel a queued/selected event,
complete its producer or grant replay authority.

`cancel_event` consumes the handle into `CancelledTimer` before flag publication.
This complete handback retains the handle, any removed event/node/source and
the queue-removal observation. Queue mutation still matches the exact retained
state, reconciles the current comparator and updates diagnostics. No allocation,
new admission or registry is introduced. Busy and remote branches return the
same owner; no final reference is released inside the cancellation transaction.
Its local mask preserves the incoming IRQ policy and leaves before return.

`removed()` observes only actual queue removal. `finish(self)` explicitly
disposes detached payloads and then releases the original handle reference,
returning that same Boolean. False includes stale, unpublished, selected,
busy-queue and remote cases. Shared cancellation can suppress a signal that has
not begun, but cannot undo an already-started signal. Neither Boolean value
establishes completion, quiescence, physical release or permission to replay.
Final weak cancellation references can still retain original admission after
both node disposal and final strong release.

Both bare-handle and complete-handback abandonment retain their remaining
fields without cleanup. A removed-node handback does not relink its event or
reconstruct cancellation from numeric metadata. This is terminal retention,
not Pending, recovery custody or an owner-preserving retry interface. An
abandoned bare handle does not invent cancellation; its queued event may still
fire normally. Its original reference remains charged afterward.

| Caller | Handback/release boundary |
| --- | --- |
| Checked scheduler timed wait | Accepted watchdog cancellation finishes after yield with no local queue/scheduler guard. Observer rejection, elapsed preparation budget and waiter rejection explicitly release the unused handle after private-node disposal; local setup masks leave first where present. |
| Completion timer cancellation owner | Its existing cancellation policy consumes and finishes the new result. Prior post-registry/inner terminal and record-disposal handbacks remain in place. This does not make arbitrary containing completion destruction inert or qualify all outer contexts. |
| Busy-queue fixtures | Retain the returned owner while checking flag/removal state, release the enclosing queue borrow, then finish explicitly. No hidden final-reference destruction under that borrow. |
| Callback fixtures | Move handles out of their local storage mutexes before cancellation/finish; the active delivery owner retains its original due prefix and recursion fence. |
| Watchdog node-construction rejection | Explicitly release the still-private handle after the rejected node construction returns. No submission or wait is replayed. |

Raw event and partial-construction destructors, raw cancellation Arc/Weak
backing, arbitrary enclosing guards/masks, first-use waiter preparation and
broader IRQ contexts remain separate qualification work. Explicit finish/release
requires a qualified caller; naming it does not remove other guards. No
scheduled cleanup worker, deferred registry, force-enabled IRQ, userspace ABI
or wire-format change is added.

## Controls and retained fixture cost

Synchronous pre-AP controls use private sponsors prepared before event capture.
They never replace live accounts, reset counters or grant user-root authority.

- Cancel 64 real queued watchdogs while the heap is held, retaining all complete
  handbacks. All 64 original event and waiter charges remain live. After heap
  exit, explicit finish checks the actual queue and heap are available before
  node/source destruction and final handle release. No callback runs. One
  weak-only allocation stays charged until its final weak release.
- Dispose an unpublished node while retaining its sole cancellation handle.
  Cancel that stale final reference with both heap and queue held. The result
  reports no removal and retains its original charge until post-guard finish.
- Exercise the remote-owner branch with a private prepared event and a synthetic
  owner-LP mismatch. Finish releases only the handle; the flagged private node
  remains charged until its own explicit disposal. This is branch qualification,
  not real cross-LP stress or a physical completion observation.
- Abandon a bare handle under heap and queue holds. Its flag remains clear and
  its already-queued event delivers exactly once. The original cancellation
  allocation remains charged after event disposal; no cancellation is invented.
- Abandon a complete successful cancellation handback under the same holds.
  Its removed node, source, callback reference, handle and original charge remain
  retained. Independent token cancellation releases only its waiter entry.

The new negative controls retain **two original event charges** until shutdown:
one for cancellation backing only, and one for the complete removed event/node,
cancellation backing and one empty admitted source-list allocation. Persistent
waiter-entry charges are zero. No real LP delivery claim or user root is
abandoned. Earlier unpublished/delivery/queue-disposal controls retain four
event charges; completion metadata retains its separate ten records, four
timer events, four CQ payload charges, two watch charges and three authority
charges. These related negative controls now retain ten event charges in the
shared timer pool. They run after full-pool baseline reservation checks;
ordinary controls independently restore their baseline.

Exact/stale cancellation identity, queued/selected cancellation, quantum ties,
callback/waiter and final observer destruction, source/admission rejection,
node/producer allocation failure, namespace reuse, successful sleep/watchdog
cleanup, nested IRQ/forced-preemption and exact scheduled IPC exit controls
continue to execute.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Cancellation handback fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 2.031596 s | 17 passed, zero failed/pending at 12.296528 s |
| Intel VT-d, no network | Passed at 1.052224 s | 15 passed, zero failed/pending at 13.987009 s |
| AMD-Vi, no network | Passed at 1.077160 s | 15 passed, zero failed/pending at 15.210041 s |
| ARM / SMMUv3, security | Passed at 1.052846 s | 19 passed, zero failed/pending at 56.306980 s |
| Intel VT-d, security | Passed at 1.067620 s | 17 passed, zero failed/pending at 61.502307 s |

Together these retain **320 complete real cancellation handbacks beneath a held
heap**, then explicitly dispose their nodes/sources and final handle references
outside the local queue/heap holds. Each profile checks a stale final reference
under simultaneous heap/queue holds, a synthetic remote-owner branch, weak-only
original charges and guarded bare-handle/complete-handback abandonment.
Existing unpublished-node disposal, post-queue delivery, nested drains,
observer destruction and exact cancellation controls also pass.
Each profile also completes 64 scheduled IPC close/exit episodes with exact exit
watches, 320 total. Both security profiles complete two scoped EL0 generations. These
are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration. No failed native sample, panic, deadline or
persistence fail-stop was observed.
The preceding batch's two introduced temporary-guard failures remain separately
preserved in its [unpublished ownership report](2026-10-16-timer-unpublished-ownership.md).
Older failed samples remain preserved in the [backlog refund report](2026-10-16-completion-backlog-refunds.md);
these passes do not establish their causes.

Commands share `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance timer-cancellation-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance timer-cancellation-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance timer-cancellation-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-cancellation-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-cancellation-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel; evidence records actual network forwards.
All staged service files and run inventories match the preceding baseline byte
for byte; service/runtime libraries were not edited or rebuilt.

[Version-controlled evidence](evidence/2026-10-16-timer-cancellation-handback.json)
records reviewed hashes for all eleven Rust files including the new cancellation
owner and fixture, commands, artifact/UART hashes, unchanged service inventories
and concise
observations. Frozen Rust hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

Raw event/partial construction, raw cancellation Arc/Weak finalization, first-use
waiter preparation and broader timer callback/disposal or enclosing caller IRQ/
guard contexts remain open. Completion's containing destructor policy remains
separate. Aggregate heap/principal admission and integrated pressure/progress
remain open. Historical IPC exact-worker exit, stack, persistence and NIC causes
remain unattributed. Finding dispositions are unchanged.
