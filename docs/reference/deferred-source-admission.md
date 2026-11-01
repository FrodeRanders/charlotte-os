# Deferred source-disposal backing

IPC claim waits, memory-close transfer waits and DMA unit-readiness waits prepare
one disposal node before creating their admitted observer list. That node now has
independent requested-byte/count admission through `deferred::PreparedDisposal`.
It covers empty candidates, published sources and producer-LP queued storage
until actual node deallocation. Observer-list backing and waiter entries retain
their separate admission and lifetime rules.

| `deferred_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 768 KiB | 1 MiB |
| Prepared/published/retained nodes | 12288 | 16384 |

The count ceilings are twice the existing observer-list ceilings (6144/8192).
Every complete source retains a list; the extra node capacity leaves headroom
for candidates before list creation and overlapping disposal. These ceilings
preallocate no RAM and leave all existing resource limits unchanged. A quarter
remains shared trusted reserve, without per-service progress entitlements.

The current native layout is 40 bytes, including its 16-byte admission charge.
Count ceilings therefore bind at 480/640 KiB; independent byte ceilings bound
future layout growth. Admission adds a short pool operation at preparation and
release, and can reject source preparation before heap access. It does not bound
allocator bookkeeping or guarantee capacity across all pools in an operation.

## Captured classification and ownership

Each existing source factory supplies its captured classification: the exact
IPC claim record, memory-close transfer owner or DMA operation/grant owner.
The node and its list use that same value. No source name, manifest or later
numeric-ASID lookup chooses reserve. The node retains its original class through
publication, source completion, queued delivery and release; a new operation's
classification cannot reclassify existing backing.

Admission and fallible allocation precede observer-list construction and local
producer publication. If the list rejects, the unused disposal candidate is
freed before its node admission is refunded. Existing outward errors remain
`ResourceLimit` for IPC/memory waits and `WaitRegistrationFailed` for DMA waits.
Rejection supplies no completion, replay or hardware-cleanup authority.

The complete source's existing `ManuallyDrop` owner still retains every field on
abandonment. Ordinary losing preparation explicitly cancels outside local
guards. Source finish closes its independent list and publishes its admitted
node into the existing producer-LP queue without allocation, callbacks or
refunds. Other LPs cannot drain that producer's queue.

After producer guards leave, the existing drain notifies detached waiters and
releases source nodes. Shared retirement storage deallocates each node before
refunding its charge. A retained interest, token or weak reference can keep list
backing alive after the disposal node is freed; its original list charge remains
independent. Callback execution and physical cleanup retain their existing
context rules. No new queue, cleanup family or recovery registry is introduced.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-01-deferred-source-admission.json)
records each of the three actual source factories under ordinary and total
candidate-node pressure. Per family, controls cover 64 injected node-allocator
failures, independent byte denials, 64 real list-admission rejections after node
allocation and 128 source denials while the heap is held. Trusted preparation
and consuming cancellation preserve ordinary accounting.

At total exhaustion, returning one ordinary candidate admits exactly one real
source. Its finish runs with actual producer, heap and node-pool guards held;
enqueue retains the full charge. Producer-LP drain then returns that exact slot
and bytes. The memory-close fixture also checks node release while a separate
late interest retains the list. Existing complete DMA grant/unit abandonment
retains its admitted source node under the new pool guard without adding another
deliberate-retention fixture. Shared collection host controls qualify
deallocation-before-refund and inert detached-node fallback.

Pressure is serialized kernel-fixture work, not sustained hostile concurrent
EL0 load. Native heap exhaustion is not forced; allocator failures are injected.
Source/list/token destructors beneath arbitrary outer guards, worst-case IRQ
latency, aggregate principal/general-heap budgets and essential-service fairness
remain separate qualification work. Waker/callback representation and other
payload/registry storage still need admission. SEC-07 remains partial; existing
cleanup G1/G4 boundaries apply.
