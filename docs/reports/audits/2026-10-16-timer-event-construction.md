# Timer-event and shared charge-holder construction

Date: 2026-10-16. Baseline `aa7277c`, following timer node construction.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gaps and qualified scope

The timer node constructor already retained its complete charged event while
allocating empty Box storage. Earlier construction still had ordinary ownership
gaps: sleep's raw event could lose its source/callback fields through implicit
failure cleanup, and cancellable events had no complete transaction through
reservation, charge-holder and cancellation-state preparation.

The shared `ChargedAllocator::try_new` also passed an owning charge in an
initialized holder value into its fallible Arc allocator callback. Timers,
completion records and observer lists use this primitive. Correcting that
constructor once avoids three separate timer/record/list implementations.
These are source findings, not attribution of any historical panic, IPC exit,
stack, persistence or NIC incident.

This batch qualifies partial charge-holder and timer-event construction. It
does not make completed raw events, published allocator references, arbitrary
resource-bearing Arc payloads or containing completion/list destructors inert.

## Shared holder ownership

`PreparingAllocation<C>` retains its original charge and any empty
`Arc<MaybeUninit<Allocation<C>>>` behind `ManuallyDrop`. The allocation callback
takes no owning charge. Private completion checks unique, unexposed storage,
initializes it once and converts that same allocation to the existing holder.
The single `assume_init` conversion follows the unique Arc initialization;
no bare or weak holder reference escapes the adapter.

The existing owning constructor explicitly cancels ordinary rejection after
the allocator callback returns. It frees empty storage before releasing unused
admission. The new parent-borrowing `try_new_retaining` instead borrows a wider
transaction's `Option<C>`: failure leaves that exact reservation installed in
the parent, while success consumes it only into the prepared holder. This lets
the event owner dispose its original payload before refunding uninstalled
admission. Both forms share the same storage/init/cancellation machinery; they
represent different containing ownership, not compatibility allocation paths.

Abandonment retains the preparation's fields without freeing, refunding,
callbacks, guards or logging. A live borrow retains parent ownership through
borrowed preparation. Published holders keep their existing final-release
policy: all allocator clones use `Arc::into_inner`; the final one frees holder
storage before refunding the original charge. The same allocation layout,
single-allocation allowance, failed-allocation reset, strong/weak lifetime and
concurrent final-owner rules remain. There is no new reservation or registry.

Generic `try_arc<T>` payload construction is unchanged. Its existing outer
allocator owner still keeps admission through payload-allocation failure, but
this batch does not qualify arbitrary payload field destruction there. Timer
cancellation payloads contain only atomics, and their enclosing event keeps
the original allocator owner through rejection.

## Complete event preparation

`PreparingEvent` captures the original raw event and source/callback before
reservation. It retains uninstalled admission separately until the borrowed
holder constructor succeeds, then retains installed admission in that same
event while cancellation-state allocation borrows it. Rejected admission or
state preparation cannot silently dispose the original event/source or refund
its retained charge inside an allocator argument.

Ordinary failure explicitly destroys the original event/source before releasing
any uninstalled reservation. Installed admission follows the event's existing
field order and exact cancellation strong/weak backing. Successful completion
consumes the same owner into a charged raw event, or into the existing node
preparation for sleep. A cancellation handle is cloned from the prepared state
only at successful return, so partial construction has no detached handle to
reconstruct or clean up.

Absolute deadlines remain unchanged. Relative cancellable constructors still
evaluate their deadline once after allocation; sleep still preserves its
existing event deadline until the scheduler's established post-registration
rebase. No wait budget, cancellation identity, queue publication, comparator,
readiness or userspace ABI changes. No enclosing mask is force-enabled.

Source-only, reserved, holder-installed and cancellation-installed abandonment
retain every remaining original field. This is terminal retention, not Pending,
an owner-preserving retry interface, physical cleanup or recovery custody.
Ordinary allocation/disposal still requires a qualified containing context;
these helpers do not remove arbitrary outer guards.

## Controls and retained fixture cost

Five new host tests cover allocator callback return before original-charge
disposal, retained parent admission on borrowed rejection, unique empty-holder
completion with the same one-allocation allowance, explicit unused empty
storage cancellation, and guarded abandonment before/after storage allocation.
Existing strong/weak and concurrent final-owner/one-allocation tests remain.
The two abandoned host-only preparations deliberately retain two synthetic
charge counters and one empty holder Arc until the test process exits; they
consume no kernel timer reservation and are separate from guest retention.

Native synchronous pre-AP controls prepare private sponsors before event
capture, without replacing live accounts or resetting shared counters:

- Prepare 64 complete original events through reservation, holder and actual
  cancellation allocation. Under held heap and actual queue guards, consume
  them into charged raw events without allocation/disposal or deadline changes.
  All original event/waiter charges remain. Explicit post-guard raw-event
  disposal releases sources; one weak-only cancellation allocation retains its
  original charge without crediting a fresh successor sponsor.
- Saturate a private fixture sponsor's 1,024 reservations. Rejected reservation
  retains the original source/deadline until post-guard cancellation. The public
  sleep adapter uses the same explicit error cleanup, closes its source entries
  and leaves all pre-existing reservations intact. This is private kernel
  fixture pressure, not a real EL0 exhaustion or shared-kernel saturation test.
- Reject the cancellation allocation adapter while heap and queue are held.
  The event still owns its exact deadline, source/callback and installed
  admission, with no fabricated cancellation state. Explicit post-guard
  cancellation releases that complete preparation.
- Abandon each of four owning phases beneath heap/queue holds: source-only,
  reserved admission, installed holder and installed cancellation. Every source
  and callback reference survives; the three admitted phases retain original
  event charges. Independent token cancellation releases only waiter entries.

The new guest negative controls retain **three original event charges, four
empty admitted source-list allocations, two private charge holders and one
cancellation allocation** until shutdown, with zero persistent waiter entries.
The source-only phase has no event reservation; its source has independent
list admission. No node Box, real LP delivery claim or user root is abandoned
by these controls. Earlier timer controls retain eight event charges;
completion metadata retains its separate ten records, four timer events,
four CQ payload charges, two watch charges and three authority charges. Related
negative controls now retain fifteen event charges in the shared timer pool.
They run after full-pool baseline checks; ordinary controls restore their
baseline separately. These quantities are not whole-heap accounting or
integrated hostile-pressure/fairness qualification.

Existing node/producer allocation rejection, namespace/publication revalidation,
queued/selected/stale cancellation, callback/waiter/final observer disposal,
sleep/watchdog, nested-mask/forced-preemption and exact scheduled IPC exit
controls continue to execute.

## Execution results

The host runner passes **505 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Event construction fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.609644 s | 17 passed, zero failed/pending at 11.614612 s |
| Intel VT-d, no network | Passed at 1.078982 s | 15 passed, zero failed/pending at 14.140233 s |
| AMD-Vi, no network | Passed at 1.104539 s | 15 passed, zero failed/pending at 16.650457 s |
| ARM / SMMUv3, security | Passed at 1.062583 s | 19 passed, zero failed/pending at 56.096642 s |
| Intel VT-d, security | Passed at 1.081383 s | 17 passed, zero failed/pending at 61.877325 s |

Together these complete **320 prepared events beneath held heap and actual
queue guards**, preserving deadlines, source/callback and original admission
without allocation or disposal. Each profile checks private full-sponsor and
cancellation allocation rejection, public sleep error cleanup, weak-only original
charges and guarded source/reservation/holder/cancellation phase abandonment.
The host suite adds five shared-holder tests; existing concurrent final-reference
and one-allocation controls remain. Existing node, cancellation, callback/waiter
and final observer disposal controls pass too.
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
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance timer-event-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance timer-event-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance timer-event-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-event-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-event-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel; evidence records actual network forwards.
All staged service files and run inventories match the preceding baseline byte
for byte; service/runtime libraries were not edited or rebuilt.

[Version-controlled evidence](evidence/2026-10-16-timer-event-construction.json)
records reviewed hashes for all six Rust files including the shared holder,
its host tests and event owner/fixture, commands, artifact/UART hashes, unchanged
service inventories and concise
observations. Frozen Rust hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

Completed raw event destruction, generic resource-bearing Arc payload
construction, published raw Arc/Weak finalization, first-use waiter metadata,
completion/list containing destructors and broader callback/disposal IRQ or
enclosing caller guards remain open. Aggregate heap/principal admission and
integrated pressure/progress remain open. Historical IPC exact-worker exit,
stack, persistence and NIC causes remain unattributed. Finding dispositions
are unchanged.
