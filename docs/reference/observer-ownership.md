# Kernel observer ownership and backing admission

Kernel notification targets use `ObserverRef<T>` and `ObserverWeak<T>`.
`Observer::notify(&self)` borrows the payload while the dispatcher retains a
strong handle for the complete call. Sources register only `ObserverWeak`;
global-allocator `Arc`/`Weak` values cannot enter this interface. A notification
remains a readiness hint, without completion, replay or cleanup authority.

The allocator-independent trait and opaque handles live in
[`observer/owned.rs`](../../crates/catten/src/klib/observer/owned.rs). The handles
keep their admitted allocator private. They expose clone, downgrade, upgrade,
pointer identity and consuming type erasure, without raw-pointer adoption or
allocator extraction. Empty weak handles own no allocation or admission.
Erasure, cloning and successful upgrade allocate nothing and preserve payload
identity. In particular, scheduler wake admission still checks both exact thread
generation and the installed waker's payload address.

## Independent backing admission

| `observer_backing` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested observer `Arc` backing bytes | 12 MiB | 16 MiB |
| Allocation bundles | 49152 | 65536 |

One bundle covers a single observer allocation and its fixed private charge
holder. Count admission precedes holder allocation. The allocator then admits
the actual `Arc` layout before allocating payload backing, including its control
header and alignment padding. The byte dimension excludes the separately
count-bounded fixed holder and allocator bookkeeping. Separate allocations inside
captures retain their existing admission; this is not aggregate heap accounting.

The count default is twice a conservative four-population planning envelope:
8192 each for waiter, timer, event-watch and pending-call CQ-bridge populations
(6144 ordinary each). Some populations share other ceilings. Headroom permits
coexistence, unused preparation and weak retention, without lowering existing
resource limits. A quarter remains shared trusted reserve. No RAM is
preallocated, and separate pool capacity does not prove service fairness.

On supported 64-bit builds, each handle gains one allocator-owner word: typed
strong/weak handles occupy two words, and erased handles three. Each observer
also needs a fixed private-holder allocation. These costs provide one uniform
allocation owner through final weak release. Existing pre-admitted families keep
the default zero-sized allocator policy and their existing admission behavior.
Size-based thread/table admission includes containing-layout growth; the native
thread-metadata bundle now measures 688 bytes on AArch64 and 1216 bytes on x86-64.

`ChargedAllocator` now optionally accepts a zero-sized `PayloadAdmission` policy.
Observer admission uses its actual-layout hook. Layout denial changes no byte
charge; failed Global allocation rolls accepted bytes back before resetting the
unstarted allocation allowance. Successful allocation consumes that allowance
once. Clones cannot multiply or revive it. The original bundle and byte charge
survive payload destruction, weak-only backing and allocator references; final
payload and private-holder deallocation precede refund. Reading an original
charge borrows it without exposing the holder's Arc or a weak reference.

Count exhaustion rejects before heap access. Byte denial rejects before payload
allocation, after the independently admitted holder exists. Ordinary constructor
failure disposes owning captures, so prepare observers outside producer guards
and retain the containing operation through rollback. The shared constructor's
inert preparation policy and existing wider caller/IRQ requirements remain.

## Allocation inventory and captured classification

Every row uses the shared backing pool, without a per-domain/principal observer
account. The original captured classification stays with the allocation;
promotion, source destruction, retirement or slot reuse cannot reclassify it.
No later numeric-ASID lookup or application role/name chooses reserve.

| Target and construction boundary | Captured sponsor/class | Strong owner and weak retention | Existing rejection |
| --- | --- | --- | --- |
| Scheduler waker, `prepare_wait` | Exact thread's generation-owned waiter sponsor | Prepared wait / Blocked thread; source entries | `WaitRegistrationFailed` |
| Timed-wait watchdog | Already admitted timer event's original charge | Wait loop; timer callback slot | `WaitRegistrationFailed`, preserving pending work and original deadline |
| Completion timer | Prepared namespace target's class | Completion record; timer callback slot | `WouldBlock` |
| Detached timer | Prepared CQ/namespace target's class | Detached record; timer callback slot | `WouldBlock` |
| Worker/thread-exit completion | Original `EventSubmission` watch charge | Completion producer observation; exact thread-exit registration | `WouldBlock` |
| Connection-close completion | Caller's original `EventSubmission` watch charge | Completion producer observation; endpoint-close list | `ResourceLimit` |
| Pending-call CQ bridge | Pending call's class captured at record creation | Pending call's `ReplyCqInterest`; call list | `ResourceLimit` |
| Per-LP quantum observer | Mandatory kernel bootstrap | One scheduler owner per LP; keyed timer callback slots | Mandatory boot failure; constructor is not application-triggerable |
| Generic `CallOnNotify::try_new` | Explicit captured owner classification | Returned handle and its registered weak references | `AllocError`; `for_test` is a mandatory fixture-only constructor |

Timer observer factories receive their captured class explicitly alongside the
payload. Pending-call construction stores its original class beside its admitted
list; CQ binding borrows that record rather than resolving a successor root.
Waker classification capture adds no new readiness/retirement decision: source
registration and wait publication retain their existing checks. Producer
callbacks and final releases still leave local source, scheduler and registry
guards. Borrowed notification does not authorize an arbitrary callback beneath
those guards, and existing complete-operation fallback remains unchanged.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-02-observer-ownership.json)
identifies all changed sources and final native artifacts. Four host tests compile
the actual trait, handle implementation, allocator and backing policy with a host
mutex adapter. They cover type erasure/identity, borrowed notification, weak-only
retention, concurrent final weak destruction, empty weak handles and aligned
zero-sized payloads. Eighteen shared allocator tests include actual-layout
admission, heap-free layout rejection and rollback/retry after allocation error.

Native controls fill ordinary and total bundle ceilings with real weak-only
observer allocations. Separate 1024-byte inline payloads reach ordinary and total
byte ceilings with count headroom. Observer backing remains charged after its
last strong reference dies. Controls verify guarded allocation-free erasure/upgrade,
128 bundle denials with the heap held, pre-payload layout denials, trusted reserve
and exact final-weak recovery. At total exhaustion the actual scheduler rejects
64 waker preparations before source registration, preserving thread generation,
state and waiter usage. Existing immediate-readiness, early/stale-wake,
timer/worker/exit rollback, CQ binding, cancellation and scheduled wait campaigns
use the same admitted representation.

The pressure fixture is serialized kernel work; it does not force general heap
OOM or establish sustained hostile concurrent EL0 fairness. Fixed-holder bytes
are bounded by bundle count, and nested captures/general heap still need
aggregate principal accounting. Non-observer weak/control storage and other
registry/payload allocations remain unfinished. Arbitrary destructor/IRQ context,
worst-case latency and physical completion retain existing G1/G4 boundaries.
SEC-07 remains partial.
