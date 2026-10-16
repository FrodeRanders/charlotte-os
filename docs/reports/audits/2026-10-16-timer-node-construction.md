# Timer node construction with complete preparation ownership

Date: 2026-10-16. Baseline `c67cbd6`, following timer cancellation handback.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source boundary and scope

Completed unpublished nodes and cancellation handbacks already retained complete
payloads on abandonment. Node construction still passed an initialized
`Node { event, ... }` by value into its fallible allocator callback. Failure
therefore relied on argument/field destruction within that call; no inert
preparation owner covered the original event, source, callback reference and
outside admission before the Box existed.

This batch qualifies **construction of a node for an already charged event**.
Raw `TimerEvent` creation, admission-holder/cancellation allocation construction,
arbitrary callback factories and enclosing caller contexts remain separate.
These are source findings, not attribution of any historical panic, IPC exit,
stack, persistence or NIC incident.

## Empty storage and retained payload

`PreparingNode` first captures the complete original event behind `ManuallyDrop`,
then clones its existing admission holder without allocating or reserving again.
It owns any subsequently allocated `Box<MaybeUninit<Node>>` in that same
transaction. The allocation callback takes no event or node payload. Rejection
leaves the original event, exact cancellation state, source and charge with
the preparation; it cannot destroy them through an allocator argument.

After successful allocation, consuming completion writes the original event
into that private Box exactly once with `Box::write` and transfers the outside
admission into `OwnedNode`, then the existing inert `PreparedEvent`. No fallible
work, allocation, callback, queue publication or comparator change occurs in
this initialization. Original deadlines are copied unchanged. Queue enqueue
remains a separate consuming operation with its existing IRQ/publication policy.
The node uses the same layout and one Box allocation as before; no new event
reservation, global registry, public status, ABI or wire grammar is added.

Ordinary allocation rejection returns from the allocator callback before the
preparation is explicitly cancelled. Unused preparation can also explicitly
cancel successful empty backing without initializing it merely to destroy it.
Cancellation frees empty Box backing first, then destroys the event/source,
then releases the outside admission. That admission outlives both allocations'
relevant ownership transitions; retained cancellation strong/weak references
still keep the original event reservation charged afterward. This is private
metadata disposal, not queued-event cancellation or producer completion.

Abandonment before or after empty storage allocation retains **all** remaining
fields without freeing, refunding, source cancellation, callback, guard or
logging work. It is terminal retention, not Pending, a retry interface, recovery
custody or physical cleanup authority. No initialized node or queue claim is
reconstructed from retained backing. The production helper attempts allocation
once and consumes the same preparation into either explicit cancellation or
completed private ownership.

Current production callers continue to prepare outside their local completion/
scheduler publication guards. The constructor does not release a caller's
unrelated outer guard or force-enable IRQs. Its ordinary error path still
requires a qualified containing context; a fixture's held-guard phase is not
permission for arbitrary production allocation/disposal beneath such guards.

## Controls and retained fixture cost

Synchronous pre-AP fixtures prepare private sponsors before event capture and
never replace live accounts or reset shared counters.

- Prepare 64 real charged events with callback and waiter references and allocate
  their empty node storage. Under held heap and actual timer-queue guards,
  consume all preparations into completed nodes without allocation or disposal.
  All 64 original event/waiter charges and exact deadlines remain intact. After
  guard exit, explicit disposal checks heap/queue availability and releases
  sources. Final cancellation references release explicitly; one weak-only
  allocation remains charged until final weak release.
- Reject storage allocation while the outer heap and queue are held using a
  nonallocating rejection adapter. The preparation retains its exact deadline,
  event/source, callback references and admission; only explicit post-guard
  cancellation releases them.
- Reject through the ordinary constructor with the allocator callback's own
  heap/queue guards held. Those guards leave when the callback returns; the
  constructor then disposes its existing preparation. No payload was given to
  the callback, and source/event accounting confirms ordinary cleanup.
- Explicitly cancel successfully allocated but uninitialized backing. The shared
  cancellation helper checks local context before backing, event and admission
  disposal. No artificial node initialization or notification is needed.
- Abandon two complete preparations under heap/queue holds: one before storage
  allocation, one with an uninitialized Box. Both retain their event, exact
  cancellation backing, source/callback and original charge. Later independent
  waiter cancellation releases only its own entry.

The new negative controls retain **two original event charges, two empty admitted
source-list allocations and one uninitialized node Box** until shutdown, with
zero persistent waiter-entry charges. No real LP delivery claim or user root is
abandoned. Earlier timer controls retain six event charges; completion metadata
retains its separate ten records, four timer events, four CQ payload charges,
two watch charges and three authority charges. These related negative controls
now retain twelve event charges in the shared timer pool. They run after the
full-pool reservation checks, and ordinary controls restore their baseline
separately. Counts do not account for the entire kernel heap or prove aggregate
pressure/progress.

Existing node/producer allocation rejection, completion publication/namespace
reuse, queued and selected cancellation, callback/waiter/final observer disposal,
original weak backing, sleep/watchdog cleanup, nested IRQ/forced-preemption and
exact scheduled IPC exit controls continue to execute.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Node construction fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.049566 s | 17 passed, zero failed/pending at 10.155122 s |
| Intel VT-d, no network | Passed at 2.359463 s | 15 passed, zero failed/pending at 19.438507 s |
| AMD-Vi, no network | Passed at 1.061731 s | 15 passed, zero failed/pending at 14.741757 s |
| ARM / SMMUv3, security | Passed at 1.149938 s | 19 passed, zero failed/pending at 59.901146 s |
| Intel VT-d, security | Passed at 1.101863 s | 17 passed, zero failed/pending at 61.870700 s |

Together these complete **320 prepared empty node allocations beneath held heap
and actual queue guards**, preserving exact deadlines and original event/waiter
charges without allocation or disposal. Each profile also checks retained
allocation rejection, ordinary post-callback rollback, unused empty backing,
weak-only original admission and guarded phase abandonment. Existing complete
cancellation, unpublished disposal, post-queue delivery, nested-drain and final
observer destruction controls pass too.
Each profile also completes 64 scheduled IPC close/exit episodes with exact exit
watches, 320 total. Both security profiles complete two scoped EL0 generations. These
are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration. No failed native sample, panic, deadline or
persistence fail-stop was observed.
Two earlier introduced temporary-guard failures remain separately
preserved in its [unpublished ownership report](2026-10-16-timer-unpublished-ownership.md).
Historical failed samples remain separately
preserved in the [backlog refund report](2026-10-16-completion-backlog-refunds.md);
these passes do not establish their causes.

Commands share `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance timer-node-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance timer-node-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance timer-node-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-node-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-node-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel; evidence records actual network forwards.
All staged service files and run inventories match the preceding baseline byte
for byte; service/runtime libraries were not edited or rebuilt.

[Version-controlled evidence](evidence/2026-10-16-timer-node-construction.json)
records reviewed hashes for all four Rust files including the new partial-node
owner and fixture, commands, artifact/UART hashes, unchanged service inventories
and concise
observations. Frozen Rust hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

Raw event/admission-holder/cancellation allocation construction, raw Arc/Weak
finalization, first-use waiter preparation, completion's containing destructor
policy and broader callback/disposal IRQ or enclosing caller guards remain
open. Aggregate heap/principal admission and integrated pressure/progress remain
open. Historical IPC exact-worker exit, stack, persistence and NIC causes remain
unattributed. Finding dispositions are unchanged.
