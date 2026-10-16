# Timer callback delivery after local queue borrow exit

Date: 2026-10-16. Baseline `f5d5ee27`, following timer queue disposal.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gap and captured ownership

The preceding batch moved complete timer-node disposal outside the LP queue
borrow. `TimerEvent::signal` still invoked callbacks and notified waiters beneath
that borrow, including destruction of temporary observer references and detached
waiter entries. A callback could therefore encounter its own busy queue. These
are source findings; no historical panic, lock cycle, IPC exit, stack,
persistence or NIC incident is attributed to them.

The queue now captures a finite prefix against one monotonic cutoff. It detaches
complete due nodes and purged cancelled nodes without allocation, destruction,
notification or a new charge. Anonymous nodes are relinked into deadline order,
preserving stable FIFO at equal deadlines and the inline quantum's priority on
a tie. Diagnostic fired counts record expiry selection, not successful
notification or physical completion.

`PendingDelivery` retains both complete chains, their original event/node/source
charges and the owner LP. It cannot be sent or shared between threads. Runtime
checks additionally require the captured LP and the caller's preserved IRQ mask
before and after callback execution. The queue's inline claim prevents a nested
drain from selecting another prefix; nested processing defers to that owner.
All production IRQ and idle callers use the same checked local driver.

After the queue borrow leaves, the owner disposes purged nodes and delivers the
selected prefix. Each event checks its retained cancellation flag immediately
before starting its signal. Temporary callback and waiter references, detached
waiter entries and complete event nodes can then be destroyed outside the local
queue borrow. A callback may enqueue or cancel another event. Work queued during
delivery, or becoming due after the cutoff, remains for the next pass. The
comparator is reconciled both before handback and after delivery against the
current queue, including callback mutations and elapsed time.

Cancellation still matches exact retained allocation identity. A selected event
is absent from the queue, so cancellation returns false while setting its flag;
that can suppress a later signal in the captured prefix. False also includes
unpublished, busy, remote and already-fired cases, and is not a completion proof.
Cancellation cannot undo a signal that has already begun.

Abandonment retains every remaining selected and purged field and leaves the
claim fenced. No fallback clears the claim, frees a sibling field, retries the
operation or grants physical cleanup authority. Ordinary purge is unstarted
disposal; selection/delivery is terminal logical retirement. Abandonment is
terminal retention, not Pending or an owner-preserving retry interface.

Callbacks still run synchronously on their original masked LP. They must remain
short and nonblocking. Removing the queue borrow does not qualify unrelated
enclosing scheduler/producer guards, arbitrary callback work, unpublished
preparation or final cancellation-handle destruction. No worker, polling,
forced IRQ enabling, wire-format change or userspace ABI change is added.

## Controls and retained fixture cost

Controls execute synchronously before AP scheduling with private fixture
sponsors prepared before event capture. Heap-held selection checks are not
permission to deliver beneath an enclosing heap guard.

- Capture 64 actual due anonymous nodes under held heap and real queue guards;
  no callback or waiter runs there. Delivery preserves deadline order. A waiter
  on event 1 runs immediately after its callback. Callback 16 enqueues an already
  prepared immediate event and attempts a nested drain; that work runs only on
  the next pass. Callback 32 cancels selected event 48 and a queued future event:
  the former returns false and suppresses its callback/waiter, the latter returns
  true. All original charges persist through their actual owners and return to
  baseline after explicit cleanup.
- In two episodes, including a nested IRQ-mask hold, notify callback and waiter
  observers whose final strong references naturally disappear during delivery.
  All four notifications and four final observer destructors verify the original
  LP, masked IRQs, and availability of both the actual timer queue and heap lock.
- A private comparator fixture checks equal-deadline order explicitly:
  inline quantum 99 precedes anonymous events 10 and 11, followed by later event
  12. The real LP comparator is restored before returning.
- Abandon a complete owner containing one selected and one purged event beneath
  heap and real queue holds. Its private claim remains fenced; another prepare
  cannot steal it. Both original charges and sources remain retained. Independent
  waiter cancellation releases its own entries only. No actual LP delivery
  claim is abandoned or recovered.

The new negative control retains **two event charges and two empty admitted
source-list allocations**, with zero persistent waiter-entry charges, until
guest shutdown. Earlier timer disposal retains one event/source; completion
metadata retains its separate ten records, four timer events, four CQ payload
charges, two watch charges and three authority charges. These related negative
controls now retain seven event charges in the shared timer pool. They run
after full-pool baseline checks, never reset counters or replace a live sponsor,
and establish no user-root, retry or physical-release authority.

Existing exact/stale/weak cancellation identity, 1,024-node reclamation,
admission failures, scheduled sleep/watchdog, completion cancellation,
returned-resource and complete queue-disposal controls continue to execute.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Timer delivery fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.229072 s | 17 passed, zero failed/pending at 11.336644 s |
| Intel VT-d, no network | Passed at 1.056896 s | 15 passed, zero failed/pending at 14.434082 s |
| AMD-Vi, no network | Passed at 1.622405 s | 15 passed, zero failed/pending at 20.091266 s |
| ARM / SMMUv3, security | Passed at 1.041793 s | 19 passed, zero failed/pending at 55.939896 s |
| Intel VT-d, security | Passed at 1.053428 s | 17 passed, zero failed/pending at 61.787375 s |

Together these select **320 actual due anonymous nodes under held heap and
queue guards**, then deliver outside the queue, including 315 selected callbacks
(the other five are cancelled before signal), ordered waiter notification,
selected/queued cancellation, nested-drain fencing and next-pass enqueue.
They also execute 20 final observer destruction controls, inline-quantum tie
ordering, and guarded complete-owner abandonment with the private claim fenced.
Each profile also completes 64 scheduled IPC close/exit episodes with exact exit watches,
320 total. Both security profiles complete two scoped EL0 generations. These
are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration. No failed native sample, panic, deadline or
persistence fail-stop was observed. Historical failed samples remain separately
preserved in the [backlog refund report](2026-10-16-completion-backlog-refunds.md);
these passes do not establish their causes.

Commands share `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance timer-delivery-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance timer-delivery-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance timer-delivery-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-delivery-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-delivery-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel; evidence records actual network forwards.
All staged service files and run inventories match the preceding baseline byte
for byte; service/runtime libraries were not edited or rebuilt.

[Version-controlled evidence](evidence/2026-10-16-timer-callback-delivery.json)
records reviewed hashes for all eight Rust files including the new delivery owner and fixture,
commands, artifact/UART hashes, unchanged service inventories and concise
observations. Frozen Rust hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

Broader timer callback/disposal IRQ and enclosing caller contexts, unpublished
preparation and final cancellation-handle destruction remain unqualified.
Aggregate heap/principal admission and integrated pressure/progress remain open.
Historical IPC exact-worker exit, stack, persistence and NIC causes remain
unattributed. This batch does not change the audit finding dispositions.
