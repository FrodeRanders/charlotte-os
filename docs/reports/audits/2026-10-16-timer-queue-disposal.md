# Timer queue disposal after local borrow exit

Date: 2026-10-16. Baseline `05eeafd1`, following global completion namespace
metadata. This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gaps and qualified scope

The LP timer queue already inserted fallibly prepared anonymous nodes and kept
one inline scheduler quantum. Cancellation matched the exact retained state
allocation. Filtering still dropped removed nodes immediately under the queue
borrow; expiry moved an event out of its Box and freed that node there too.
Cancelled-before-enqueue nodes and duplicate/removed keyed payloads also ran
their remaining field destructors beneath the borrow. Queue storage destruction
iterated through ordinary disposal regardless of its enclosing context.

These are source findings. No historical panic, lock cycle, IPC exit, stack,
persistence or NIC incident is attributed to them. The preceding global-map
report identified this local boundary separately from callback execution and
outer-context qualification.

## Complete event handback

`RetiredEvents` reuses the existing owning chain and one inline keyed slot.
Filtering, purge, expiry and rejected enqueue move original nodes into this
complete owner without allocation, free, refund or notification. A duplicate
or removed quantum transfers by value into its inline slot. Each local queue
transaction can detach at most its one existing keyed event or one rejected
candidate; no new disposal node, reservation or registry is introduced.

Anonymous nodes retain their complete event, cancellation allocation, waiter
source, remaining callback reference and outside `EventAdmission`. Their
original charges follow actual backing and final cancellation strong/weak
references. Queue membership diagnostics remain distinct from retained backing
usage. A removed event can remain charged through the complete transaction,
including notification of other due events, before the receipt is released.
There is no new timer capacity, identity selector, public status or wire format.

Explicit finish iteratively detaches each next link before destruction,
preventing recursive stack use. The existing outside node admission survives
actual Box deallocation; retained cancellation backing can keep its original
account occupied afterward. Both retirement receipt and queue-storage fallback
retain every remaining node and inline keyed payload without allocator,
source cancellation, callback or budget work. Ordinary private queue fixtures
now explicitly consume their storage instead of relying on field Drop.

All production mutation callers have the handback:

| Caller | Borrow exit and disposal |
| --- | --- |
| `PreparedEvent::enqueue` | Comparator/publication remain locally masked; the queue guard leaves before rejected/purged payload disposal. |
| `cancel_event` | Exact retained state still selects one entry; the match's queue guard leaves before receipt finish and IRQ restoration. Busy/remote cases remain flag-only. |
| `process_local_events` | The queue guard leaves before explicit finish; the original IRQ policy is restored afterward. |
| ARM timer IRQ and x86 timer IRQ adapter | Explicitly drop the queue guard before finishing returned payloads; IRQ execution and architecture return order remain unchanged. |
| Round-robin quantum ensure, idle wake and stop | The temporary queue guard ends at the handback statement; explicit finish follows while the caller's existing scheduler context remains. |

Comparator rearm/stop, deadline ordering, exact cancellation identity and
diagnostic membership transitions remain serialized. `TimerEvent::signal`
still runs before front removal and still invokes callbacks/waiter notifications
under the queue borrow, including destruction of their temporary references
and detached waiter entries. This batch does not move notification or qualify
arbitrary callback work. Explicit finish can still run beneath an enclosing
scheduler/producer guard or in IRQ context; unpublished preparation and final
handle destruction have their existing contexts. No guard-free deferred
transport or scheduled disposal worker is added. This is the local payload
destruction boundary, not complete G1/G4/G7 qualification.

Ordinary rejection is unstarted disposal; expiry/removal is terminal logical
retirement. Abandonment is terminal retention, never Pending, an owner-preserving
retry API or recovery custody. No root pin, physical completion, resource
adoption, operation replay or readiness contract is added.

## Controls and retained fixture cost

Heap-held probes run in the synchronous phase before AP schedulers. They do
not authorize ordinary finish while an outer heap guard is held. The same
explicit finish helper checks local queue and heap availability immediately
before each fixture's actual event/node destruction; these are local serialized
checks, not cross-LP stress or all-caller context proof.

- Prepare and enqueue 64 real cancellable nodes, each with a waiter and callback
  reference. Remove all 64 exact identities under heap and queue holds. All
  original event and waiter charges remain live until post-queue finish.
  Final cancellation handles retain 64 event charges; one weak-only state keeps
  its original retired account charged until final weak release, without
  crediting an independent successor sponsor.
- Cancel one queued event while its queue is busy and another before publication.
  Enqueue under heap and queue holds purges the former and returns the rejected
  prepared node in the same complete receipt. Both event/source charges remain
  until explicit disposal; no notification is fabricated.
- Use a private local-comparator queue under the fixture's original mask.
  Under heap and queue holds, reject a duplicate quantum, fire one due anonymous
  event and the inline keyed event, retain the future deadline, and return the
  complete expired owners. Both callbacks are bounded atomic fixture counters
  with live external owners and no waiter entries; this does not claim that
  arbitrary notification is allocation-free. Also remove the future exact node
  and a keyed quantum under the same holds. Restore the real LP queue's
  comparator before returning.
- Abandon one complete detached anonymous receipt under heap and queue holds.
  Its original event/node/cancellation charge and waiter source remain intact.
  Later independent token cancellation releases its own waiter entry only;
  the event charge and empty admitted source allocation remain retained.

The negative control deliberately retains **one event reservation**, its node,
cancellation backing and one waiter-list allocation until guest shutdown, with
no persistent waiter-entry charge. It uses a private fixture sponsor, not a
replacement live thread sponsor or a user root. It runs after the timer fixture's
full-pool reservation checks and never resets counters. The existing completion
metadata controls still retain their separate ten records, four timer events,
four CQ payload charges, two watch charges and three authority charges. With
this new control, those related negative controls retain five event charges
in the shared timer pool. Successful reclamation remains separately asserted.

Existing exact/stale/weak cancellation identity, 1,024 real-node reclamation,
node/ordinary admission, producer-allocation failure, scheduled sleep/watchdog,
completion cancellation and returned-resource controls continue to run.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Timer disposal fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.122741 s | 17 passed, zero failed/pending at 10.690778 s |
| Intel VT-d, no network | Passed at 1.247632 s | 15 passed, zero failed/pending at 19.892020 s |
| AMD-Vi, no network | Passed at 1.077384 s | 15 passed, zero failed/pending at 16.145863 s |
| ARM / SMMUv3, security | Passed at 1.519268 s | 19 passed, zero failed/pending at 57.958880 s |
| Intel VT-d, security | Passed at 1.155910 s | 17 passed, zero failed/pending at 63.052064 s |

Together these execute **320 bulk exact timer-node removals under held heap and
queue guards**, plus purge, rejected enqueue, anonymous/keyed expiry, keyed
handback, weak-only retention and guarded abandonment controls. Each profile
also completes 64 scheduled IPC close/exit episodes with exact exit watches,
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
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance timer-disposal-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance timer-disposal-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance timer-disposal-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-disposal-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-disposal-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel; evidence records actual network forwards.
All staged service files and run inventories match the preceding baseline byte
for byte; service/runtime libraries were not edited or rebuilt.

[Version-controlled evidence](evidence/2026-10-16-timer-queue-disposal.json)
records reviewed hashes for all seven Rust files including the new fixture,
commands, artifact/UART hashes, unchanged service inventories and concise
observations. Frozen Rust hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

The next timer boundary is signal/callback execution under the LP queue borrow,
including waiter-entry and temporary observer destruction. Moving that work
requires preserving exact cancellation, readiness publication, comparator
reconciliation and owner-LP execution without inventing replay or physical
proof. Outer scheduler/producer/IRQ contexts, unpublished preparations and final
cancellation-handle cleanup still need qualification. Aggregate namespace,
heap/principal admission and integrated pressure/progress remain open.
Historical IPC exact-worker exit, stack, persistence and NIC causes remain
unattributed. No polling, forced IRQ enabling or custody policy changes.
