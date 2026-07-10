# CharlotteOS Async Syscall / Completion-Capability ABI

> Phase 2, Option C of the sitas ↔ CharlotteOS collaboration (see
> [`sitas-runtime-model.md`](./sitas-runtime-model.md) §3, §5.4, §9a). This note
> specifies the userspace↔kernel boundary for asynchronous system calls by
> treating **sitas as the executable specification** of the ABI: if sitas's
> shard model, completion futures, and cross-shard submit map cleanly onto the
> interface described here, the interface is right.
>
> Status: exploratory design ("Option C on paper"). No syscall path exists in
> the kernel yet (`sync_dispatcher` panics on SVC; there is no x86_64 syscall
> handler; there is no capability table). This document defines the ABI that the
> future implementation should target and records the decision-gate answers for
> Phase 2. The Rust in §9 is a *type sketch*, not compiled kernel code.

---

## 1. The thesis, stated as an ABI question

CharlotteOS's design premise is that *nearly all syscalls are asynchronous*: a
syscall **submits** an operation and returns a **completion capability** that
userspace can wait on, rather than blocking the calling thread until the work is
done. This intent is already written into the kernel — see the exit-observer
comment on `Thread` in
`crates/catten/src/cpu/scheduler/threads/mod.rs:63-69`:

> "the completion capability returned from the system call would be registered
> as an observer of the thread that is executing the work whose completion it
> represents so that userspace software can monitor it in real time using the
> same mechanism the kernel would use."

sitas asks the *same* question from userspace: *what is the right
submission/completion boundary for async I/O?* Its answer — visible in the
`os-backend-seam` branch — is a three-method reactor contract plus an
owned-buffer completion model. Option C is the observation that **sitas's
reactor contract is the shape of CharlotteOS's syscall ABI**, and its io_uring
buffer discipline is the shape of the kernel's buffer-ownership contract.

So we design the ABI by making sitas's `ReactorBackend`, `Reply<T>`, and
`ShardedSubmitter` implementable against it as a thin *translation* (not an
emulation).

---

## 2. The reference consumer: what sitas actually demands

From Phase 1 (Option A) we have a precise, minimal statement of what a
shard-per-core runtime needs from the OS. This is the specification the ABI must
satisfy.

### 2.1 The reactor contract (`sitas/src/reactor_backend.rs`)

```rust
pub trait ReactorBackend {
    type Waker: ReactorWaker;
    type Handle: Copy + Eq;                 // "what is a waitable interest?"
    type Event: ReactorEvent<Handle = Self::Handle>;

    fn waker(&self) -> Self::Waker;
    fn wait(&self, read: &[Self::Handle], write: &[Self::Handle],
            timeout: Option<Duration>) -> io::Result<Self::Event>;
}

pub trait ReactorWaker: Clone + Send + Sync { fn wake(&self) -> io::Result<()>; }

pub trait ReactorEvent {
    type Handle;
    fn woke(&self) -> bool;
    fn readable(&self) -> &[Self::Handle];
    fn writable(&self) -> &[Self::Handle];
}
```

Reading `executor::driver`, the executor's idle path needs exactly three
capabilities from the OS:

1. **Block until something happens**, with a deadline — `wait(...)`.
2. **Wake a blocked reactor** from another shard/core — `ReactorWaker::wake`.
3. **Submit/complete async operations** — currently a *second* wait source
   (io_uring), not yet folded into `wait`.

### 2.2 The sharpest finding from Phase 1

> *Abstracting "block until an event" was trivial; abstracting "what is an
> interest" is the actual work.*

On Unix, `ReactorBackend::Handle` is pinned to `RawFd` because the readiness
layer (`io_interest`, `unix_io`, the io_uring completion fd — ~33 `RawFd` uses)
identifies every interest by file descriptor. The associated `Handle` type is
where the design pressure concentrates. **For CharlotteOS, `Handle` is the
completion capability.** Getting the completion capability right *is* getting
the ABI right. Everything else (the wait primitive, the waker) is comparatively
trivial and already exists in the kernel.

### 2.3 The completion + buffer-ownership model (`sitas` io_uring path)

sitas's owned-buffer futures give us the cancellation/reclaim contract the
kernel must honor:

- `read_at(fd, offset, buffer: Vec<u8>) -> future`; on completion the buffer is
  returned with the byte count.
- **Drop is cancellation.** `IoUringReadFuture::drop` calls
  `defer_buffer_drop(op, buffer)`, which issues a cancel and *retains the
  buffer* (keyed by operation id) until the completion is observed — so the
  kernel's DMA target stays alive.
- **Drain-or-leak teardown.** On shutdown, if live wakers remain the buffers
  belong to live futures; otherwise a bounded drain runs, and any buffer still
  owed to the kernel on timeout is `mem::forget`'d rather than freed. The rule:
  *never free a buffer while the kernel may still touch it.*

This is not Unix-specific; it is an ownership contract. The kernel side of the
ABI must expose exactly this: **submit an owned buffer, get a completion
capability, reclaim the buffer only on observed completion or acknowledged
cancel.**

### 2.4 The shard/submit layer (`ShardedSubmitter`, `ShardSender<M>`)

- A **shard** is a thread pinned to a core with a private executor.
- `ShardedSubmitter::submit_to(shard_id, future)` submits work to *another*
  shard and returns a `JoinHandle` (an awaitable completion).
- `ShardSender<M>`/`ShardReceiver<M>` are bounded, owned-message channels
  between shards.

These map onto CharlotteOS's per-LP scheduler + `IPI_CMD_QUEUES` + IPI wake.

---

## 3. The core abstraction: the completion capability

A **completion capability** (`CompletionCap`) is the CharlotteOS realization of
sitas's `ReactorBackend::Handle`. It is:

> a kernel-managed, per-address-space handle naming an in-flight (or completed)
> asynchronous operation, that a userspace task can register interest in and
> wait on, and whose signalling reuses the kernel's existing
> `Observable`/`Observer` mechanism.

It fuses two things the kernel already has:

| Ingredient | Exists today | File |
|---|---|---|
| An `Observable` whose completion notifies observers | `Thread: Observable`, `TimerEvent: Observable` | `scheduler/threads/mod.rs:70`, `timers/mod.rs:64` |
| An `Observer` that wakes a blocked thread | `Waker(ThreadId): Observer` → `submit_ready_thread` | `scheduler/threads/waker.rs:16` |
| The block-on-observable pattern | `SystemScheduler::block_thread(tid, &dyn Observable)` | `system_scheduler/mod.rs:70` |

Today `block_thread` only ever blocks on a `TimerEvent` (via `sleep`). The ABI
generalizes "block on a timer event" to "block on any completion capability,"
which is the identical mechanism with a different `Observable` source.

### 3.1 What is *new* (must be built)

1. **A capability table** — per address space, mapping a small integer
   `CompletionCap` (the value crossing the syscall boundary) to a kernel object
   implementing `Observable`. None exists today (the only "capability" in-tree is
   PCIe device capabilities).
2. **A syscall entry/dispatch path** — `sync_dispatcher` panics on SVC; x86_64
   has no `SYSCALL`/`LSTAR` handler. This must decode the syscall and route to a
   dispatcher.
3. **A per-shard completion queue** — the userspace-visible ring that reports
   which capabilities have completed (the concrete backing for
   `ReactorEvent::readable`).

### 3.2 What is *reused* (already present)

Per-LP scheduling (`SYSTEM_SCHEDULER`, one `LpScheduler` per LP), the
observer/waker wake path, per-LP timer queues, the AArch64
`irq_dispatcher → drain_local_ipi_queue → cond_yield_lp` path, and
`IPI_CMD_QUEUES` for cross-LP delivery.

---

## 4. The ABI surface

Five operations. Everything sitas needs is expressible in terms of them.

### 4.1 `submit` — start an async operation

```
submit(op: OpCode, args: OpArgs, buffers: BufferSet) -> Result<CompletionCap, SubmitError>
```

- Returns *immediately* with a `CompletionCap` naming the in-flight operation.
  The kernel spawns/schedules the work (e.g. `spawn_thread(asid, worker)` for
  operations that run as a worker thread) and registers the returned
  capability's internal `Observable` against that work's completion — exactly
  the exit-observer pattern in `threads/mod.rs:63-69`.
- `buffers` transfers **ownership** of userspace buffers to the kernel for the
  operation's duration (see §5, the reclaim contract). This is the direct analog
  of sitas passing `Vec<u8>` into `read_at`.
- `SubmitError::WouldBlock` is the backpressure signal (§6): the submission
  queue / capability table is full.

`submit` is the kernel-side of sitas's `submit_* -> Reply<T>` and of
`ShardedSubmitter::submit_with_handle_to`. The `CompletionCap` *is* the
`Reply<T>` / `JoinHandle`.

### 4.2 `wait` — the reactor's only sleep

```
wait(cq: CompletionQueueId, min_complete: u32, deadline: Option<Timestamp>)
    -> Result<u32, WaitError>
```

- Blocks the calling (shard) thread until at least `min_complete` completions
  are available on its completion queue, or a cross-LP wake arrives
  (`min_complete = 0` returns on any event), or `deadline` elapses.
- Returns the number of completion entries now readable in the shard's CQ; the
  caller drains them from shared memory (no per-completion syscall).
- Implemented by `block_thread(tid, &completion_queue_observable)` — the CQ is
  an `Observable`; each arriving completion and each wake `notify()`s it. On
  wake the thread is resubmitted to its LP via the existing `Waker` path.

This is precisely sitas's `ReactorBackend::wait(read, write, timeout) -> Event`.
The mapping:

| sitas `wait` | CharlotteOS `wait` |
|---|---|
| `read`/`write: &[Handle]` (fds to watch) | *not needed* — interests are registered at `submit` time and reported via the CQ; the shard watches one CQ, not N handles |
| `timeout: Option<Duration>` | `deadline: Option<Timestamp>` |
| returns `Event { woke, readable[], writable[] }` | returns count; CQ entries carry `{ cap, result }` |
| `woke` (wake pipe drained) | a wake IPI raised the CQ observable with no completion payload |

Note the structural improvement: sitas must *hand the OS the fd set on every
wait* because readiness is edge/level-triggered per fd. CharlotteOS registers
the interest once at `submit` and the completion is a one-shot event delivered to
the owning shard's CQ. This is why the ABI can present **one** wait source (§2.1
item 3 folds into item 1) — timers, cross-LP wakes, and async-syscall
completions all land in the same CQ, resolving sitas's "two wait sources →
one" roadmap item natively (`sitas-runtime-model.md` §7.5).

### 4.3 `wake` — cross-shard wake

```
wake(cq: CompletionQueueId) -> Result<(), WakeError>
```

- Signals another shard's completion queue, unblocking its `wait`. Implemented
  as `send_ipi(target_lp)` landing in the target's `irq_dispatcher`, which
  drains and `cond_yield_lp`s — the path already wired on AArch64
  (`aarch64/interrupts/mod.rs:51`).
- This is sitas's `ReactorWaker::wake`. The `ReactorWaker` is `Clone + Send +
  Sync`; a `CompletionQueueId` (a small copyable capability) satisfies that.

### 4.4 `cancel` — drop-as-cancellation

```
cancel(cap: CompletionCap) -> Result<CancelState, CancelError>

enum CancelState { AlreadyComplete, CancelRequested }
```

- Requests cancellation of the in-flight operation. Mirrors sitas's
  `abandon_operation`: the kernel stops waking on the original op and, if
  buffers were transferred, **retains them until a terminal completion is
  observed** (§5). Userspace calls this from a future's `Drop`.
- `CancelRequested` means a completion (success, error, or `Cancelled`) will
  still be posted to the CQ; userspace must observe it before reclaiming
  buffers, or use the leak fallback (§5.3).

### 4.5 `reclaim` / capability lifecycle

```
close(cap: CompletionCap) -> Result<(), CapError>   // release a completed/observed capability
```

- Frees the capability-table slot once its completion has been observed (or its
  buffers reclaimed). Analogous to dropping a `Reply<T>` after `wait`.

---

## 5. The buffer-ownership / reclaim contract

Lifted verbatim from sitas's io_uring discipline, because it is an ownership
rule, not a Linux detail.

1. **Transfer on submit.** `submit` moves buffer ownership to the kernel. While
   an operation is in flight, userspace must not read/write or free those pages.
   (Enforcement options: the kernel pins/maps the pages; a debug build can poison
   them. Open question, §8.)
2. **Return on completion.** The CQ entry for a completed op carries back the
   buffer identity + result (bytes transferred), handing ownership back — the
   analog of `WriteAtUringCompletion { bytes, buffer }`.
3. **Cancel is deferred-reclaim.** After `cancel`, the kernel may still touch the
   buffer until it posts the terminal completion. Userspace reclaims only then.
   This is sitas's `defer_buffer_drop` on the kernel side.
4. **Drain-or-leak at teardown.** When an address space exits with operations
   still in flight, the kernel drains outstanding completions; any buffer it may
   still DMA into is *retained (leaked into the dying AS's reaping), never handed
   to a new owner*. This is the kernel mirror of sitas's `mem::forget`-on-timeout
   rule. Because the AS is being torn down, "leak" means "reap with the AS," so
   there is no permanent loss.

This contract is the answer to `sitas-runtime-model.md` §7.2 ("what happens to an
in-flight async syscall when the awaiting task is dropped?"): **the kernel needs
a cancel + deferred-reclaim contract mirroring sitas's drain-or-leak,** and here
it is, specified by the reference consumer.

---

## 6. Backpressure, end to end

sitas has bounded mailboxes and a spawn `BackpressureGuard`; the ABI must carry
backpressure across the boundary (`sitas-runtime-model.md` §7.3).

- **Submission backpressure.** Each shard's submission queue and the per-AS
  capability table are **bounded**. `submit` returns `SubmitError::WouldBlock`
  when full; this is a first-class, synchronous, non-fatal result — the analog of
  `ShardSender::try_send` returning `Full`. Userspace can await CQ space (its own
  completion queue draining) before retrying.
- **Completion backpressure.** The per-shard CQ is bounded. If it is full, the
  kernel does **not** drop completions; it stops producing new ones for that
  shard (applying backpressure upstream to whatever generates them) and marks the
  CQ *overflow-pending*, so the next `wait` reports that draining is required.
  This mirrors sitas's bounded mailbox semantics (`full_rejections` counter)
  rather than a lossy queue.
- **Cross-shard submit backpressure.** `ShardedSubmitter::submit_to(other)` maps
  to enqueuing on the target LP's queue (generalized `IPI_CMD_QUEUES`). Today
  that queue is *unbounded* (`ConcurrentQueue<IpiRpc>`); the ABI requires it to
  become **bounded per target shard**, so a heavily-targeted inbox exerts
  backpressure instead of growing without limit — matching sitas's bounded
  `ShardSender<M>` and the cache-coherence analysis in sitas's `ARCHITECTURE.md`.

---

## 7. The shard model on the ABI

| sitas concept | ABI realization | Kernel facility |
|---|---|---|
| Shard = pinned OS thread | LP-affine thread, one per LP, each owning a CQ | `spawn_thread(asid, entry)`; scheduler is already per-LP |
| `ShardId` | `CompletionQueueId` / `LpId` | `LpId` (an LP *is* a core — placement is not advisory) |
| Executor idle `wait` | `wait(cq, ...)` | `block_thread` on the CQ observable |
| `ReactorWaker::wake` | `wake(cq)` | `send_ipi(target_lp)` → `irq_dispatcher` → `cond_yield_lp` |
| `submit_* -> Reply<T>` | `submit(...) -> CompletionCap` | `spawn_thread` + register cap as exit-observer |
| `Reply<T>::wait_async` (`ReplyFuture`) | await a `CompletionCap` via the CQ | one-shot CQ entry `{ cap, result }` |
| `ShardedSubmitter::submit_to` | enqueue on target LP + `wake` | generalized bounded `IPI_CMD_QUEUES` |
| `ShardSender<M>`/`ShardReceiver<M>` | bounded typed capability channel | generalized `IpiRpc` → typed `M` (feeds Option B) |
| Drop = cancel | `cancel(cap)` + deferred reclaim | new; specified by §4.4/§5 |

### 7.1 Assembled picture (the CharlotteOS `ReactorBackend`)

```
sitas-charlotte backend (implements sitas's ReactorBackend + ShardRuntime)
 ├─ Handle          = CompletionCap                     (the Phase-1 "what is an interest" answer)
 ├─ Waker           = CompletionQueueId                 → wake() = wake(cq) syscall
 ├─ wait(_, _, to)  = wait(own_cq, min_complete, deadline) → drain CQ → Event
 ├─ submit read/write = submit(op, args, buffers) -> CompletionCap
 │                       (buffer ownership transferred; returned on completion)
 └─ spawn_shard     = spawn_thread(asid) pinned to Lp; create its CQ
    channel<M>      = bounded typed capability channel (generalized IPI queue)
```

Everything above the backend is sitas unchanged. The backend is a *translation*:
no blocking call is faked, no thread is parked to simulate a completion, no
signal-handler tightrope — because the kernel's async model already matches
sitas's.

---

## 8. Decision-gate answers (Phase 2)

The §9a gate asks: *Do sitas's shard model, completion futures, and cross-shard
submit map cleanly onto the ABI? Are cancellation and backpressure expressible?*

1. **Shard model — yes.** A sitas shard = an LP-affine thread with a private CQ.
   Placement is *stronger* than Unix (an LP is a core; no `sched_setaffinity`
   race). `ShardRuntime::spawn_shard` → `spawn_thread` + CQ creation.
2. **Completion futures — yes, and more naturally than on Unix.** `Reply<T>` /
   `ReplyFuture<T>` / `JoinHandle` all reduce to "await one `CompletionCap` via
   the CQ." The kernel already stores a `Waker` as an `Observer` and wakes the
   blocked thread; sitas already stores a `Waker` in `ReplyShared`. Same object,
   different name. The one-shot completion event is a *better* fit than fd
   readiness (no re-arm, no level/edge nuance), and it unifies sitas's two wait
   sources into one CQ.
3. **Cross-shard submit — yes.** `submit_to(other)` → enqueue on target LP's
   (bounded) queue + `wake`. The kernel already does exactly this for its own
   TLB-shootdown RPCs over `IPI_CMD_QUEUES`; generalizing the message type and
   bounding the queue is the delta.
4. **Cancellation — yes, and the reference consumer *specifies* it.** sitas's
   drop-as-cancel + `defer_buffer_drop` + drain-or-leak translate directly into
   §4.4/§5. This turns an open kernel question into a settled contract.
5. **Backpressure — expressible, with one required kernel change.** Bounded SQ,
   bounded CQ (overflow-pending, non-lossy), and — the change — **bounding the
   per-LP cross-shard queue** (today unbounded). With that, backpressure is
   end-to-end, matching sitas's bounded mailboxes.

**Where the design pressure really is (Phase 1's finding, confirmed):** not the
wait primitive (trivial, already in-kernel) but *what a waitable interest is*.
The ABI's center of gravity is the **completion capability + its table + the
buffer-ownership contract**. That is the thing to prototype first in Option C.

### 8.1 Open items carried forward

- **Enforcing buffer transfer.** How strictly does the kernel prevent userspace
  touching a transferred buffer (page remap vs. trust vs. debug poison)? Affects
  the DMA-safety strength of §5.
- **CQ/SQ memory model.** Shared-memory rings (io_uring-style, mapped into the
  AS) vs. syscall-per-drain. Rings are assumed above for zero-syscall draining;
  needs an mmap-like facility.
- **Interrupting user code for upcalls.** The async-first model must deliver
  wakes into a running userspace shard safely; sitas's executor is
  cooperative-only (no in-shard preemption). Where wakes are *observed* (at the
  next `wait`, never mid-task) must be nailed down (`sitas-runtime-model.md`
  §7.4). The CQ model helps: a wake need not interrupt user code, it only needs
  to be visible at the next `wait`.
- **Prerequisites that do not exist yet:** syscall entry/dispatch (both ISAs),
  a per-AS capability table, and a userspace-mappable CQ/SQ. x86_64 IPI handlers
  are also stubbed (`todo!()`), so the first runnable prototype targets AArch64.

---

## 9. In-tree Rust type sketch (non-compiled)

A concrete rendering of the ABI as Rust types, to pin down shapes. This is
illustrative — it does not compile into the kernel and commits no ABI numbers.

```rust
//! Async syscall / completion-capability ABI — type sketch (Option C).
//! Specified against sitas's `ReactorBackend` as the reference consumer.

/// A per-address-space handle naming an in-flight or completed async operation.
/// This is the CharlotteOS answer to sitas's `ReactorBackend::Handle`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CompletionCap(u32);

/// Identifies a shard's completion queue; also the cross-shard wake target.
/// Satisfies sitas's `ReactorWaker: Clone + Send + Sync`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CompletionQueueId(u32);

/// One entry drained from a shard's completion queue.
pub struct Completion {
    pub cap: CompletionCap,
    pub result: OpResult,
    /// Buffer ownership handed back to userspace (identity + bytes), mirroring
    /// sitas's `WriteAtUringCompletion { bytes, buffer }`.
    pub returned: Option<BufferSet>,
}

pub enum OpResult {
    Ok(i64),          // e.g. bytes transferred
    Err(ErrorCode),
    Cancelled,        // terminal result after `cancel`
}

// ---- the five ABI operations -------------------------------------------------

/// Start an async operation. Returns immediately with a capability naming it.
/// Ownership of `buffers` transfers to the kernel until a terminal completion.
pub fn submit(op: OpCode, args: OpArgs, buffers: BufferSet)
    -> Result<CompletionCap, SubmitError>;

/// The reactor's only sleep. Blocks the shard thread until >= `min_complete`
/// completions are ready on `cq`, a wake arrives, or `deadline` elapses.
/// Returns how many entries are now drainable from the (shared-memory) CQ.
pub fn wait(cq: CompletionQueueId, min_complete: u32, deadline: Option<Timestamp>)
    -> Result<u32, WaitError>;

/// Cross-shard wake: unblock another shard's `wait`. (send_ipi under the hood.)
pub fn wake(cq: CompletionQueueId) -> Result<(), WakeError>;

/// Drop-as-cancellation. Kernel retains transferred buffers until it posts the
/// terminal completion (deferred reclaim), mirroring sitas's abandon path.
pub fn cancel(cap: CompletionCap) -> Result<CancelState, CancelError>;

/// Release a completed/observed capability slot.
pub fn close(cap: CompletionCap) -> Result<(), CapError>;

pub enum SubmitError { WouldBlock, BadArgs, NoBuffers /* ... */ } // WouldBlock = backpressure
pub enum CancelState { AlreadyComplete, CancelRequested }

// ---- how the sitas backend binds to it --------------------------------------

/// Sketch of the CharlotteOS impl of sitas's `ReactorBackend`.
struct CharlotteReactor { cq: CompletionQueueId }

impl ReactorBackend for CharlotteReactor {
    type Waker  = CompletionQueueId;       // wake() -> wake(cq)
    type Handle = CompletionCap;           // the Phase-1 "what is an interest" answer
    type Event  = CharlotteEvent;

    fn waker(&self) -> CompletionQueueId { self.cq }

    fn wait(&self, _read: &[CompletionCap], _write: &[CompletionCap],
            timeout: Option<Duration>) -> io::Result<CharlotteEvent> {
        // Interests are registered at submit-time, not passed here; the shard
        // watches its single CQ. Drain into an owned Event after waking.
        let n = wait(self.cq, 0, timeout.map(deadline_from))?;
        Ok(drain_cq(self.cq, n))
    }
}
```

The key line is `type Handle = CompletionCap`: the abstraction sitas identified
in Phase 1 ("what is a waitable interest?") is answered by a kernel-managed
completion capability rather than a Unix file descriptor — and the rest of the
backend falls out as a translation.

### 9.1 Validation status: the ABI has an executable model

The type sketch above is no longer only on paper. A **reference model of this
ABI now exists and is tested in the sitas repository** (branch
`reactor-handle-seam`, `src/charlotte_abi.rs`): an in-memory `MockKernel`
implementing the five operations (`submit`/`wait`/`wake`/`cancel`/`close`), plus
a `CharlotteReactor` that implements sitas's `ReactorBackend` contract with
`Handle = CompletionCap`. It is not a Unix backend and talks to no kernel — it
is the ABI's semantics made executable, so the shapes can be exercised before
any kernel path exists.

Tests validate each decision-gate claim concretely:

- **Completion path:** `submit → complete → wait` reports the ready capability
  and drains a completion that hands the owned buffer back (the `Reply<T>` /
  `WriteAtUringCompletion` shape).
- **Cross-shard wake:** a blocked reactor is unblocked by a cloned waker from
  another thread (the model's stand-in for a cross-LP IPI).
- **Cancellation / deferred reclaim:** `cancel` on an in-flight op returns
  `CancelRequested` and the kernel *keeps* the buffer until the terminal
  (`Cancelled`) completion hands it back — sitas's `defer_buffer_drop`, mirrored.
- **Backpressure:** a full capability table returns `SubmitError::WouldBlock`;
  a full completion queue refuses the post **non-lossily** (op stays in flight,
  keeps its buffer, queue marked overflow-pending) until a drain frees space.
- **Contract fit:** the reactor is driven purely through
  `ReactorBackend<Handle = CompletionCap>`, proving the executor could rely on it
  exactly as it relies on `OsReactor` today.

This confirms the Phase-2 finding empirically: the wait primitive is trivial and
the design weight is entirely on *what a waitable interest is* — here, a
`CompletionCap` backed by a capability table + buffer-ownership contract.

### 9.2 Kernel prototype: the `completion` module

The submission side of the ABI now exists **in the kernel** as
`crates/catten/src/completion/mod.rs`, exercised by boot-time self-tests in
`crates/catten/src/self_test/completion.rs`. It is built entirely on facilities
that already existed:

- a per-address-space **capability table** is an
  `IdTable<Arc<Completion>>` keyed by `AddressSpaceId` — the same `IdTable` that
  backs the thread and address-space tables;
- a `Completion` is `Observable`: a waiting thread registers its `Waker` as an
  `Observer` (via `SystemScheduler::block_thread`) and `complete()` notifies
  them — byte-for-byte the mechanism `TimerEvent`/`sleep` already use;
- an owned `Vec<u8>` transferred on `submit` is retained until a terminal
  completion hands it back (buffer ownership / deferred reclaim).

The five operations are present as kernel-internal functions: `submit → cap`,
`complete(cap, result)` (the kernel-side hook a worker's exit-observer would
call), `poll` (non-blocking drain), `wait` (blocks via `block_thread`, with a
lost-wake re-check), `cancel`, `close`, plus an `observe` hook mirroring "monitor
a capability with the same mechanism the kernel uses."

**What the self-tests validate at boot** (whitebox integration tests, the kernel
has no `cargo test`): buffer transfer on submit; the observer-signal path firing
on completion; the buffer handed back on `poll`; `close` freeing the slot; cancel
retaining the buffer until the terminal `Cancelled` completion (deferred
reclaim); and `submit` returning `WouldBlock` at the capacity bound (submission
backpressure).

**What is deliberately *not* here yet** (honest scoping): `wait` is implemented
but not exercised in self-tests because it blocks the calling thread, and
self-tests run before the BSP yields to the scheduler; there is no real-EL0
test thread (the dispatch path is exercised via synthetic `TrapFrame` — §9.3);
and there is no shared-memory CQ/SQ ring — completion is delivered via the
observer wake, not yet a userspace-visible queue. The module builds and links
cleanly for both `aarch64-unknown-none-catten` and `x86_64-unknown-none-catten`
(with the `display` feature off, which has an unrelated pre-existing host-link
issue), and adds no new warnings.

### 9.3 Syscall entry prototype: `sync_dispatcher` no longer panics on SVC

The AArch64 `sync_dispatcher` now decodes `ESR_EL1.EC` and routes SVC
instructions to a syscall dispatch table (`crates/catten/src/syscall/mod.rs`).
When an SVC is taken from EL0 (EC = 0x15):

1. The raw volatile registers saved on the kernel stack by `push_volatile_regs`
   (x0–x18, 19 u64 values) are read into a `TrapFrame`, together with the
   architectural state (`SPSR_EL1`, `SP_EL0`, the exception link register).
2. `ELR_EL1` is advanced by 4 (past the 4-byte SVC instruction) and written
   back so the IVT tail `eret` returns to the following instruction.
3. The SVC immediate from `ESR_EL1.ISS` is used as the syscall number; a match
   arm routes it to the corresponding handler in the dispatch table.
4. The syscall handler reads arguments from `frame.regs[0..]` (x0–x7 in the
   AArch64 calling convention) and calls the completion-cap operations.
5. Any result the handler writes into `frame.regs[0]` (x0) is written back to
   the stack before the IVT `pop_volatile_regs` restores it into the user's x0.

The dispatch table currently maps SVC #0 (LOG, a debug/test placeholder) and
SVC #1–6 (the five completion-cap operations) to handler functions.
Self-tests in `crates/catten/src/self_test/syscall.rs` exercise every dispatch
route by calling `syscall_dispatch` directly with a synthetic `TrapFrame`,
verifying that the completion-cap operations (submit/complete/poll/cancel/close)
are reachable through the syscall table without panicking.

**What is not here yet:** a real-EL0 test thread that executes actual `SVC #n`
instructions. That requires mapping a page with user access (`AP_EL0`) in a
user address space, writing a small assembly stub there, and creating a user
thread with that page as its entry point — page-table work deferred to the next
step. The dispatch path itself is real and exercised from the self-test harness.
An unknown syscall number still panics (fatal), which is the expected behavior
until error-return conventions are defined.

**Update (branch `shard-local-kernel`):** the first real-EL0 user thread now
exists as a self-test (`crates/catten/src/self_test/el0.rs`). It creates a user
address space, maps a user-code page at `0x0001_0000` with `AP_EL0` access,
writes an AArch64 assembly stub (`SVC #0; wfi; b -4`) into it, and calls
`spawn_thread(asid, vaddr)` to create a user thread. When the scheduler switches
to this thread, `user_trampoline` eret's to EL0, the stub executes `SVC #0`,
`sync_dispatcher` decodes and dispatches it, advances `ELR_EL1` by 4, and
eret's back. This validates the entire syscall infrastructure end-to-end.

### 9.4 Bounded cross-LP IPI queue and typed-message dispatch

The cross-shard backpressure specified in §6 now exists in the kernel. The
`IPI_CMD_QUEUES` are **bounded** (256 entries per LP, configurable via
`DEFAULT_QUEUE_CAPACITY` in `ipi.rs`), created via `ConcurrentQueue::bounded`
instead of the previous `unbounded`. Two push paths exist:

- `try_push_to` / `try_send_ipi_rpc` — when the target LP's queue is full,
  `push()` on the bounded `ConcurrentQueue` returns `Err(Full(rpc))`, which
  propagates back to the caller as first-class backpressure (analogous to
  `submit` returning `WouldBlock`). This is the path sitas's
  `ShardedSubmitter::submit_to` would use.
- `push_to` / `send_ipi_rpc` — a must-not-drop path for kernel-internal RPCs
  (TLB shootdowns, scheduler wakeups). If the queue is full, the oldest entry is
  force-evicted (a `force_push` fallback sized to virtually never fire).

The `IpiRpc` enum gains a `Closure(Box<dyn FnOnce() + Send>)` variant, plus a
manual `Debug` impl. `try_run_on_lp(target, closure)` wraps arbitrary work in
this variant and returns the closure back on backpressure. This is the
kernel-side seed of a typed `ShardMailbox<M>`: a sender boxes work and the
target LP executes it on its own scheduler.

The `dispatch_ipi_rpc` function (shared by `drain_local_ipi_queue` from the
AArch64 IRQ path and `ih_interprocessor_interrupt` on x86_64) handles the
`Closure` variant by calling `f()`, executing the boxed work inline.

Self-tests in `crates/catten/src/self_test/ipi.rs` validate: bounded queue
semantics (entry count, backpressure on full queue), `try_run_on_lp` closure
execution (flag set after drain), `try_send_ipi_rpc` Wakeup round-trip, and
backpressure rejection returning the exact `IpiRpc` variant sent. Tests run
locally on the BSP since only one LP is active at boot time; the contract is
independent of whether the target LP is local or remote. Builds cleanly for
both architectures, no new warnings.

### 9.5 Shared-memory CQ ring

The completion-queue ring buffer, the prerequisite for zero-syscall completion
draining from userspace (§4.2), now exists as
`crates/catten/src/completion/cq.rs`. It is a single 4 KiB page containing a
header (head, tail, capacity, overflow — 4 × u32) and a circular entry array
(16 bytes per entry: `cap: u64, result: i64`). The kernel is the single
producer (`write(cap, result)` advances the head); the consumer (ultimately
userspace) calls `read()` which advances the tail.

The ring is allocated from the kernel heap (`alloc::vec![0u8; 4096]`) and the
reference is a raw pointer — once mapped into a user address space, the same
physical memory is visible from both sides. Entries are written/read with
`volatile` accesses and a `fence(Release)`/`fence(Acquire)` barrier pair,
ensuring correct ordering without cache-coherence surprises. Overflow is
detected (head + 1 == tail) and counted rather than silently overwriting.

Self-tests validate: write → pending count, drain in insertion order,
fill-to-capacity, overflow detection, and `OpResult` ↔ `i64` encoding
round-trip. An integration self-test (`self_test/cq_completion.rs`) validates
the full submit → complete → ring-entry cycle: an AS with an attached CQ ring,
`complete()` writes the entry to the ring, `cq_pending()` returns the count,
and the ring is drained and verified. Both architectures build cleanly.

What remains: map the ring page into a user address space (the AP_EL0
page-mapping infrastructure from the real-EL0 test now exists) and wire
the `wait` syscall to block until `pending() > 0`. That is the last
piece of the zero-syscall-completion loop.

---

## 10. Suggested next steps (within Option C)

1. **Nail the CQ/SQ shared-memory layout** (io_uring-style rings mapped into the
   address space) — this is the prerequisite for zero-syscall draining assumed
   throughout §4.
   **(Done — see §9.5: `completion::cq::CompletionQueueRing`, a single-page ring
   with header + entry array; kernel-side producer tested.)**
2. **Prototype the capability table** (per-AS `IdTable`-backed map from
   `CompletionCap` → `Arc<dyn Observable>`), reusing `IdTable` and the
   observer/waker machinery.
   **(Done — see §9.2: in-tree `completion` module with boot-time self-tests.)**
3. **Bring up a syscall entry path on AArch64** (decode `ESR_EL1.EC == 0b010101`
   in `sync_dispatcher` instead of panicking) sufficient to call `submit`/`wait`
   from an EL0 test thread.
   **(Done — see §9.3: syscall dispatch table wired; self-tests exercise all
   routes via synthetic `TrapFrame`. Real-EL0 test thread deferred.)**
4. **Bound `IPI_CMD_QUEUES`** and generalize `IpiRpc` toward a typed message, so
   cross-shard submit exerts backpressure (this also seeds Option B / Phase 3).
   **(Done — see §9.4: bounded per-LP queues with `push`-returns-backpressure
   + `Closure` variant for arbitrary cross-LP work dispatch.)**
5. **Feed back into Option A:** generalize sitas's `ReactorBackend::Handle` from
   `RawFd` to an associated type end-to-end (the readiness layer), then implement
   `CharlotteReactor` against a mock of this ABI to validate the shapes before the
   kernel path exists. **(Done — see §9.1.)**

---

## 11. References

- This collaboration's design note: [`sitas-runtime-model.md`](./sitas-runtime-model.md)
  (§3 Option C, §5.4 async syscalls, §7 friction, §9a phased plan, §11 findings).
- sitas Option A seam (`os-backend-seam` branch): `src/reactor_backend.rs`
  (`ReactorBackend`/`ReactorWaker`/`ReactorEvent`), `src/executor/driver.rs`
  (idle wait), `src/os/uring.rs` + `src/executor/uring.rs` (buffer-ownership),
  `src/sharded_executor.rs` (`ShardedSubmitter`), `src/shard_mailbox.rs`
  (`ShardSender<M>`), `src/runtime.rs` (`Reply<T>`/`ReplyFuture<T>`).
- CharlotteOS kernel facilities the ABI binds to:
  - Completion-capability design intent: `crates/catten/src/cpu/scheduler/threads/mod.rs:63-69`
  - Observer/waker: `crates/catten/src/klib/observer/mod.rs`,
    `crates/catten/src/cpu/scheduler/threads/waker.rs`
  - Block-on-observable: `crates/catten/src/cpu/scheduler/system_scheduler/mod.rs:70`
  - Timer events (awaitable completion pattern): `crates/catten/src/timers/mod.rs`
  - Cross-LP RPC / IPI: `crates/catten/src/cpu/multiprocessor/ipi.rs`
  - AArch64 IRQ→IPI→yield: `crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs:51`
  - Syscall entry (panics today): `crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs:30`
  - Per-LP state: `crates/catten/src/cpu/multiprocessor/spin/per_lp.rs`
</content>
</invoke>
