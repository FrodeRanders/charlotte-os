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
- **`ConcurrentQueue`** — the only mechanism for crossing IRQ context into
  thread context without locks: `DEFERRED_WAKES` and observer waitlists.
  `deliver_interrupt` takes no locks (see `scheduler-state-machines.md` §8,
  LO5).
- **`Atomic*`** — `on_cpu` byte-sized ownership handshake, generation
  counters, and `IRQ_PENDING` counts.

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
| **LO-memory-budget** | Admission checks the generation under the address-space table lock before taking the budget ledger. Ledger operations never enter IPC or the object registry. Staged frames release physical storage before dropping their charge; no allocator lock is held when the charge returns to the ledger. | `memory/budget.rs`, `memory/object.rs` (`ChargedFrames`) |
| **LO-IPC-retirement** | Lifecycle retirement fences admission and drains IPC before destroying memory attachments. IPC-locked loan revocation must not acquire the lifecycle lock. It fences the object as revoking, releases the object registry around unmap/shootdown, then removes the loan capability. Failed unmap restores the prior loan state. Direct kernel revocation uses the lifecycle-serialized wrapper. | `memory/mod.rs` (`close_user_address_space_locked`), `memory/object.rs` (`revoke_lend_under_ipc`) |
| **LO-timer-admission** | Timer budget operations take domain counter before node counter and never enter another subsystem. Platform identity is sampled before the completion registry is locked and compared with its stored generation-qualified identity. Enqueue runs after dropping the registry. Cancellation under the registry may only try the local queue guard; a busy or remote queue retains the flagged event and its charge for later purge. | `timers/budget.rs`, `timers/mod.rs` (`cancel_event`), `completion/mod.rs` |
| **LO-completion-record-admission** | Record reservation takes the completion registry, then domain counter, then node counters. Budget guards allocate nothing and enter no other subsystem. Platform identity is sampled before the registry; retirement is checked against its stored generation. A retained completion owns its charge after cap closure; a detached result transfers it into the CQ backlog until delivery/discard. | `completion/budget.rs`, `completion/mod.rs` (`reserve_record`, `complete_detached`, `replace_cq`) |
| **LO-completion-queue-admission** | CQ staging takes the completion registry, then domain and node counters. Counter guards neither allocate nor enter another subsystem. Backing allocation and physical-ring initialization follow reservation; failure preserves the old registry entry. Loader rollback enters lifecycle teardown only after CQ setup releases the registry. Platform identity is sampled before the registry and matched to its captured generation. | `completion/cq_budget.rs`, `completion/mod.rs` (`stage_cq`, `attach_cq`), `service/loader.rs` (`PreparingDomain`) |
| **LO-close-watch-registration** | Staging reserves under the completion registry, then watch domain/node counters, and releases that registry before IPC registration. IPC may enter the independent list lock; token Drop may enter it from completion teardown, but the list never enters either registry. Entry allocation precedes the list lock, removed entries are dropped after it, and detached callbacks run only after IPC/list locks are released. Rollback validates the exact staged object. | `klib/observer/registration.rs`, `completion/watch_budget.rs`, `completion/mod.rs` (`EventSubmission`), `ipc/mod.rs` (`watch_connection_closed`, `close_cap`) |
| **LO-scheduler-waiter-registration** | Capture the sponsor at Thread construction via address-space table → memory ledger. Parking follows LP scheduler (Ready only) → master thread table → source registry (CQ/IPC sources) → independent waiter counters/list. Counters use domain → node and never enter a subsystem; token Drop enters only the list and drops removed charges after its guard. Waker cancellation on Ready/reap cannot invoke callbacks. IPC/CQ source notifications release registry/list locks before scheduler callbacks; combined detached batches do not allocate. IPC prepares waiter lists before transferring call attachments. Retire/promotion may enter waiter counters from the memory ledger, never the reverse. | `klib/observer/{mod,registration,waiter_budget}.rs`, `memory/budget.rs`, scheduler, `completion/mod.rs`, `ipc/mod.rs` |
| **LO-blocking-lock-waiters** | First contention may allocate a list under the independent initialization spin guard; release it before entry reservation/allocation and registration. Token Drop enters only the list. Release data ownership and detach every selected list before callbacks. Mask local IRQs across park/lost-wake recheck so unlock-before-registration cannot strand a preempted Blocked caller. No initialization/list/scheduler guard or local mask crosses yield. Final-reader RwLock release broadcasts both classes, without ownership handoff. | `klib/observer/waiter_source.rs`, `cpu/scheduler/sync/{mutex,rwlock}/mod.rs` |
| **LO-timer-observers** | Timer scheduler registrations use the same independent owning source. Raw timer callbacks occupy one embedded weak slot. Detach both forms before callbacks, releasing their guards but retaining the LP-local queue borrow; callbacks must not re-enter that queue. Sleep masks park/rebase/enqueue and drops the mask before yielding. On rejected admission it discards the unqueued event and waits runnable to the deadline. | `timers/mod.rs`, `cpu/scheduler/mod.rs` (`sleep_with_event`) |
| **LO-scheduler-timer-preparation** | Capture the timer sponsor at Thread construction through address-space table → memory ledger. Clone it under the master table, then release that guard before domain → shared-node event counters. Reserve/prepare watchdog callback, cancellation state and fixed-size node before parking; all fallible preparation precedes Blocked. Anonymous insertion allocates nothing. Completion-node staging does not borrow a queue under its registry; enqueue follows registry release. The inline quantum slot consumes no anonymous-node charge. Publication updates cancellation's shared owner LP under the enqueue mask. | `timers/{budget,queue,mod}.rs`, `memory/budget.rs`, scheduler, completion and syscall timed waits |
| **LO-endpoint-admission** | Endpoint budgets take domain counter before node counter; neither budget guard enters another subsystem or allocates. Generation/platform identity is sampled before IPC, and retirement is checked under IPC before creation or growth. Queue allocation runs after dropping budget guards; staged old/new backing both retain charges. Teardown marks retirement without holding the address-space table or budget ledger while entering IPC. | `ipc/budget.rs`, `ipc/mod.rs` (`endpoint_create`, `endpoint_resize`, `close_cap`) |
| **LO-IPC-record-admission** | Under IPC, revalidate namespace generation/retirement and kernel platform identity through the address-space table and memory ledger, releasing those guards before domain → node record counters. Counters allocate nothing and enter no subsystem. Reserve call/reply records before waiter allocation or attachment transfer; admit delegated/returned connections before transfer or loan revocation. IPC retirement fences receive and connection publication before teardown's capability snapshot. | `ipc/record_budget.rs`, `ipc/mod.rs` (`accepting_namespace`, `stage_call`, `complete_reply`, `close_address_space`) |
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
