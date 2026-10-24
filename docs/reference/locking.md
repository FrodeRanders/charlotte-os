# CharlotteOS synchronization primitives and lock ordering

This document enumerates the synchronization primitives actually used by the
kernel, their semantics, and the ordering rules that keep them from
deadlocking. It complements the scheduler-specific ordering in
[`scheduler-state-machines.md`](scheduler-state-machines.md) §8, which covers
the `SYSTEM_SCHEDULER → lp_scheduler → MASTER_THREAD_TABLE` chain.

When two documents disagree, the source is authoritative. Paths below point at
the defining module.

---

## 1. Lock families in use

| Family | Module | Interrupt policy | Blocking? | Typical users |
|---|---|---|---|---|
| Interrupt-masking spin `Mutex` | `cpu/multiprocessor/spin/mutex.rs` | Masks IRQs while owned; restores the caller's state while contending | No (spin) | Frame allocator, memory-object registry, address-space table, kernel AS, domain authorities, AS lifecycle, scratch-window allocator, talc |
| Interrupt-masking spin `RwLock` | `cpu/multiprocessor/spin/rwlock.rs` | Masks IRQs while owned; nesting-aware save/restore and interruptible contention | No (spin) | `MASTER_THREAD_TABLE`, `DEAD_THREADS`, `SYSTEM_SCHEDULER`, `IPC`, `COMPLETIONS`, `USER_MAILBOX_CAPS`, each `PerLp` slot |
| External `spin` crate | `spin::{Mutex, RwLock, LazyLock}` | None (manual, see §3) | No (spin) | Stack allocator arena + guard-page map; one-time lazy init everywhere |
| talc global-allocator lock | `TalcLock<MutexCore, ExtendOnOom>` in `memory/allocators/global_allocator.rs` | Masks IRQs (reuses the spin `MutexCore`) | No (spin) | Kernel heap |
| Lock-free containers | `ShardLocal`, `concurrent_queue::ConcurrentQueue`, `Atomic*` | n/a | n/a | Per-LP state, IRQ→thread deferred-wake handoff, generation counters, `on_cpu` |

There is also a **scheduler-blocking** lock family under
`cpu/scheduler/sync/{mutex,rwlock}` that parks the caller via `block_thread`
instead of spinning. It has **no production callers**; kernel fixtures now
exercise remote contention and admission failure. Its owning waiter entries
share the [scheduler budgets](scheduler-waiter-budgets.md). Lock acquisition
retries while runnable on admission failure. Unlock broadcasts candidates
after releasing ownership; RwLock broadcasts only when its final reader leaves
or its writer releases. This is not FIFO handoff or a fairness guarantee, and
does not replace the interrupt-masking family in existing production paths.

Device IRQ readiness uses statically initialized per-route atomic mailboxes.
Thread-context drain takes `DEVICES → COMPLETIONS → waiter list` to validate
the route and detach the exact queue's notification batch. All these guards
are released before callbacks run. Individual close and address-space teardown
retire interrupt routes under `DEVICES` before another grant can reuse them.
See [interrupt wake storage](interrupt-wake-storage.md).

Mailbox capability open uses `ADDRESS_SPACE_LIFECYCLE → USER_MAILBOX_CAPS`
before domain/node record counters or unified identity minting. This closes the
retirement/publication window. Its send/receive paths do not acquire the
lifecycle guard. Teardown already owns that guard before draining mailbox
payloads; counter Drop never reenters the registry or lifecycle. See
[mailbox capability budgets](mailbox-capability-budgets.md).

User address-space registration stages capability-budget metadata under
lifecycle before taking the address-space table, then publishes its exact
namespace. Shared admission under the completion registry uses that registry's
captured generation: `COMPLETIONS → CAPABILITIES → capability domain → node`.
It never enters lifecycle or the address-space table under `CAPABILITIES`.
Mailbox's already-owned lifecycle guard is borrowed through its shared-admission
helper, not recursively acquired. Platform designation releases the memory
ledger and address-space guard before updating the matching namespace.
Entry-charge Drop enters only capability counters; reservation/escrow Drop
enters `CAPABILITIES` and must run outside that registry's own guard. See
[shared capability admission](capability-admission.md) for enforcement scope.

Device grant/close ordering is `ADDRESS_SPACE_LIFECYCLE → DEVICES → CAPABILITIES
→ capability domain → node`. Grant borrows lifecycle for reservation, never
reacquires it under `DEVICES`. DMA creation retains lifecycle but runs without
the device registry; its rollback owner is declared before the device guard,
so backend destruction runs after that guard releases. Observer grants check
their exact handle under lifecycle. Observer startup uses a cancellable atomic
claim, not a registry guard held through loader/domain teardown.

IPC namespace storage and both budget accounts prepare before root-publication
guards. Fresh registration acquires its first exact-root lease under lifecycle
and the root table, then releases both guards before relinking the prepared
namespace under IPC alone. The containing registration releases the lease after
IPC unlock. This adds no lifecycle/table-to-IPC edge; existing IPC root lookups
retain their order. Fenced close detaches the empty namespace node under IPC and
disposes it and its account references after unlock. Partial/complete preparation
and detached-node fallback retain all fields without allocator or registry entry.
Payload/shared-authority factories under IPC and wider disposal contexts remain
separate. See [namespace publication](ipc-namespace-publication.md).

Pending-call preparation captures its caller under IPC, releases IPC before
acquiring the exact-root operation lease, then revalidates and reserves call/reply
counts under IPC. It leaves that guard before fallible pending/reply-node allocation.
Count/node admission stays in the containing owner. Pending publication only
relinks the prepared node under IPC. Close/cancellation detach its complete node
and release payload/storage/charge after IPC unlock. Ordinary unused preparation
finishes after local IPC guards and releases its root last; fallback retains all
fields. This uses C08/C18, with no new disposal registry. Other IPC maps, vector/
waiter construction and wider enclosing contexts remain separate. See
[record storage](ipc-record-budgets.md#preparation-and-retirement).

Reply-token node allocation now follows the pending node in the same call owner,
after count admission and outside IPC. Publication relinks the prepared node;
its charge stays with the allocation. Reply/cancellation owners and the existing
serialized close operation detach token nodes without allocation and release
them after IPC unlock. Serialized close reuses those nodes as its temporary
retirement list. Scalar completion retains its detached notifications with the
node and delivers them after disposal. Fallback retains complete new owners.
Claim-source preparation reads the token's original captured classification;
no current-ASID reclassification or new guard edge is added. Other map/factory/
payload destructor and enclosing IRQ contexts remain separate. See
[reply storage](../reports/audits/2026-10-24-security-ipc-reply-storage.md).

IPC endpoint/direct-grant/receiver-reply admission uses the generation captured
in `AsIpcCaps`: `IPC → CAPABILITIES → capability domain → node`. Family charges
and result writes run outside capability/counter guards. `PreparedReceive`
exclusively borrows IPC while speculative reply authority is installed; its Drop
removes only that capability on result-write failure, without consuming a token
or invoking cancellation callbacks. Publication precedes result writing/dequeue,
so there is no fallible admission after either mutation. Lifecycle is never
acquired under IPC.

`PreparedCall` and returned connections compose their reserved IPC identities
with memory using `commit_transfers_with_authority`: `IPC → MEMORY_OBJECTS →
CAPABILITIES → capability domain → node`. Both payload guards serialize joint
publication and subsequent infallible registry insertion/enqueue. Attachment
owners release their pins outside `MEMORY_OBJECTS`, still under IPC. Scalar-only
transactions skip `MEMORY_OBJECTS` and publish under IPC → CAPABILITIES.
Shared/family quota admission precedes reply loan revocation; later unmap or
retirement failure can leave individually revoked loans, but no fresh returned
authority is published on batch validation failure.

Memory-object admission captures source/target handles before taking
`MEMORY_OBJECTS`, then uses `reserve_captured`/`escrow_captured`:
`MEMORY_OBJECTS → CAPABILITIES → capability domain → node`. Batch commit holds
the memory registry while validating and atomically publishing all mixed-mode
authorities; payload updates follow under the same guard. Neither admission nor
publication acquires lifecycle under a subsystem. `PreparedTransfer` Drop cancels
captured authority before releasing its backing-retention pin, which reenters
`MEMORY_OBJECTS`; the owner must therefore drop outside that registry guard.
IPC vector owners drop under IPC serialization; private loan cancellation
restores source escrow without live borrower state or an unmap/shootdown.
Committed reply/cancellation revocation uses the existing lifecycle-free loan
helper and removes each successful revocation from the reply token immediately.
Private copy frames use captured-generation backing admission under address-space
table → memory ledger, preventing a delayed allocation from charging a successor.

---

## 2. Interrupt-masking spin locks

### 2.1 `Mutex` (`spin/mutex.rs`)

A `lock_api::RawMutex` (`GuardNoSend`) that attempts acquisition with maskable
IRQs disabled and keeps them disabled for the ownership interval. If the
attempt fails, it restores the caller's original interrupt state before
spinning and retries with IRQs masked. Unlock restores the state saved by the
successful attempt. Why ownership masks IRQs is explained in
`memory/mod.rs:19-24`: the locks it guards are taken from both preemptible
kernel threads and synchronous userspace exception paths. If the owner could
be timer-preempted, every LP could end up spinning for a lock whose owner can
never be scheduled again.

Properties:

- **Non-reentrant**: re-acquiring the *same* lock on one LP deadlocks.
- **Interruptible contention**: a waiting LP can handle timer and synchronous
  TLB-shootdown IPIs. This prevents a lock holder waiting for an IPI
  acknowledgement from deadlocking with a remote contender.
- **Nesting across *different* locks works**: each lock records its own
  pre-acquire interrupt state, so a nested acquire on another `Mutex` sees
  "already masked" and correctly defers unmasking to the outermost unlock.
- The saved interrupt flag lives on the lock object itself, not per-LP; it is
  correct because the guard is non-sendable and IRQs are masked for the entire
  ownership interval.

Users (all via `memory::Mutex`): `PHYSICAL_FRAME_ALLOCATOR`,
`MEMORY_OBJECTS`, `ADDRESS_SPACE_TABLE`, `KERNEL_AS`,
`DOMAIN_AUTHORITIES`, `ADDRESS_SPACE_LIFECYCLE`, `SCRATCH_WINDOW_NEXT`, and
the talc lock.

### 2.2 `RwLock` (`spin/rwlock.rs`)

A `lock_api::RawRwLock` (`GuardNoSend`) over an `AtomicI64` reader count with
`-1` for a writer. Acquisition attempts run with IRQs masked, but a failed
attempt restores `INT_STATE` before spinning. The `waiting_writers` counter
gives a writer preference only during its masked atomic acquisition attempt;
it is deliberately cleared before the writer waits. Leaving writer preference
asserted while waiting could deadlock an interrupt handler on the same LP that
needs a read guard before the writer can acquire the lock.

Instead of a per-lock saved flag, it uses the shared per-LP
[`INT_STATE`](#3-interrupt-masking-discipline) save/restore, which is
**nesting-aware** (a per-LP save count). This lets a thread take several
different `RwLock`s, or a read-after-read, without prematurely re-enabling
IRQs. Re-acquiring the same lock for writing is still a deadlock.

As with `Mutex`, interruptible contention is required by synchronous cross-LP
operations. A CPU waiting for a lifecycle or page-table lock must remain able
to acknowledge a TLB-shootdown IPI initiated by the current lock owner.

This is the workhorse lock: it protects the IPC registry, the completion
registry, the master thread table, the deferred-dead table, the system
scheduler, and each `PerLp` slot.

---

## 3. Interrupt-masking discipline (`INT_STATE`)

`cpu/multiprocessor/interrupt_tracking/int_save_restore.rs` maintains, per LP:

- a raw `AtomicBool` lock protecting the counters,
- a `save_count` (nesting depth),
- the saved interrupt-enable bit for the *outermost* save.

`save_int()` masks IRQs, bumps the depth, and records the enable bit only on
the 0→1 transition; `restore_int()` unmasks only on the 1→0 transition. The
`RwLock` and `interrupt_depth` (`extern "C"` entry/exit hooks) share this
machinery so nested lock acquisitions and nested exception entry re-enable
IRQs exactly once.

The stack allocator does **not** use `INT_STATE`: `STACK_ARENA_LOCK`
(`memory/allocators/stack_allocator.rs:66-115`) is an external `spin::Mutex`
wrapped in explicit `mask_interrupts!`/`unmask_interrupts!`. The external
`spin::RwLock` guarding `KERNEL_GUARD_PAGES` is only touched from thread
context (stack spawn/teardown), never from IRQ context, so it needs no mask.

---

## 4. Lock-free structures

- **`ShardLocal<T>`** (`spin/shard_local.rs`) — lock-free per-LP storage behind
  an `UnsafeCell`, gated by an owner-check plus a per-LP borrow flag that
  rejects re-entrant access. References never escape the closure. Cross-LP
  mutation is only through `unsafe with_on_lp` under IPI/closure dispatch.
  Use when state is strictly LP-local.
- **`PerLp<T>`** (`spin/per_lp.rs`) — a `Box<[RwLock<T>]>`, i.e. sharded
  interrupt-masking spin rwlocks; cross-LP access via `unsafe get_nonlocal*`.
  Use when an ISR or another LP may need to touch the slot.
- **`ConcurrentQueue`** — used by per-LP IPI command ingress, among other
  bounded handoffs. It is not the sole IRQ-to-thread mechanism. Device readiness
  uses independent atomic mailboxes, while owning observer lists have their own
  interrupt-masking guards and detached notification batches.
- **`Atomic*`** — `on_cpu` byte-sized ownership handshake, generation
  counters, `IRQ_PENDING` counts and generation-tagged `DEFERRED_WAKES`
  mailboxes. `deliver_interrupt` takes no locks (see
  `scheduler-state-machines.md` §8, LO5).

---

## 5. Lock-ordering rules

The scheduler chain is fixed and is documented in
[`scheduler-state-machines.md`](scheduler-state-machines.md) §8:

```
SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
```

Cross-subsystem rules that hold today:

| Rule | Description | Source |
|---|---|---|
| **LO-alloc** | Never take the address-space or frame-allocator locks while the talc heap lock is held. The heap's growth reserve is pre-mapped at boot so `ExtendOnOom` can extend within mapped memory without taking those locks. | `global_allocator.rs:41-48` |
| **LO-mem** | Do not hold the memory-object registry across the address-space table lock. Teardown takes table → frame allocator; allocation takes allocator → registry; holding registry → table closes an AB-BC-CA cycle. | `memory/object.rs` (`map_locked`) |
| **LO-mem-copy** | Bulk copying may run without the memory-object registry only after acquiring a shared-read copy pin. While any copy pin exists, every tracked writer path (writable CPU mapping, in-kernel write, writable or exclusive DMA pin, write lend, move, or rollback) must fail. Owner teardown marks the object for deferred destruction; the final DMA/copy release removes it only when both pin counts are zero. Frame deallocation occurs after releasing the registry lock. | `memory/object.rs` (`pin_for_copy`, `take_deferred_frames_if_unpinned`) |
| **LO-principal-backing** | Signed identity installation holds the captured address-space table, prepared authority slot and generation ledger before the leaf principal counter. The existing principal-history map supplies admitted, boot-lifetime storage. Page/charge reservation and release enter only its IRQ-preserving principal-record lock; no allocator, pool, table or subsystem is entered from that guard. Initial verified-principal preparation publishes its admitted record before architecture/root guards. Table accounts and stack reservations retain typed shares; abandoned Drop enters no counter. Principal reservation ends before node-pool admission; rollback releases pool guards before principal disposal. Preparation abandonment only merges existing shares and drops an emptied owner, including beneath the principal guard. Physical completion still precedes refund. | `memory/{budget,backing_budget,usage/principal_history}.rs` |
| **LO-memory-budget** | Admission checks the generation under the address-space table lock before taking the budget ledger. Ledger operations never enter IPC or the object registry. Staged frames release physical storage before dropping their charge; no allocator lock is held when the charge returns to the ledger. | `memory/budget.rs`, `memory/object.rs` (`ChargedFrames`) |
| **LO-user-backing-admission** | The address-space table guard validates the captured generation and covers heap or image/runtime reservation, fallible frame tracking, allocation, initialization, mapping and charge commit. Backing-pool counters never enter another subsystem; their first-use RAM sizing takes the frame allocator before installing the pool, without holding a pool guard. Allocator guards end before failed frame owners refund charges. Retirement marks both embedded accounts under the table guard; physical teardown precedes account Drop/refund. Loader preparation cleanup acquires lifecycle only after its mapping/table guards are released. | `memory/{mod,backing_budget}.rs`, `service/loader.rs`, both architecture `paging/mod.rs` |
| **LO-IPC-retirement** | Namespace cleanup borrows the exact closing root, fences admission under IPC, then uses per-token cancellation with peer leases and loan receipts outside IPC/lifecycle. Revoke only after claiming the exact token; confirmed cleanup precedes terminal results and capability removal. Failure retains uncertain Revoking/pin/backing and the IPC cleanup fence; only unstarted receipts cancel. Serialized boot adapters must not acquire lifecycle under IPC and retain their separate context qualification. | `ipc/namespace_close.rs`, `ipc/cancellation.rs`, `memory/object/revocation.rs` |
| **LO-timer-admission** | Timer budget operations take domain counter before node counter and never enter another subsystem. Platform identity is sampled before the completion registry is locked and compared with its stored generation-qualified identity. Enqueue runs after dropping the registry. Cancellation under the registry may only try the local queue guard; a busy or remote queue retains the flagged event and its charge for later purge. | `timers/budget.rs`, `timers/mod.rs` (`cancel_event`), `completion/mod.rs` |
| **LO-completion-record-admission** | Record reservation takes the completion registry, then domain counter, then node counters. Budget guards allocate nothing and enter no other subsystem. Platform identity is sampled before the registry; retirement is checked against its stored generation. A retained completion owns its charge after cap closure; a detached result transfers it into the CQ backlog until delivery/discard. | `completion/budget.rs`, `completion/mod.rs` (`reserve_record`, `complete_detached`, `replace_cq`) |
| **LO-completion-queue-admission** | CQ staging takes the completion registry, then domain and node counters. Counter guards neither allocate nor enter another subsystem. Backing allocation and physical-ring initialization follow reservation; failure preserves the old registry entry. Loader rollback enters lifecycle teardown only after CQ setup releases the registry. Platform identity is sampled before the registry and matched to its captured generation. | `completion/cq_budget.rs`, `completion/mod.rs` (`stage_cq`, `attach_cq`), `service/loader.rs` (`PreparingDomain`) |
| **LO-close-watch-registration** | Staging reserves under the completion registry, then watch domain/node counters, and releases that registry before IPC registration. IPC may enter the independent list lock; token Drop may enter it from completion teardown, but the list never enters either registry. Entry allocation precedes the list lock, removed entries are dropped after it, and detached callbacks run only after IPC/list locks are released. Rollback validates the exact staged object. | `klib/observer/registration.rs`, `completion/watch_budget.rs`, `completion/mod.rs` (`EventSubmission`), `ipc/mod.rs` (`watch_connection_closed`, `close_cap`) |
| **LO-scheduler-waiter-registration** | Capture the sponsor at Thread construction via address-space table → memory ledger. Parking follows LP scheduler (Ready only) → master thread table → source registry (CQ/IPC sources) → independent waiter counters/list. Counters use domain → node and never enter a subsystem; token Drop enters only the list and drops removed charges after its guard. Waker cancellation on Ready/reap cannot invoke callbacks. IPC/CQ source notifications release registry/list locks before scheduler callbacks; combined detached batches do not allocate. IPC prepares waiter lists before transferring call attachments. IPC close-claim waits lease their captured root after IPC unlock and prepare source/disposal storage outside IPC. Relinking checks the exact namespace, capability and claim epoch; registration enters only the independent list beneath the master table. Claim removal/rollback/failure queues its admitted source in shared producer-LP transport; callback and final source disposal run after producer guards. Owned endpoint front waiting borrows the existing server lease and retains its interest. Serialized cleanup returns Pending rather than parking beneath lifecycle. Retire/promotion may enter waiter counters from the memory ledger, never the reverse. | `klib/observer/{mod,registration,waiter_budget}.rs`, `memory/budget.rs`, scheduler, `completion/mod.rs`, `ipc/mod.rs` |
| **LO-reaper-readiness** | Staging links the complete existing node under `DEAD_THREADS`, then detaches admitted waiters into the shared producer-LP transport without callbacks/allocation. Reaper registration enters only its independent owning source, never the retirement table under the master table. Only the producer LP claims from yield/idle and post-switch dispatch after local scheduler guards leave; short setup masks bind enqueue/claim before preemption/migration. Source/list/queue guards leave before generation-bound Waker admission. Dispatch preserves the containing IRQ state and performs no stack cleanup. The durable head recheck follows park setup and releases its guard before yield. Physical release remains on IRQ-enabled pinned workers; failed pairs are nonready. | `cpu/scheduler/threads/mod.rs`, scheduler and both architecture `lp/ops.rs` |
| **LO-memory-close-readiness** | A transfer stores its exact source root before arming its fence. A waiting close leases that captured root after memory unlock, never resolving a reusable ASID under the registry. Prepare the charged interest and admitted disposal node outside memory/lifecycle/IPC; relink after exact object/fence revalidation. Registration uses only the independent list beneath the scheduler table. Producer escrow/pin/fence completion precedes closing that source and queuing its admitted metadata in the same producer-LP transport as reapers. Only that LP claims after producer guards leave; callbacks and final list/node disposal leave all local source/queue guards. Wait rejection leaves authority/pins intact. Nonwaiting close keeps its existing serialized fixture path. | `memory/object/{close.rs,close/tests.rs}`, `cpu/scheduler/deferred.rs` |
| **LO-blocking-lock-waiters** | First contention may allocate a list under the independent initialization spin guard; release it before entry reservation/allocation and registration. Token Drop enters only the list. Release data ownership and detach every selected list before callbacks. Mask local IRQs across park/lost-wake recheck so unlock-before-registration cannot strand a preempted Blocked caller. No initialization/list/scheduler guard or local mask crosses yield. Final-reader RwLock release broadcasts both classes, without ownership handoff. | `klib/observer/waiter_source.rs`, `cpu/scheduler/sync/{mutex,rwlock}/mod.rs` |
| **LO-timer-observers** | Timer scheduler registrations use the same independent owning source. Raw timer callbacks occupy one embedded weak slot. Detach both forms before callbacks, releasing their guards but retaining the LP-local queue borrow; callbacks must not re-enter that queue. Sleep masks park/rebase/enqueue and drops the mask before yielding. On rejected admission it discards the unqueued event and waits runnable to the deadline. | `timers/mod.rs`, `cpu/scheduler/mod.rs` (`sleep_with_event`) |
| **LO-scheduler-timer-preparation** | Capture the timer sponsor at Thread construction through address-space table → memory ledger. Clone it under the master table, then release that guard before domain → shared-node event counters. Reserve/prepare watchdog callback, cancellation state and fixed-size node before parking; all fallible preparation precedes Blocked. Anonymous insertion allocates nothing. Completion-node staging does not borrow a queue under its registry; enqueue follows registry release. The inline quantum slot consumes no anonymous-node charge. Publication updates cancellation's shared owner LP under the enqueue mask. | `timers/{budget,queue,mod}.rs`, `memory/budget.rs`, scheduler, completion and syscall timed waits |
| **LO-endpoint-admission** | Endpoint budgets take domain counter before node counter; neither counter guard allocates or enters another subsystem. Capture the namespace under IPC, leave before exact-root lease admission, then revalidate before original-class record/slot admission. Endpoint node, queue, lists and shared-authority storage prepare outside IPC; shared count admission relinks under IPC, then capability payload allocation leaves it again. Growth keeps both backings and the original sponsor/class through checked FIFO replacement and post-IPC disposal. Root completion stays last. | `ipc/budget.rs`, `ipc/endpoint_storage.rs`, `ipc/cap_storage.rs` |
| **LO-IPC-record-admission** | Under IPC, revalidate namespace generation/retirement and kernel platform identity through the address-space table and memory ledger, releasing those guards before domain → node record counters. Counters allocate nothing and enter no subsystem. Reserve call/reply records before waiter allocation or attachment transfer. The exact-root call owner prepares shared-authority storage outside IPC, admits the captured shared count under IPC, then allocates payload and pending waiter list unlocked; staging only revalidates. Direct grant/receive/returned-connection factories and mixed memory payload maps remain guarded; admit delegated/returned connections before transfer or loan revocation. IPC retirement fences receive and connection publication before teardown's capability snapshot. | `ipc/record_budget.rs`, `ipc/mod.rs` (`accepting_namespace`, `stage_call`, `complete_reply`, `close_address_space`) |
| **LO-noblock-under-lock** | Never call `block_thread`/`yield_lp`/`cond_yield_lp` while holding any lock. A spin lock additionally masks IRQs; parking the thread would abandon the lock and the LP cannot schedule its successor. All guards are dropped before `switch_ctx` (`scheduler-state-machines.md` LO4). | `scheduler-state-machines.md:372-375` |
| **LO-block-event** | If a thread publishes itself as `Blocked` before installing the event that will wake it, mask local IRQs across both operations. Otherwise a scheduler-quantum IRQ can switch out a `Blocked` thread before any wake source exists. Timed waits use `LocalInterruptMask` across park/watchdog enqueue/recheck; rejection restores IRQ state and the mask is dropped before yield. Do not hold a lock across yield. | `scheduler/mod.rs` (`sleep`, `block_until`), `completion/mod.rs` (`wait_on_cq_timeout`), `syscall/mod.rs` (`sys_completion_wait_timeout`) |
| **LO-irq** | IRQ context takes no locks and never blocks. | `scheduler-state-machines.md` LO5 |

The interrupt-masking spin locks and the scheduler's `block_thread` path are
compatible only because blocking never happens under a held spin lock: a
blocking syscall releases its registry guards before registering a waker and
yielding. Local IRQ masking without lock ownership is permitted for the short
`Blocked`-publication/event-installation transaction described by
**LO-block-event**; IRQ state is restored before `yield_lp()`.

---

## 6. Choosing a primitive

| Need | Primitive |
|---|---|
| Shared state touched from syscall and/or IRQ context | Interrupt-masking spin `Mutex` (single writer) or `RwLock` (read-mostly) |
| Data read from several LPs, written rarely | Interrupt-masking spin `RwLock` (brief writer preference during each acquisition attempt) |
| Strictly per-LP data | `ShardLocal` (lock-free) or `PerLp` (if ISR/cross-LP reach is needed) |
| IRQ → thread handoff | `ConcurrentQueue` + atomics, drained in thread context |
| Cross-LP mutation | IPI/closure dispatch, never direct shared-memory writes |
| One-time global initialization | `spin::LazyLock` |

## Thread context/node accounting

Thread preparation validates the captured root through address-space table →
memory ledger. Principal reservation uses the existing history counter, releases
that guard, then enters the independent thread-metadata node counter. Node
rejection releases its guard before undoing the local principal reservation.
Final post-deallocation refund enters the node counter, leaves it, then releases
the original principal charge. Neither counter allocates or enters root,
scheduler, allocator or observer registries. Published thread/node/list Drop
retains the charge without entering either counter. No fresh counter nesting is
introduced between node and history. Explicit metadata disposal and ordinary
unused preparation still require their existing post-guard contexts. See
[thread metadata admission](thread-metadata-admission.md).

## Principal-history node admission

Prepare history/candidate node admission under its independent leaf counter,
leave that guard, then allocate outside lifecycle/root/authority/history holds.
Publication only relinks the charged node under the existing history guard;
losing candidates remain with their root. Explicit unused or confirmed final
metadata disposal deallocates the node, then enters the counter to refund its
original classification. Published table/candidate fallback retains both without
locks. The counter never looks up a root, enters history or allocates. Ordinary
cancellation and root metadata disposal still require their existing outer
contexts. See [history admission](principal-history-admission.md).

## Root-record byte/count admission

Limits, usage and signed-authority node preparation reserves its exact requested
layout under an independent leaf counter and leaves it before allocation or
root publication. Relinking/detachment under existing registry guards enters no
counter. Explicit unused or confirmed post-invalidation disposal frees the node
before refunding its original classification. Fallback retains storage and
charges without locks. The counter enters no root, policy, history, allocator or
other subsystem and performs no ASID lookup. Existing outer cleanup contexts
remain required. See [root metadata admission](root-metadata-admission.md).

Endpoint creation also captures its namespace under IPC, leaves before exact-root
lease acquisition, and revalidates before atomic record/slot admission. Endpoint
node, queue backing and close/readiness lists allocate outside IPC. Shared
authority storage now also prepares unlocked, count admission relinks under
IPC after captured-namespace validation, and payload allocation leaves IPC again.
Payload insertion relinks prepared nodes. Endpoint publication
itself only relinks prepared storage, and unused preparation finishes post-guard,
root last.

Queue growth retains the exact root and original endpoint account/classification,
reserves/allocates unlocked, then rechecks root, cap, endpoint and closing/backing
state before an allocation-free FIFO move. Replaced or unused backing stays in
that owner until explicit disposal outside IPC. Final references to closed,
empty, unclaimed endpoints detach complete charged nodes into the existing close
or cancellation owner; payload/node disposal leaves IPC. Fallback retains all
new fields without allocator, counter-lock or callback work. These changes add
no lifecycle-under-IPC edge or new cleanup registry. Linear scans, broader
caller masks/guards and pressure hold times remain unqualified. See
[endpoint storage](endpoint-budgets.md).

All per-capability payload nodes now prepare fallibly before shared authority
publication. Existing containing operations retain unused payload/authority
preparation through local guard exit and explicitly finish it there. Connection
counts follow node lifetime. Removal detaches payload and unified authority
metadata together; the same close/cancellation temporary lists carry both outside
IPC. Speculative receive rollback and scalar/claimed reply use these owners too.
No new lifecycle-under-IPC edge, registry or physical retry authority is added.
Other capability callers still execute those factories under IPC. See
[capability storage](ipc-capability-storage.md) for that remaining boundary and
linear traversal/outer-context limitations.

Call/endpoint `PreparingCapability` retains captured identity, shared preparation,
reservation and payload storage across these guard transitions. Call pending
waiter-list allocation now also leaves IPC, with its original family class.
Stage-call revalidates without allocating. Rejected shared admission retains
unused metadata for post-IPC finish, including provisional counts on identity
exhaustion; existing exact roots finish last. Guarded direct-grant/receive/return
contexts use the same helpers without adding an unlocked interval. The joint
memory/authority helper now iterates borrowed owners without a temporary vector.
Mixed memory payload maps still allocate under their registry and enclosing IPC. No factory or physical cleanup retry is introduced.
See [unlocked preparation](../reports/audits/2026-10-24-security-ipc-unlocked-preparation.md).

`PublicationBatch` provides stable borrowed members to one shared-authority
hold. Both validation and commit iterate under that hold; providers must preserve
membership/dispositions and perform no allocation, callbacks or subsystem work.
The memory provider chains existing additional reservations and prepared transfer
fields, with no temporary vector or batch-sized stack array. Scalar-only commit
skips `MEMORY_OBJECTS`; mixed transfer validation and later payload installation
retain that existing guard. Memory payload maps still allocate there after
authority publication. The serialized heap-held mixed control releases heap
before those payload updates; complete scalar call commit is tested with both
IPC and heap held. No owner/root/pin/charge or notification order changes. See
[publication batches](ipc-publication-batches.md).
