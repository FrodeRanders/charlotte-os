# Sitas on CharlotteOS: A Co-Designed Shard-per-Core Runtime

> A design note exploring what a Seastar-like, shard-per-core userspace runtime
> ([`sitas`](https://github.com/FrodeRanders/sitas)) would look like implemented on
> top of — or in co-design with — CharlotteOS, an operating system built to be
> asynchronous from the first commit and to *expose* that asynchrony to its
> inhabitants.
>
> Status: exploratory. This note records the ideas; it does not commit the
> kernel to any specific ABI. Nothing here is implemented yet.

---

## 1. Why these two projects belong together

Two independent experiments arrived at the same shape from opposite directions.

**`sitas`** starts in userspace and asks: *what does shared-nothing,
shard-per-core concurrency look like when it is built out of Rust ownership
instead of locks?* Its answer is a discipline: each shard owns its state, only
the owning shard mutates it, and everything that crosses a shard boundary is an
**owned value moved through an explicit typed message**. On top of that it grows
a custom async executor, per-shard reactors, `io_uring` completion I/O, and
snapshot-based observability — deliberately re-deriving the Seastar model rather
than cloning it.

**CharlotteOS** starts in the kernel and asks: *hardware is asynchronous with
respect to the CPU, so why is the OS synchronous?* Its answer is to make the
kernel async-first — cheap threads, an observer/waker event model, and (per its
design) system calls that are asynchronous by default, returning a completion
capability that userspace can wait on.

The essay that framed this collaboration
([*"Hardware Is Asynchronous. Most of Our Operating Systems Still Aren't."*](https://vorjdux.com/articles/hardware-is-async.html))
makes the tell explicit: a Unix file descriptor, a Windows `HANDLE`, and an
observer capability are the same primitive — *a kernel-managed reference to
something you can wait on*. Everyone converges on it. The interesting move is to
**start there on purpose**.

`sitas` and CharlotteOS have both started there. This document is about what
happens when they meet: a userspace runtime whose async model **agrees** with
the kernel it runs on, instead of emulating async-first on a blocking
foundation. That agreement is the whole point — it removes the "retrofit tax"
(async-signal-safety, thread pools, `io_uring`-over-blocking-AIO) that exists
only because the runtime and the OS disagree about what is fundamental.

---

## 2. The structural correspondence

The reason this is more than an analogy: **the pieces already exist on both
sides, and they line up almost one-to-one.** CharlotteOS's kernel already
contains the kernel-side equivalents of sitas's runtime layers (several were
built or completed during the AArch64 port).

| `sitas` concept | Role | CharlotteOS equivalent (exists today) |
|---|---|---|
| Shard = OS thread pinned to a core | Unit of isolation | **Logical Processor (LP)** + `PerLp<T>` sharded state |
| `ShardId` explicit placement | Addressing | `LpId` |
| Shard mailbox (`ShardSender`/`ShardReceiver`, bounded, owned `M: Send`) | Cross-shard transport | `IPI_CMD_QUEUES`: per-LP `ConcurrentQueue<IpiRpc>` + `send_unicast_ipi` |
| Custom executor `Waker` re-enqueue | Wake a task | `Waker` (`cpu/scheduler/threads/waker.rs`) submitting a ready thread |
| `Reply<T>` / `Notify` (awaitable completion) | Await a result | `Observable`/`Observer` + `TimerEvent` |
| Reactor idle wait (`epoll`/`kqueue`/`poll`) | Block until events | GIC/APIC IRQ dispatch → timer/IPI → `cond_yield_lp` |
| `io_uring` completion I/O | Async syscalls | (Design goal) async syscalls returning a completion capability |
| `ShardLocal<T>` (no mutex, owner-checked, non-escaping refs) | Shard-owned mutable state | `PerLp<T>` (no mutex, LP-owner-checked via `RwLock` guards) |
| CPU pinning (`sched_setaffinity`, cpuset-aware) | Bind shard to core | Intrinsic: an LP *is* a core; the scheduler is already per-LP |
| NUMA memory placement (`set_mempolicy`) | Local allocation | (Future) per-LP allocator arenas |
| `ShardSnapshot` / `RuntimeSnapshot` owned observability | Debug without shared state | The kernel's house style is already owned snapshots |
| Scheduling groups (weighted virtual runtime) | Resource classes | `RoundRobin` LP scheduler (extension point) |

The lesson: **`sitas` is, in effect, re-deriving in userspace the runtime that
CharlotteOS is trying to be natively.** Where sitas fought to build a reactor on
top of a synchronous OS, CharlotteOS offers the reactor as the substrate.

### 2.1 The one primitive underneath everything

Both projects reduce to one waitable-handle primitive:

- sitas: `Reply<T>` / `ReplyFuture<T>` / `Notify` — waker-aware, `no`-external-runtime.
- CharlotteOS: `Weak<dyn Observer>` registered against an `Observable`
  (e.g. a `TimerEvent`), where `Observer::notify` wakes a blocked thread.

sitas's `Reply<T>` stores a `Waker`; CharlotteOS's `Waker` *is* an `Observer`
that resubmits a thread to the scheduler. These are the same object with
different names. A CharlotteOS **completion capability** is exactly the fusion:
a kernel-managed handle that a userspace task can `.await`, backed by the
observer list the kernel already maintains.

---

## 3. Three realizations, from least to most ambitious

### Option A — `sitas` as a `no_std` guest runtime (port the seam)

Keep sitas's entire semantic core — the shard model, typed commands,
`Reply<T>`, `ShardLocal<T>`, the executor, scheduling groups, observability —
and replace only its **bottom layer**, the `os` module, with a CharlotteOS
backend.

This is realistic because sitas was architected precisely so the OS is a thin,
swappable seam:

- The `os` module (~1,000 lines: pipe/`epoll`/`kqueue`/`poll` + socket FFI +
  `io_uring`) is the *entire* OS-facing surface.
- Everything above it is std/ownership logic that is already `no_std` in spirit
  (edition 2024, zero external dependencies).

What changes:

| sitas today (Unix) | sitas on CharlotteOS |
|---|---|
| `std::thread` per shard | CharlotteOS thread per LP (`spawn_thread`, LP-affine) |
| `std::sync::mpsc` mailbox | kernel capability-backed typed channel |
| `OsReactor::wait` (epoll/kqueue) | await a set of kernel completion capabilities |
| `OsWaker::wake` (pipe write) | signal a capability / cross-LP IPI |
| `io_uring` file/net futures | async syscall → completion capability |
| `std` alloc/collections | `alloc` + `core` |

Result: sitas becomes the reference **userspace shard runtime for
CharlotteOS** — the most faithful expression of both projects' "boring core,
experimental edge" stance, with minimal kernel change.

### Option B — sitas's *discipline* adopted inside the kernel

CharlotteOS already has the ingredients; sitas contributes the **invariants** as
an internal programming model:

- Promote `PerLp<T>` into a `ShardLocal<T>`-style API that enforces sitas's
  strongest rule — *references to shard-owned state must not escape the access
  closure or cross `.await`.* Today `PerLp<T>` hands out `RwLock` guards; sitas
  shows how to get the same safety with an owner-check + closure and no lock on
  the hot path.
- Generalize `IpiRpc` into a typed `ShardMailbox<M>`. The kernel already does
  exactly this for one message type (TLB shootdown RPCs over
  `IPI_CMD_QUEUES`); sitas shows how to make it a first-class, typed,
  owned-message transport for any `M`.
- Unify `TimerEvent`/`Observer` and a task `Waker` into one "awaitable
  completion" type used everywhere in the kernel.

This is less "run sitas" and more "**CharlotteOS adopts sitas's invariants as
its concurrency doctrine**," which keeps the kernel's own code shared-nothing.

### Option C — sitas as the executable spec of the async syscall ABI

The deepest connection. sitas's central research question —
*what is the right submission/completion boundary for async I/O?* — **is**
the userspace↔kernel ABI question CharlotteOS must answer. So sitas can serve as
the **executable specification** of CharlotteOS's async syscall interface:

- sitas `submit_* -> Reply<T>` and its `io_uring` completion model → the shape
  of a CharlotteOS async syscall: *submit an operation, receive a completion
  capability, await it.*
- sitas shard-per-core → CharlotteOS per-LP scheduling; a sitas "shard" is a
  userspace thread bound to an LP with a private kernel completion queue.
- sitas `ShardedSubmitter` (submit to another shard, await the handle) →
  cross-LP work submission over IPI — which the kernel already implements for
  its own use.

In this framing you design the syscall ABI by making sitas run well on it: if
the shard model, completion futures, and cross-shard submit map cleanly onto the
ABI, the ABI is right.

The three options are not exclusive. A natural path is **A first** (prove the
seam), which pressure-tests the ABI ideas that feed **C**, while **B**
opportunistically pulls sitas's discipline into kernel code where it already
half-exists.

---

## 4. The seam: an OS backend trait for sitas

The concrete first engineering step benefits sitas *even on Unix*: extract the
implicit OS contract into an explicit trait so the Unix backend and a future
CharlotteOS backend implement the same interface. Today the coupling is direct
(`executor::driver` calls `os::OsReactor` under `#[cfg(unix)]`); the goal is to
make the reactor/mailbox/thread-spawn a named boundary.

### 4.1 What the executor actually needs from the OS

Reading `src/executor/driver.rs` and `src/os.rs`, the executor's idle path needs
exactly three capabilities:

1. **Block until something happens**, with a deadline: readiness on a set of I/O
   handles, or a wake, or a timeout (`OsReactor::wait_io`).
2. **Wake a blocked reactor** from another thread/core (`OsWaker::wake`).
3. **Submit/complete async operations** (the `io_uring` completion path), which
   is really "an operation whose completion is itself a wakeable event."

Everything else (timers, `Reply<T>`, `Notify`, `ShardLocal<T>`) is built in safe
Rust on top of those.

### 4.2 A proposed backend trait

```rust
/// The OS capabilities a single-shard executor needs. One instance per shard.
/// The Unix implementation wraps epoll/kqueue/poll + a wake pipe; the
/// CharlotteOS implementation wraps kernel completion capabilities.
pub trait ShardOsBackend {
    /// Opaque, cheaply-cloneable handle used to wake this shard's reactor from
    /// another shard/core. On Unix: the pipe write end. On CharlotteOS: a
    /// capability whose signal delivers a cross-LP IPI.
    type Waker: ShardWaker;

    /// A kernel/OS reference to a pending async operation whose completion is a
    /// wakeable event. On Unix: an io_uring operation id. On CharlotteOS: a
    /// completion capability returned by an async syscall.
    type Completion: ShardCompletion;

    fn waker(&self) -> Self::Waker;

    /// Block until: an interest becomes ready, a completion arrives, the reactor
    /// is woken, or the deadline elapses. This is the executor's only sleep.
    fn wait(
        &self,
        interests: &Interests<'_>,
        deadline: Option<Instant>,
    ) -> io::Result<ShardEvent>;
}

pub trait ShardWaker: Clone + Send + Sync {
    fn wake(&self) -> io::Result<()>;
}

pub trait ShardCompletion {
    /// Non-blocking check; the reactor turns readiness into a task wake.
    fn poll(&self) -> Poll<io::Result<CompletionResult>>;
}
```

`Interests` carries the current readiness registrations and pending completions
the reactor should watch; `ShardEvent` reports which fired (mirroring today's
`OsEvent { woke, readable, writable }` plus completions). The executor loop in
`driver.rs` becomes backend-agnostic: it calls `backend.wait(..)` and applies
the returned event, exactly as it does now for `OsReactor`.

A parallel, smaller trait covers **shard startup and cross-shard transport** so
`sharded_executor` and `shard_mailbox` are not hardwired to `std::thread` +
`std::sync::mpsc`:

```rust
pub trait ShardRuntime {
    type JoinHandle;
    /// Spawn one shard worker, ideally pinned to a specific core/LP.
    fn spawn_shard(&self, shard: ShardId, placement: Placement, entry: ShardEntry)
        -> io::Result<Self::JoinHandle>;
    /// Create a bounded owned-message channel between shards.
    fn channel<M: Send + 'static>(&self, capacity: usize)
        -> (ShardSender<M>, ShardReceiver<M>);
}
```

On Unix these are thin wrappers over what sitas already does. On CharlotteOS
they map to `spawn_thread` (LP-affine) and a kernel capability channel.

### 4.3 Why this helps sitas immediately

Even before CharlotteOS exists as a target, this refactor:

- turns the current `#[cfg(unix)]` sprawl in `driver.rs`/`os.rs` into one named
  contract;
- makes the `io_uring` vs readiness "two wait sources" tension (a known sitas
  non-goal item) a matter of *which backend*, not scattered cfgs;
- gives a clean place to add a test/mock backend for executor unit tests;
- documents, in types, exactly what sitas assumes about the OS.

---

## 5. The CharlotteOS side: what the backend binds to

The CharlotteOS backend of the trait above binds to kernel facilities that
mostly already exist. Concretely:

### 5.1 Shard = LP-affine thread
`crate::cpu::scheduler::spawn_thread(asid, entry)` already creates a thread; the
scheduler is already per-LP (`SYSTEM_SCHEDULER` holds one `LpScheduler` per LP).
`ShardRuntime::spawn_shard` maps to spawning one thread per LP and keeping it
there. Unlike Unix, **placement is not advisory** — an LP is a core, so a sitas
shard bound to an LP is bound to a core by construction. No `sched_setaffinity`
race, no cpuset surprises.

### 5.2 Mailbox = typed capability channel
The kernel already runs a per-LP owned-message queue: `IPI_CMD_QUEUES`
(`ConcurrentQueue<IpiRpc>`), drained by `drain_local_ipi_queue()` from the IRQ
path, with delivery kicked by `send_unicast_ipi`. Generalizing `IpiRpc` to a
typed `M` and exposing it to userspace as a capability yields sitas's
`ShardSender<M>`/`ShardReceiver<M>` directly. The cache-behavior analysis in
sitas's `ARCHITECTURE.md` (payload cache-line migration, queue-metadata false
sharing, one-heavily-targeted-inbox coherence bottleneck) applies verbatim and
is, if anything, *more* controllable in-kernel.

### 5.3 Reactor wait = await completion capabilities
`OsReactor::wait` (block until readiness/wake/timeout) maps to the kernel
blocking a thread on a set of completion capabilities. The machinery exists:
`TimerEvent` is `Observable`, threads block by registering their `Waker` as an
`Observer` (see `SystemScheduler::block_thread`), and `sleep`/timer expiry wakes
them. Generalize "block on a timer event" to "block on any completion
capability" and the reactor is native.

### 5.4 Async syscalls = the completion path
This is the design frontier and where Option C lives. CharlotteOS's stated model
is that nearly all syscalls are asynchronous, returning a completion capability
(the `Thread`'s exit-observer comment in `scheduler/threads/mod.rs` already
describes syscalls registering a completion capability as an observer of the
worker thread). sitas's `io_uring` futures (`read_at`, `write_all_at`, and the
abandoned-buffer safety discipline) are the userspace consumer that would drive
the shape of this ABI: submit an op + owned buffer, get a completion capability,
await it, reclaim the buffer on completion. sitas's **drain-or-leak teardown
rule** ("never free a buffer while the kernel may still touch it") is exactly
the ownership contract the kernel side must honor too.

### 5.5 Wake = IPI
`ShardWaker::wake` from shard A targeting shard B is `send_ipi(target_lp)` / a
signaled capability that raises a cross-LP IPI, landing in B's IRQ dispatcher,
which drains work and yields — the path we wired during the AArch64 GIC work
(`irq_dispatcher` → `drain_local_ipi_queue` → `cond_yield_lp`).

---

## 6. What it looks like assembled (Option A)

```
CharlotteOS user process
 ├─ 1 thread per LP        (kernel: spawn_thread, LP-affine)   ← ShardRuntime
 ├─ sitas executor / LP    (unchanged: tasks, wakers, timers,
 │                           scheduling groups, observability)
 ├─ sitas ShardLocal<T>    (unchanged: KV, counter, your services)
 ├─ sitas Reply<T>/Notify  (unchanged: awaitable completions)
 └─ ShardOsBackend (CharlotteOS)      ← the only new code
        wait()        → await kernel completion capabilities (deadline-aware)
        Waker::wake() → signal capability → cross-LP IPI
        channel<M>()  → kernel typed capability channel
        Completion    → async-syscall completion capability
```

Everything above the backend is sitas as it exists. The backend is the port.
And because the kernel's async model already matches sitas's, the backend is a
*translation*, not an *emulation* — there is no blocking call being faked, no
thread parked to simulate a completion, no signal-handler tightrope.

---

## 7. Where the friction (and the research) is

Honest corners, because the interesting failures live here:

1. **`no_std` reality.** sitas is std-only today (even the "std-only baseline"
   is a selling point). Splitting the semantic core into a `no_std` +
   `alloc` crate with the OS behind a trait is real work, though the zero
   external dependencies help. Some `std` types (`Instant`, `mpsc`, `thread`)
   need `core`/`alloc` replacements backed by the kernel.
2. **Cancellation across the boundary.** sitas already treats drop as
   cancellation and has the abandoned-`io_uring`-buffer safety model. On
   CharlotteOS the same question becomes "what happens to an in-flight async
   syscall when the awaiting task is dropped?" — the kernel needs a cancel +
   deferred-reclaim contract mirroring sitas's drain-or-leak. This is a place
   where sitas's existing discipline can *specify* kernel behavior.
3. **Backpressure end to end.** sitas has bounded mailboxes and a spawn
   `BackpressureGuard`. Extending backpressure through a kernel capability
   channel (what happens when a shard's inbound completion queue is full?) is an
   open ABI question — and exactly the kind the essay flags as "hard problems
   move, they don't disappear."
4. **Interrupting user code at an arbitrary instruction.** The async-first model
   still has to deliver upcalls/wakes into a running userspace shard safely.
   CharlotteOS's upcall design and sitas's cooperative-only executor (no
   preemption within a shard) have to agree on where wakes are observed.
5. **Two wait sources → one.** sitas's roadmap wants to unify
   timers/readiness/`io_uring` into one deadline-aware wait. On CharlotteOS
   there is a chance to get this right from the start: **one** completion-wait
   primitive that covers timers, IPIs, and async-syscall completions — because
   the kernel controls all three.

---

## 8. Why this is worth pursuing

- **A rare alignment.** Almost every Rust async runtime fights its OS. sitas on
  CharlotteOS is a case where the runtime and the kernel *agree* on the async
  model. That makes the pairing a clean experiment in what the essay calls
  building the alternative "clean, with no backwards-compatibility burden."
- **Bidirectional validation.** sitas gives CharlotteOS a demanding,
  well-specified userspace consumer to design its async syscall ABI against
  (Option C). CharlotteOS gives sitas a substrate where its model is native, not
  emulated (Option A). Each is the other's proof.
- **The invariants transfer.** sitas's shared-nothing discipline is not
  Unix-specific; it is an *ownership* discipline. It is arguably more at home in
  a kernel that already shards state per LP than in a userspace fighting a global
  address space (Option B).
- **It's the same idea, honestly followed.** Both projects independently
  concluded that the right primitive is a waitable completion handle and that
  state should be shard-owned. Putting them together is not a mashup; it is two
  halves of one thesis meeting in the middle.

---

## 9. Suggested first steps

Ordered so each step has standalone value:

1. **Extract the `ShardOsBackend` / `ShardRuntime` traits in sitas** (Unix
   backend implements them; no behavior change). Immediately improves sitas by
   replacing `#[cfg(unix)]` sprawl with a named contract and enabling a mock
   backend for tests.
2. **Split sitas into `sitas-core` (`no_std` + `alloc`, the model + executor
   behind the traits) and `sitas-unix` (the backend).** Proves the core is
   OS-agnostic.
3. **Draft the CharlotteOS async-syscall/completion-capability ABI** using
   sitas's `submit -> Reply/Completion` and `io_uring` buffer-ownership model as
   the reference consumer (Option C on paper).
4. **Prototype `sitas-charlotte`**: implement the backend traits against
   `spawn_thread`, a typed capability channel (generalized `IpiRpc`), and the
   completion-capability wait. Bring up `basic_kv` on CharlotteOS as the
   hello-world.
5. **Feed back into the kernel (Option B):** where the prototype shows
   `PerLp<T>`/`IpiRpc` want sitas's `ShardLocal<T>`/typed-mailbox ergonomics,
   upstream those into CharlotteOS itself.

---

## 9a. Phased execution plan (spikes, one branch per alternative)

The three alternatives of §3 are pursued as **time-boxed spikes**, in the order
**A → C → B**, following the dependency arrows: A produces the reusable trait
seam that C specs against, and B is most convincing once the userspace shape it
serves is understood. Each spike is small, reversible, and ends by appending a
"what we learned" note to this document. Nothing merges into a mainline branch
until we consciously decide it earns its place.

| Phase | Alt | Repo | Branch | Deliverable | Decision gate |
|-------|-----|------|--------|-------------|---------------|
| 1 | **A** | `sitas` | `os-backend-seam` | Extract `ShardOsBackend`/`ShardRuntime` traits; Unix backend implements them with no behavior change. Optionally begin the `sitas-core`/`sitas-unix` split. | Is the OS contract genuinely small and clean? Does the Unix build/test stay green? Does a mock backend become possible? |
| 2 | **C** | `charlotte-os` | `async-syscall-abi` | Written async syscall / completion-capability ABI, specified against A's trait boundary as the reference consumer. Optional in-tree Rust type sketch. | Do sitas's shard model, completion futures, and cross-shard submit map cleanly onto the ABI? Are cancellation and backpressure expressible? |
| 3 | **B** | `charlotte-os` | `shard-local-kernel` | Generalize `PerLp<T>` → a `ShardLocal<T>`-style API (non-escaping references) and `IpiRpc` → a typed `ShardMailbox<M>`. | Does the discipline improve kernel code without fighting `no_std`? Does it stay compatible with the existing scheduler and IPI paths? |

Rules for the spikes:

- Each branch stays a spike: minimal, self-contained, not merged until decided.
- The AArch64 work stays isolated on its own branch and is not entangled here.
- Findings are recorded per phase in §11 so the eventual decision is documented
  rather than remembered.

---

## 10. References

- CharlotteOS in-tree:
  - Per-LP state: `crates/catten/src/cpu/multiprocessor/spin/per_lp.rs`
  - Cross-LP mailbox / IPI RPC: `crates/catten/src/cpu/multiprocessor/ipi.rs`
  - Observer/waker model: `crates/catten/src/klib/observer/mod.rs`,
    `crates/catten/src/cpu/scheduler/threads/waker.rs`
  - Timer events / awaitable completion: `crates/catten/src/timers/mod.rs`
  - Scheduler surface: `crates/catten/src/cpu/scheduler/mod.rs`
  - AArch64 IRQ→IPI→yield path: `crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs`
- [`sitas`](https://github.com/FrodeRanders/sitas):
  - Architecture & invariants: `docs/ARCHITECTURE.md`
  - OS seam to replace: `src/os.rs`, `src/executor/driver.rs`
  - Shard runtime: `src/runtime.rs`, `src/sharded_executor.rs`
  - Shard-local state: `src/shard_local.rs`
  - Reply/Notify completion primitives: `src/runtime.rs`, `src/executor/sync.rs`
- Motivation: *"Hardware Is Asynchronous. Most of Our Operating Systems Still
  Aren't."* — <https://vorjdux.com/articles/hardware-is-async.html>

---

## 11. Findings (updated per spike)

Recorded as each phase completes, so the eventual choice among the alternatives
is grounded in evidence rather than recollection.

### Phase 1 — Option A (`sitas` `os-backend-seam`)

_In progress. First step complete: reactor contract extracted and validated._

**What was done (commit `61ead07` on `sitas` branch `os-backend-seam`):** added a
`reactor_backend` module defining the OS contract the executor requires as
explicit traits — `ReactorBackend` (obtain a waker; block until an interest is
ready / woken / timed out), `ReactorWaker` (cross-thread wake), and
`ReactorEvent` (owned wait result). Blanket impls prove the existing Unix
`OsReactor`/`OsWaker`/`OsEvent` satisfy it with no behavior change; a mock
in-memory backend in tests proves a non-Unix backend is implementable behind the
same interface. `cargo fmt`/`clippy -D warnings`/`test` all green (318+ existing
tests unaffected).

**Decision-gate answers so far:**

1. *Is the OS contract genuinely small and clean?* **Yes — strikingly so.** By
   inspection of `executor::driver`, the executor needs exactly three things
   from the OS: obtain a cloneable waker, block-until-ready-or-woken-or-timeout,
   and wake from elsewhere. The whole reactor seam is `waker()`,
   `wait(read, write, timeout) -> Event`, and `wake()`. This strongly supports
   Option A's feasibility and de-risks the CharlotteOS backend.
2. *Does a non-Unix backend become possible?* **Yes.** The mock backend
   satisfies the contract with no OS descriptors, using an opaque capability-id
   `Handle` type instead of `RawFd` — exactly the shape a CharlotteOS
   completion-capability reactor would take. The associated `Handle` type is the
   key design win: the executor's notion of "the thing I registered interest in"
   is no longer hardwired to a Unix file descriptor.

**Notes / open items carried forward:**

- This step defines and validates the boundary but does **not** yet thread the
  trait through `Executor`/`Scheduler` (they still hold concrete
  `OsReactor`/`OsWaker`). That generic-threading is the next sub-step and will
  reveal whether the `RawFd`-typed interest tracking inside `Scheduler`
  (`read_interest_fds`/`wake_readable_fds`) generalizes cleanly to an associated
  `Handle`.
- The `io_uring` completion path (`executor::driver` under
  `#[cfg(target_os = "linux")]`) is a *second* wait source not yet covered by
  the reactor trait. This mirrors sitas's own "unify the wait sources" roadmap
  item and is exactly where CharlotteOS could do better from the start (one
  completion-wait primitive for timers, IPIs, and async-syscall completions).
- The `ShardRuntime` half of the seam (shard spawn + typed channel, replacing
  `std::thread` + `std::sync::mpsc`) is still to be sketched.

**Step 2 — trait threaded through `Executor`/`Scheduler` (commit `d1085bb`):**
the reactor is now a real type parameter, `Executor<R: ReactorBackend<Handle =
RawFd> = OsReactor>`, defaulted so existing callers are unchanged. The driver's
idle wait goes through `R` via the trait rather than a concrete `OsReactor`. The
scheduler's waker is type-erased to `Box<dyn SchedulerWake>` — because the
scheduler only ever *wakes* its reactor and never needs the concrete type, this
avoids rippling a type parameter through ~10 files while still routing wakes
through the active backend. All 318+ tests, `fmt`, and `clippy -D warnings`
green; `executor_sleep` and `async_tcp_echo` verified end-to-end.

**The sharpest finding from threading it through:** the reactor *seam* abstracts
cleanly, but the **readiness-tracking layer is the real coupling point**, not the
reactor. `io_interest`/`unix_io` and the `io_uring` completion fd all identify
interests by `RawFd` (33 uses across three files), so the backend's `Handle` is
currently pinned to `RawFd` by a bound on the `Executor` struct. In other words:
*abstracting "block until an event" was trivial; abstracting "what is an
interest" is the actual work.* This is a direct, useful input to Option C — the
CharlotteOS async ABI's central question is precisely **what a waitable interest
handle is** (a completion capability), and this spike shows that is where the
design effort must concentrate, not on the wait primitive itself.

**Carried into the next sub-step / Option C:**

- Generalize the interest `Handle` from `RawFd` to an associated type end-to-end
  (scheduler interest maps, `io_interest`, readiness futures). This is the change
  that would let a CharlotteOS completion-capability backend drop in.
- The `io_uring` completion path remains a second wait source; unifying it with
  readiness under one handle/one wait is shared work between sitas's roadmap and
  the CharlotteOS ABI.
- `ShardRuntime` (spawn + typed channel) half still to be sketched.

### Phase 2 — Option C (`charlotte-os` `async-syscall-abi`)

_In progress. First deliverable complete: the async syscall / completion-capability
ABI is drafted and specified against Option A's trait boundary._

**What was done (branch `async-syscall-abi`):** wrote
[`docs/async-syscall-abi.md`](./async-syscall-abi.md) — a full ABI specification
that treats sitas as the executable spec. It grounds the design in the kernel
facilities that already exist (`Thread: Observable` + `Waker: Observer` +
`SystemScheduler::block_thread`, per-LP schedulers, `IPI_CMD_QUEUES`, the
AArch64 `irq_dispatcher → drain_local_ipi_queue → cond_yield_lp` path) and maps
sitas's `ReactorBackend`/`Reply<T>`/`ShardedSubmitter` onto a five-operation
surface: `submit → CompletionCap`, `wait(cq, min_complete, deadline)`,
`wake(cq)`, `cancel(cap)`, `close(cap)`. It includes a non-compiled Rust type
sketch and an explicit list of what must be built vs. reused.

**Second deliverable — the ABI now has an executable model (sitas branch
`reactor-handle-seam`, `src/charlotte_abi.rs`):** an in-memory `MockKernel`
implementing the five operations plus a `CharlotteReactor` that satisfies
sitas's Option-A `ReactorBackend` contract with `Handle = CompletionCap`. Nine
tests exercise every decision-gate claim (completion path returning the owned
buffer; cross-shard wake unblocking a blocked reactor; `cancel` with deferred
buffer reclaim; `WouldBlock` submission backpressure and non-lossy CQ overflow;
driving the reactor purely through the trait). `cargo fmt`/`clippy -D
warnings`/`test`/`doc` all clean; the existing 318+ tests are unaffected. This
turns Option C from "on paper" into a validated, executable specification. The
larger remaining piece — threading a generic `Handle` through the real
`Executor`/`Scheduler` so the actual executor runs on the backend — is deferred:
the `unix_io` readiness layer is inherently `RawFd`-bound and a completion-based
kernel backend does not use it (it uses the CQ/completion path modelled here).

**Third deliverable — the submission side of the ABI now exists in the kernel
(branch `async-syscall-abi`, `crates/catten/src/completion/mod.rs`):** a per-AS
capability table (`IdTable<Arc<Completion>>`) where each `Completion` is
`Observable`, so `wait` blocks a thread by registering its `Waker` as an
`Observer` exactly as `sleep`/`TimerEvent` do. The five operations are present as
kernel-internal functions (`submit`/`complete`/`poll`/`wait`/`cancel`/`close`,
plus `observe`), built entirely on facilities that already existed (`IdTable`,
the observer/waker model, `block_thread`). Boot-time self-tests
(`crates/catten/src/self_test/completion.rs`) validate buffer transfer, the
observer-signal path, buffer hand-back, `close`, cancel-with-deferred-reclaim,
and `WouldBlock` submission backpressure. Builds and links cleanly for both
`aarch64` and `x86_64` (display feature off — unrelated pre-existing host-link
issue), no new warnings.

**Fourth deliverable — syscall entry path:** the AArch64 `sync_dispatcher` in
`interrupts/mod.rs` no longer panics on SVC. It decodes `ESR_EL1.EC`, and when
EC == 0x15 (SVC from AArch64) reads the volatile register context saved by the
IVT's `push_volatile_regs` into a `TrapFrame` (x0-x18, ELR_EL1, SPSR_EL1,
SP_EL0), advances `ELR_EL1` by 4 to skip the SVC instruction on `eret`, extracts
the SVC immediate from `ESR_EL1.ISS` as the syscall number, and hands off to
`syscall_dispatch()` in `crates/catten/src/syscall/mod.rs`. The dispatch table
maps SVC #0 (LOG) and SVC #1–6 (the five completion operations) to handler
functions that call the `completion` module. Boot-time self-tests
(`crates/catten/src/self_test/syscall.rs`) exercise every dispatch route by
calling `syscall_dispatch` directly with a synthetic `TrapFrame`, verifying all
seven syscalls route without panicking and the completion-cap operations are
reachable through the table. The real-EL0 SVC path (`sync_dispatcher` with
`push_volatile_regs` on the kernel stack, `eret` back to EL0) is compiled into
`sync_dispatcher` but not exercised in self-tests because that requires a
user-mapped code page (page-table work deferred). Builds cleanly for both
architectures, no new warnings.

**Decision-gate answers:**

1. *Shard model maps cleanly?* **Yes** — shard = LP-affine thread + private
   completion queue; placement is stronger than Unix (an LP *is* a core).
2. *Completion futures map cleanly?* **Yes, more naturally than on Unix.**
   `Reply<T>`/`ReplyFuture<T>`/`JoinHandle` all reduce to "await one
   `CompletionCap` via the CQ." The kernel already stores a `Waker` as an
   `Observer`; sitas stores a `Waker` in `ReplyShared` — the same object. A
   one-shot completion event beats fd readiness (no re-arm) and **unifies
   sitas's two wait sources into one CQ** (resolves the §7.5 roadmap item).
3. *Cross-shard submit maps cleanly?* **Yes** — `submit_to(other)` → enqueue on
   target LP + `wake`, exactly what the kernel already does for TLB-shootdown
   RPCs; the delta is a typed, bounded message.
4. *Cancellation expressible?* **Yes — and the reference consumer specifies it.**
   sitas's drop-as-cancel + `defer_buffer_drop` + drain-or-leak translate
   directly into the ABI's `cancel` + deferred-reclaim buffer contract. An open
   kernel question (§7.2) becomes a settled contract.
5. *Backpressure expressible?* **Yes, with one required kernel change:** bounded
   SQ + non-lossy overflow-pending CQ + **bounding the per-LP cross-shard queue**
   (`IPI_CMD_QUEUES` is unbounded today). With that, backpressure is end-to-end.

**The confirmed center of gravity (from Phase 1):** the hard part is not the
wait primitive (trivial, already in-kernel) but **what a waitable interest is**.
The ABI answers this with `type Handle = CompletionCap`: a kernel-managed
completion capability backed by a per-AS capability table + a buffer-ownership
contract. That table + contract is what Option C should prototype first.

**Fifth deliverable — bounded cross-LP IPI queue + typed-message dispatch:**
`IPI_CMD_QUEUES` is now bounded per LP (`ConcurrentQueue::bounded(256)`) and
`IpiRpc` carries a `Closure(Box<dyn FnOnce() + Send>)` variant that packages
arbitrary work for cross-LP execution — the kernel side of the cross-shard
backpressure contract from the ABI doc §6. `try_push_to`/`try_send_ipi_rpc`
propagate `Err(Full(rpc))` back to the caller (first-class backpressure);
kernel-internal RPCs (TLB shootdown, scheduler wakeup) use `push_to` with a
force-evict fallback. `try_run_on_lp(target, closure)` wraps work in the
`Closure` variant and returns the closure on backpressure. Both
`drain_local_ipi_queue` (AArch64 IRQ path) and `ih_interprocessor_interrupt`
(x86_64 legacy path) dispatch `Closure` by invoking `f()`. Self-tests
(`self_test/ipi.rs` — bounded semantics, closure execution, backpressure
rejection returning the exact RPC variant sent) pass; both architectures build
cleanly, no new warnings. This directly seeds Option B (Phase 3): the
`Closure` variant is the prototype of a typed `ShardMailbox<M>` and the bounded
queue enforces the sitas bounded-mailbox discipline in the kernel itself.

### Phase 3 — Option B (`charlotte-os` `shard-local-kernel`)

_In progress. First deliverables complete: lock-free `ShardLocal<T>` and typed
`ShardMailbox<M>`._

**What was done (branch `shard-local-kernel`, builds on the IPI work from Phase
2):**

- **`ShardLocal<T>`** (`crates/catten/src/cpu/multiprocessor/spin/shard_local.rs`)
  — a lock-free per-LP container that replaces `PerLp<T>`'s `RwLock` with
  `UnsafeCell<T>` + two runtime assertions: (1) owner-check (the caller must be
  on the owning LP), (2) re-entrancy guard (a per-LP `AtomicBool` borrow flag,
  set on entry, cleared via RAII `BorrowGuard` on exit). `try_with(f)` returns
  `Result<R, ShardLocalAccessError>`; `with(f)` panics on violation. There is no
  lock — the safety comes from the single-threaded-LP invariant that sitas's
  `ShardLocal<T>` enforces clousure-style. Useful for LP-local data that is
  never touched from an IRQ handler or cross-LP.

- **`ShardMailbox<M>`** (`crates/catten/src/cpu/multiprocessor/shard_mailbox.rs`)
  — a typed bounded per-LP queue (`ConcurrentQueue::bounded(256)`) with
  cloneable `ShardSender<M>` and single-consumer `ShardReceiver<M>`. `try_send`
  returns `Err(M)` on backpressure (sitas `ShardSendError::Full` equivalent) and
  delivers a wake IPI on success; `try_recv` polls the local queue.
  `ShardMailboxSet<M>` is the per-LP collection indexed by `LpId`, with
  `sender_to(lp)` and `receiver_for(lp)`. This is the kernel realization of
  sitas's typed owned-message transport.

Self-tests (`crates/catten/src/self_test/shard.rs`) validate: `try_with`
owner-access and value persistence, re-entrant access rejection, `try_send` /
`try_recv` round-trip, send-backpressure on full queue, and multiple-clone
senders. Both architectures build cleanly, no new warnings.

**Decision-gate status:**

- *Does the discipline improve kernel code without fighting `no_std`?*
  **Yes.** `ShardLocal<T>`'s closure-based access eliminates the spin-lock
  acquire for LP-local data that is never touched cross-LP or from an IRQ. The
  API is a complement to (not a replacement for) `PerLp<T>` — use `PerLp<T>`
  when interrupt-safety or cross-LP access is needed, `ShardLocal<T>` when the
  data is strictly single-LP. The borrow flag is a single `AtomicBool` swap
  vs. a CAS loop + interrupt mask.
- *Does it stay compatible with the existing scheduler and IPI paths?* **Yes.**
  `ShardMailbox<M>` uses the existing `send_ipi` and bounded `ConcurrentQueue`
  from the IPI generalization (Phase 2, step 5). `ShardLocal<T>` is a new type
  alongside `PerLp<T>`, not a replacement — no existing callers are affected.

**Additional — first real-EL0 user thread + CQ ring + QEMU boot:** the full
stack has been verified on QEMU AArch64: all 8 self-test modules pass at boot
(completion, syscall, IPI, ShardLocal, ShardMailbox, EL0 SVC round-trip, CQ
ring, CQ ring integration). 4 bugs fixed (close semantics, duplicate poll,
post-dispatch poll, ASID registration). CI added (`.github/workflows/ci.yml`)
building aarch64 + x86_64 with `-D warnings`, plus an automated QEMU boot gate
on `dev` branch pushes. Consolidated into `dev` branch. Two boot scripts:
`scripts/boot-smp1.sh` (quick self-test verification) and
`scripts/boot-aarch64.sh` (full build + boot).

### Phase 3 — Option B (`charlotte-os` `shard-local-kernel`) — **COMPLETE**
_Phase 2 (C) and Phase 3 (B) are now consolidated on the `dev` branch._

### Current scrutiny against the runtime model

The codebase has moved well beyond the original "nothing implemented yet"
status in this note: `sitas-core`/`sitas-charlotte` exist, CharlotteOS has a
completion-capability table, shared CQ ring, real AArch64 SVC dispatch, EL0
thread spawning, bounded IPI queues, `ShardLocal<T>`, and a typed
`ShardMailbox<M>`. The present sitas smoke test therefore proves a meaningful
slice of Option A/C/B: a no-std Rust userspace binary can allocate, spawn a
shard, exercise `ShardedKv`, and report success from EL0.

The review also exposed the line between "executable spike" and "OS ABI":

1. **Caller identity is not yet authority.** Several EL0 syscalls accept an
   ASID in `x0` and operate on that address space's completion table or spawn
   into that address space. For a real ABI, the kernel must derive the caller's
   address space from the running thread/trap context. User-supplied ASIDs are
   acceptable only for kernel-side self-tests and bootstrapping diagnostics.
2. **CQ delivery must become non-lossy.** The CQ ring currently increments an
   overflow counter when full. The model requires the opposite contract:
   completions are either delivered, retained in a kernel backlog, or explicit
   backpressure is returned before submission. A terminal completion must never
   be silently lost from the userspace-visible completion path.
3. **Polling must return ownership/result data.** A syscall-level poll that
   drains a completion but discards the result violates the completion-capability
   contract. Poll and wait need a stable register/shared-memory result ABI.
4. **Mailbox syscalls are not capability channels yet.** The kernel has a typed
   bounded `ShardMailbox<M>`, but the exposed EL0 mailbox calls are still a
   global `u64` smoke-test path. A real userspace channel needs a capability
   naming the channel/receiver, not a transient receiver object constructed on
   each syscall.
5. **The sitas loader is still a flat-image harness.** `UserFlatImage` RWX pages
   and fixed CQ/result/heap virtual addresses were the right way to get the
   first binary running, but the next boundary is an ELF loader with segment
   permissions, declared heap/stack metadata, and no global result-page ABI.
6. **The reactor wait path is still polling.** The runtime model's core win is
   one native wait primitive for timers, IPIs, and completions. Today the CQ
   wait path still spins; the next implementation step is to make the CQ itself
   observable and block the shard on CQ readiness.
7. **Kernel cross-LP closures need a safer representation.** Returning an
   arbitrary `FnOnce` closure on IPI queue backpressure via trait-object pointer
   casts is too brittle for kernel code. The long-term typed mailbox path should
   avoid this escape hatch or represent returned work without unsafe downcast
   assumptions.

Implementation should proceed in that order: make authority correct first,
then make completion delivery non-lossy, then tighten the exposed channel and
loader surfaces.

**Implementation progress after this review:** caller ASID authority now comes
from the running thread for real EL0 syscalls; syscall poll returns completion
status/result instead of discarding it; CQ overflow is retained in a per-AS
kernel backlog and retried; the smoke-test mailbox receive path no longer takes
and drops the durable receiver flag on each syscall; and `wait_on_cq` now blocks
through the scheduler/observer path instead of spinning. `CQ_WAIT` is exposed
through the syscall table, `try_run_on_lp` no longer recovers backpressured
closures through trait-object pointer casts, and mailbox access now has an
incremental sender/receiver capability ABI while preserving the legacy raw-LP
smoke-test calls. Receiver capabilities are AS-scoped single-consumer endpoints
per LP, sender opens validate the target LP up front, and mailbox capability
tables can be torn down with the address space. The hand-written EL0 ping-pong
stub has been migrated from raw-LP mailbox calls to mailbox endpoint-capability
calls, but its default boot gate remains disabled because the test still hangs
under the pre-existing HVF-flaky path and needs a dedicated stabilization pass.
The remaining larger ABI work is to switch `sitas-charlotte` to the `CQ_WAIT`
syscall instead of busy-polling and replace the flat RWX sitas loader with an
ELF/segment-aware loader.
