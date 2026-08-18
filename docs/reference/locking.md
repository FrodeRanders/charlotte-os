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
| Interrupt-masking spin `Mutex` | `cpu/multiprocessor/spin/mutex.rs` | Masks IRQs for the whole critical section | No (spin) | Frame allocator, memory-object registry, address-space table, kernel AS, domain authorities, AS lifecycle, scratch window, talc |
| Interrupt-masking spin `RwLock` | `cpu/multiprocessor/spin/rwlock.rs` | Masks IRQs; nesting-aware save/restore | No (spin) | `MASTER_THREAD_TABLE`, `DEAD_THREADS`, `SYSTEM_SCHEDULER`, `IPC`, `COMPLETIONS`, `USER_MAILBOX_CAPS`, each `PerLp` slot |
| External `spin` crate | `spin::{Mutex, RwLock, LazyLock}` | None (manual, see §3) | No (spin) | Stack allocator arena + guard-page map; one-time lazy init everywhere |
| talc global-allocator lock | `TalcLock<MutexCore, ExtendOnOom>` in `memory/allocators/global_allocator.rs` | Masks IRQs (reuses the spin `MutexCore`) | No (spin) | Kernel heap |
| Lock-free containers | `ShardLocal`, `concurrent_queue::ConcurrentQueue`, `Atomic*` | n/a | n/a | Per-LP state, IRQ→thread deferred-wake handoff, generation counters, `on_cpu` |

There is also a **scheduler-blocking** lock family under
`cpu/scheduler/sync/{mutex,rwlock}` that parks the caller via `block_thread`
instead of spinning. As of the last audit it has **no callers**; it is
aspirational infrastructure, not a live constraint. Do not document it as part
of any ordering guarantee until a consumer exists.

---

## 2. Interrupt-masking spin locks

### 2.1 `Mutex` (`spin/mutex.rs`)

A `lock_api::RawMutex` (`GuardSend`) that saves the caller's interrupt-enable
bit, masks maskable IRQs, spins on an `AtomicBool`, and restores the saved bit
on unlock. Why it masks IRQs is explained in `memory/mod.rs:19-24`: the locks
it guards are taken from both preemptible kernel threads and synchronous EL0
exception paths. If the owner could be timer-preempted, every LP could end up
spinning for a lock whose owner can never be scheduled again.

Properties:

- **Non-reentrant**: re-acquiring the *same* lock on one LP deadlocks.
- **Nesting across *different* locks works**: each lock records its own
  pre-acquire interrupt state, so a nested acquire on another `Mutex` sees
  "already masked" and correctly defers unmasking to the outermost unlock.
- The saved interrupt flag lives on the lock object itself, not per-LP; it is
  correct only because IRQs are masked for the entire ownership interval.

Users (all via `memory::Mutex`): `PHYSICAL_FRAME_ALLOCATOR`,
`MEMORY_OBJECTS`, `ADDRESS_SPACE_TABLE`, `KERNEL_AS`,
`DOMAIN_AUTHORITIES`, `ADDRESS_SPACE_LIFECYCLE`, `SCRATCH_WINDOW_NEXT`, and
the talc lock.

### 2.2 `RwLock` (`spin/rwlock.rs`)

A `lock_api::RawRwLock` (`GuardSend`) over an `AtomicI64` reader count with
`-1` for a writer, plus a `waiting_writers` counter that gives **writer
preference**: once a writer queues, new readers defer so a dense reader stream
cannot starve the writer (documented in `rwlock.rs:53-56`).

Instead of a per-lock saved flag, it uses the shared per-LP
[`INT_STATE`](#3-interrupt-masking-discipline) save/restore, which is
**nesting-aware** (a per-LP save count). This lets a thread take several
different `RwLock`s, or a read-after-read, without prematurely re-enabling
IRQs. Re-acquiring the same lock for writing is still a deadlock.

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
| **LO-noblock-under-lock** | Never call `block_thread`/`yield_lp`/`cond_yield_lp` while holding any lock. A spin lock additionally masks IRQs; parking the thread would abandon the lock and the LP cannot schedule its successor. All guards are dropped before `switch_ctx` (`scheduler-state-machines.md` LO4). | `scheduler-state-machines.md:372-375` |
| **LO-irq** | IRQ context takes no locks and never blocks. | `scheduler-state-machines.md` LO5 |

The interrupt-masking spin locks and the scheduler's `block_thread` path are
compatible only because blocking never happens under a held spin lock: a
blocking syscall releases its registry guards before registering a waker and
yielding.

---

## 6. Choosing a primitive

| Need | Primitive |
|---|---|
| Shared state touched from syscall and/or IRQ context | Interrupt-masking spin `Mutex` (single writer) or `RwLock` (read-mostly) |
| Data read from several LPs, written rarely | Interrupt-masking spin `RwLock` (writer preference prevents reader starvation) |
| Strictly per-LP data | `ShardLocal` (lock-free) or `PerLp` (if ISR/cross-LP reach is needed) |
| IRQ → thread handoff | `ConcurrentQueue` + atomics, drained in thread context |
| Cross-LP mutation | IPI/closure dispatch, never direct shared-memory writes |
| One-time global initialization | `spin::LazyLock` |
