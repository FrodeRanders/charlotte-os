# CharlotteOS Scheduler State Machines

This document defines the interacting state machines that govern thread
scheduling, completion-queue (CQ) wait semantics, and the idle-load
properties of the system.  It is intended as a reasoning tool: every
scheduler change should be checked against these invariants.

---

## 1. Thread State Machine (`ThreadState`)

**Source:** `crates/catten/src/cpu/scheduler/threads/mod.rs`

```
spawn_thread()
      │
      ▼
NeedsLpAssignment ── submit_ready_thread() ──► Ready(lp)
      │                                             │
      │ block_thread()                              │ next(): dequeue
      ▼                                             ▼
Blocked(Waker) ◄──── block_thread() ───────── Running(lp)
      │                                             │
      └── Waker::notify(generation) ──► Ready(lp) ◄─┘
                                           ▲       preemption: requeue
                                           │
                                  soft-affinity LP

Any live state ── abort_thread() ──► staged DEAD ── reap after switch
```

### Invariants

| # | Invariant |
|---|-----------|
| T1 | `Running(lp)` ⇒ thread IS `current_handle` of LP `lp`. |
| T2 | `Ready(lp)` ⇒ thread IS in `run_queue` of LP `lp`; NOT any `current_handle`. |
| T3 | `Blocked` ⇒ thread is NOT in any `run_queue`. Transiently it may still be `current_handle` (during the `block_thread → cond_yield_lp` window). |
| T4 | `idle_tid` is never in `run_queue` and never `Blocked`. |
| T5 | Queued handles and asynchronous Wakers carry a `generation`; dispatch and wake admission reject stale generations after slot reuse. |
| T6 | `add_thread()` for an already-`Running` or already-`Ready` thread is a benign no-op (aggregated wakes before the thread parks). |

### Lock order for transitions

```
spawn_thread:         MASTER_THREAD_TABLE.write() [drop] → SYSTEM_SCHEDULER.read() → lp_scheduler.lock()
block_thread (self):  SYSTEM_SCHEDULER.read() → MASTER_THREAD_TABLE.read() [drop] → MASTER_THREAD_TABLE.write()
block_thread (Ready): SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
submit_*:             SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
abort_thread:         MASTER_THREAD_TABLE.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
cond_yield_lp:        SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
                      ALL DROPPED before switch_ctx (otherwise deadlock — abandoned stack).
```

The global lock order is: **lp_scheduler → MASTER_THREAD_TABLE**.  `block_thread` for self-block goes `MASTER_THREAD_TABLE` first (no LP lock needed), then `yield_lp` re-acquires in the canonical order. The `abort()` and `sleep()` callers bind `tid` in a temporary scope, drop all guards, then proceed — this avoids holding a read guard across a write for the non-reentrant `RwLock`.

---

## 2. LP Scheduler State (`RoundRobin`)

**Source:** `crates/catten/src/cpu/scheduler/lp_schedulers/round_robin.rs`

```
                    ┌─────────┐
                    │  IDLE   │  quantum timer NOT armed
                    │ is_idle │  thread_count() == 0
                    └────┬────┘
                         │ next() picks real thread from run_queue
                         │  (submitted by IPI or same-LP admission)
                    ┌────▼─────┐
                    │   BUSY   │  quantum timer armed every 10ms
                    │ !is_idle │  thread_count() >= 1
                    └────┬─────┘
                         │ next() picks idle_tid (run_queue empty)
                         │  clear_ctx_switch_pending() skips timer re-arm
                         │
                         ▼
                    [back to IDLE]
```

### Quantum timer sub-state (`TimerEventObserver` + `TimerQueue`)

```
  no SchedulerQuantum key ── ensure_event() ──► one keyed queue event
          ▲                                           │
          │         process_events(): pop + notify    │
          └───────────────────────────────────────────┘
                    (quantum PPI fires)

  pending=false ── set_ctx_switch_pending() ──► pending=true
       ▲                                              │
       │      clear_ctx_switch_pending()              │
       └──────────────────────────────────────────────┘
              (honoured inside cond_yield_lp)
```

### Invariants

| # | Invariant |
|---|-----------|
| L1 | `is_idle == true` ⇒ `current_handle.tid == idle_tid`. |
| L2 | `clear_ctx_switch_pending()` arms a quantum timer ONLY when `!is_idle`. An idle LP gets NO periodic ticks; it is woken by admission IPI or deferred wake. |
| L3 | At most one `SchedulerQuantum` event is queued per LP. Uniqueness belongs to the timer queue; there is no separate `armed` flag that can diverge from it. |
| L4 | `cond_yield_lp` with `curr_tid == next_tid` (sole runnable) still calls `clear_ctx_switch_pending()` to re-arm the timer. Without this, `sleep()` freezes. |
| L5 | The idle loop calls `drain_deferred_wakes()` and interrupt-masked `process_local_events()` before `cond_yield_lp()`, so deferred wakes and due timers are reconciled before `wfi`/`hlt`. |
| L6 | The hardware comparator represents the sorted timer-queue head. Queue mutation and comparator programming are one local interrupt-masked transaction. |

### What makes an LP idle vs busy

| LP goes IDLE | LP goes BUSY |
|---|---|
| `next()` selects `idle_tid` because `run_queue` is empty | `submit_*()` or IPI admits a thread, sets `pending`, `next()` picks it |
| All real threads have blocked, aborted, or been removed | `drain_deferred_wakes()` → `completion::wake()` → `submit_woken_thread()` |

---

### Per-LP logical timer queue

**Source:** `crates/catten/src/timers/mod.rs`

Each LP has one hardware comparator but may have many future logical events.
The logical events stay in deadline order and the hardware always represents
the head:

```
enqueue E:
  mask local IRQs
  insert E in absolute-deadline order
  arm hardware from queue.front()
  restore prior IRQ state

timer IRQ / idle reconciliation:
  while queue.front().deadline <= now:
      event = pop_front()
      notify(event.observers)
  if queue.front() exists: arm its deadline
  else: stop hardware timer
```

Ordinary sleep/completion events are anonymous. The round-robin quantum uses
the key `SchedulerQuantum`, allowing at most one such event per LP. The armed
event remains at the queue head rather than being moved to a separate slot, so
the queue remains authoritative. Normally the comparator receives the head's
absolute deadline; if that deadline has already passed, it receives a minimal
prompt timeout so the IRQ can drain the due head.

| # | Invariant |
|---|-----------|
| TM1 | Logical timer events are sorted by absolute deadline. |
| TM2 | The hardware comparator represents the earliest queued event. |
| TM3 | Insertion, removal, and comparator programming cannot be re-entered by the local timer IRQ. |
| TM4 | An IRQ processes every event due at the sampled `now`, then arms only the new head. |
| TM5 | `SchedulerQuantum` is a keyed singleton; anonymous logical timers remain independent. |

---

## 3. Completion Operation Lifecycle (`OpState`)

**Source:** `crates/catten/src/completion/mod.rs`

```
  submit()
     │
  InFlight ──cancel()──► CancelPending
     │                       │
     │ complete()            │ complete()
     │                       │ (forces Cancelled)
     ▼                       │
  Completed ◄────────────────┘
     │
     │ take() (drain result + buffer)
     ▼
  Observed ── close() ──► [slot reclaimed]
```

### Invariants

| # | Invariant |
|---|-----------|
| C1 | `Completed` and `Observed` are terminal; `is_terminal()` returns true for both. |
| C2 | `Observed` implies the buffer has been returned to the caller. |
| C3 | All `OpState` transitions hold `CompletionInner`'s mutex. |
| C4 | CQ ring write happens-before observer notification in `complete()` (ring entry posted, then observers signalled). |

### Blocking on a completion (`COMPLETION_WAIT`, syscall 4)

```
completion::wait(asid, cap):
  1. Fast path: is_terminal()? → return
  2. block_thread(tid, &completion)   [Waker registered on Completion.observers]
  3. Guard: is_terminal()? → submit_ready_thread(tid)   [lost-wake]
  4. yield_lp()                       [thread sleeps]

  When completion fires:
  → CompletionTimerObserver::notify() → complete()
  → post_to_cq(ring_entry)            [ring write]
  → signal_cq()                       [wakes CQ waiters]
  → completion.signal()               [wakes capability-specific waiters]
  → Waker::notify() → submit_woken_thread()
```

`COMPLETION_WAIT` blocks directly on the completion capability, **bypassing the CQ ring entirely**. This is why services that poll individual completions via `poll(cap)` can block correctly with `wait()` even when the ring is full of stale entries.

---

## 4. CQ Ring Buffer (`CompletionQueueRing`)

**Source:** `crates/catten/src/completion/cq.rs`

Single-producer (kernel), single-consumer (userspace) ring buffer.

```
  head = tail  →  EMPTY  (pending() == 0)
  head ≠ tail  →  NON-EMPTY
  (head + 1) % capacity == tail  →  FULL  (write() returns false)
```

- **Kernel writes:** `write()`, `write_batch()` → advance `head`
- **Userspace reads:** `read()` → advance `tail`
- **Kernel drain:** `drain()` → sets `tail = head` (consumes all)

### Ring interaction with `cq_wait`

```
wait_on_cq(asid, cq, min_complete):
  loop:
    ┌── work_generation != last_seen_generation
    │   → advance last_seen_generation and return
    │
    ├── block_thread(tid, &observable)
    │
    ├── guard: generation changed while registering Waker
    │   → submit_ready_thread(tid)   [lost-wake guard]
    │
    └── yield_lp() → blocks until woken → loop
```

Every posted completion, detached completion, explicit wake, or endpoint-bound
wake increments `work_generation`. The queue-wide `last_seen_generation` is
advanced by the single shard reactor when it consumes readiness. Ring occupancy
is deliberately not the wait condition.

### Resolution of the undrained-ring problem

Under the historical implementation, every `complete()` posted a ring entry
and `cq_wait` treated the undrained `cq_pending()` count as new readiness. If a
caller polled the individual capability without reading the shared ring, the
old entry made every later wait return immediately and the reactor busy-spun.

The old implementation used `cq_pending()` as readiness and therefore spun on
an undrained historical entry. The generation counter decouples reactor
readiness from ring consumption. A caller may poll a specific completion
without draining the shared ring; its next `cq_wait` still blocks until new
work changes the generation.

### Invariants

| # | Invariant |
|---|-----------|
| Q1 | `capacity >= 2` (one sentinel slot to distinguish full from empty). |
| Q2 | At most `capacity - 1` entries are in-flight simultaneously. |
| Q3 | Ring write happens-before observer notification (`complete()` posts ring, then signals). |
| Q4 | `drain()` (`tail = head`) requires `&mut self` — exclusive access. |
| Q5 | CQ wait readiness depends on generation change, not `ring.pending()`. |

---

## 5. CQ Waiter State (`CqState`)

**Source:** `crates/catten/src/completion/mod.rs`

### Work-generation semantics

```
  work_generation == last_seen_generation  → no new work; block
  work_generation != last_seen_generation  → consume by assigning
                                              last_seen_generation = work_generation
```

- `wake()`: increments `work_generation`, releases the registry write lock,
  then calls `signal_cq()`.
- Completion posting increments the same generation after writing/backlogging
  the CQ record.
- The design assumes one blocking reactor per shard CQ. Multiple independent
  waiters would require a cursor per waiter.

### Lost-wake race closure

```
1. generations equal         [fast-path miss]
2. block_thread()             [register Waker]
3.  ◄── wake() fires here ──  [race window]
4. yield_lp()                 [would sleep forever]
5.  BUT: guard observes generation change → submit_ready_thread()
   → thread re-admitted and consumes the new generation
```

### Invariants

| # | Invariant |
|---|-----------|
| W1 | Equal generations mean no unconsumed readiness; unequal generations mean at least one new event. |
| W2 | N arrivals between waiter passes coalesce into one return while preserving monotonic evidence that work arrived. |
| W3 | The lost-wake guard observes a generation change during observer registration and re-admits the thread. |

---

## 6. Device Interrupt Delivery

**Source:** `crates/catten/src/device/mod.rs`

Two-phase, crossing IRQ context → thread context without locks.

```
PHASE A (IRQ context, LOCK-FREE):
  irq_dispatcher(intid)
    → deliver_interrupt(intid)
      → ROUTE_TABLE[intid].load()       [atomic]
      → arch_disable_irq(intid)         [MMIO]
      → IRQ_PENDING[intid]++            [atomic]
      → DEFERRED_WAKES.push(asid, cq)   [lock-free queue]

PHASE B (thread context):
  yield_lp() / lp_idle_loop()
    → drain_deferred_wakes()
      → DEFERRED_WAKES.pop() → (asid, cq)
      → completion::wake(asid, cq)
        → CqState.work_generation++     [COMPLETIONS.write()]
        → signal_cq()                   [COMPLETIONS.read()]
          → Waker::notify()
            → submit_woken_thread()
              → Thread: Blocked → Ready
```

### Invariants

| # | Invariant |
|---|-----------|
| D1 | `deliver_interrupt` holds **zero** kernel locks (only atomics + MMIO). |
| D2 | `DEFERRED_WAKES` is bounded at 256. Full ⇒ equivalent wake already pending, drop is safe. |
| D3 | `interrupt_ack()` re-arms the GIC source AND clears `IRQ_PENDING`. |

---

## 7. Context Switch (`cond_yield_lp`)

**Sources:** `crates/catten/src/cpu/isa/{aarch64,x86_64}/lp/ops.rs`

```
cond_yield_lp():
  mask_interrupts
  ├── ctx_switch_pending?
  │   NO  → unmask, return
  │   YES →
  │   ├── next() selects next tid
  │   │   ├── no runnable → await_interrupt (log + wfi)
  │   │   └── got tid:
  │   │       ├── capture raw ptrs to saved_sp, on_cpu (under MASTER_THREAD_TABLE.write())
  │   │       └── clear_ctx_switch_pending() [re-arms quantum if !is_idle]
  │   └── switch_ctx(curr_sp, next_sp, curr_on_cpu, next_on_cpu)
  │       ├── save callee-saved + TTBR0 → *curr_sp
  │       ├── release *curr_on_cpu = 0   [SMP handoff publish]
  │       ├── acquire-wait *next_on_cpu == 0, claim = 1
  │       ├── restore from *next_sp
  │       └── ret → [resume in incoming thread]
  ├── reap_dead_threads()
  └── unmask_interrupts (if were enabled)
```

### Critical invariant

**ALL locks released before `switch_ctx`.**  Raw pointers to `saved_sp` and
`on_cpu` are captured under `MASTER_THREAD_TABLE.write()`, then dereferenced
lock-free inside `switch_ctx`.  Holding any lock across `switch_ctx` would
leak it (the outgoing thread's stack is abandoned).

### SMP thread handover (`on_cpu` field)

```
Thread T switching out on LP A      Defensive cross-LP ownership check
──────────────────────────────      ─────────────────────────────────
switch_ctx saves T's context
  writes *curr_sp = saved SP
  stlrb wzr, [curr_on_cpu] ──────────►   ldaxrb + loop until 0
  (release-store 0)                          (acquire-wait)
  [T is now free]                        stxrb 1, [next_on_cpu]
                                         (claim ownership)
                                         restore from *next_sp
```

Automatic and wake-path load migration remain disabled. `affinity_lp` keeps
ordinary threads on their established LP, while `pinned_lp` is a hard
constraint for explicit shard placement.

The scheduler supports one deliberately narrow rebalancing operation:
`try_rebalance()` may move an explicitly certified, queued `Ready` thread from
the busiest LP to the least-loaded LP when their load differs by at least two.
Certification is opt-in through `spawn_migratable_thread`; ordinary
`spawn_thread` and explicit `submit_to_lp` work are non-migratable. Running and
Blocked threads are never candidates because their CPU hand-off or LP-local
timer/resource ownership may still be live.

Migration locks both LP schedulers in numeric LP order and then the master
thread table. It revalidates generation, state, pin and certification before
moving the queue handle and updating `ThreadState::Ready(destination)` plus
soft affinity as one transaction. It currently runs only at the boot quiescent
point. There is no departure-, periodic-, or wake-driven work stealing. In
particular, timer and CQ wake admission return to the established affinity LP.
A tested departure trigger was rejected because wake-before-save can make a
still-on-CPU thread appear `Ready`. The `on_cpu` protocol prevents concurrent
restoration but does not make such migration preserve timer affinity.

`Thread::migration_constraints` now records typed ownership reasons rather
than relying only on the spawn-time `migration_safe` promise. The first wired
constraints are general scheduler waits, timer waits, CQ waits, and endpoint
waits. `block_thread_with_constraint` installs the appropriate bit while the
thread is Blocked; the LP scheduler clears the temporary blocking bits only on
the validated `Blocked -> Ready` transition. A migration candidate must also
have an empty constraint mask and an acquire-load of AArch64 `on_cpu == 0`.
The lifecycle regression checks after every timer wake that the timer
constraint has cleared while the running context is still rejected by the
`on_cpu` condition.

Runtime policy also has a low-pass entry point,
`try_rebalance_sustained(now_millis)`. The default 100 ms window is adjustable
at runtime through `set_rebalance_window_millis`. A sample starts a window for the current
busiest/least-loaded LP pair; balance recovery or a different pair resets it.
Only a pair that remains at least two runnable threads apart for the entire
window can reach transactional migration. No wake or interrupt path invokes
it. An experimental 10 ms maintenance sleeper was removed after consecutive
HVF runs missed different service/device gates despite performing no runtime
migrations: adding another periodic timer thread during boot was itself too
disruptive. Device domains and deferred workers remain default-denied because
their ownership is recorded per address space or worker rather than attributed
to a certified migratable thread.

A second experiment sampled from LP0 after context switching and dead-thread
reaping, avoiding an extra thread and timer. Reading queue depth by locking
every LP still perturbed the device gate, so each round-robin scheduler now
publishes its runnable count to an atomic per-LP summary whenever its queue or
current thread changes. LP0's rate-limited sample reads only those atomics; LP
locks are acquired only after an imbalance survives the full low-pass window
and transactional migration is actually attempted. x86_64 sampling remains
disabled until it has an `on_cpu` ownership protocol.

The regression suite starts a delayed runtime workload after the early service,
device and timer-affinity gates: three certified compute-only workers are
soft-placed on LP0 and cooperatively yield. They remain runnable long enough to
cross the sustained window, require `REBALANCE_SUCCESSES` to advance beyond the
boot migrations, then terminate. This exercises the automatic path without
mistaking the boot quiescent rebalance for runtime evidence.

The timer-lifecycle workers use a narrower certification lifetime: they may be
moved while queued at boot, then clear `migration_safe` on first dispatch before
starting their 384 fixed-affinity timer cycles. This prevents the runtime
sampler from invalidating the timer-affinity regression while leaving runtime
migration to the compute-only workload.

---

## 8. Lock Ordering Rules

The canonical lock order (outermost first):

```
SYSTEM_SCHEDULER.read()
  → lp_scheduler.lock()
    → MASTER_THREAD_TABLE.write()
```

This is the scheduler's nested order, not one global chain containing every
subsystem lock. Completion paths release `COMPLETIONS.write()` before
signalling observers, and device IRQ delivery defers lock-taking work to
thread context.

| Rule | Description |
|------|-------------|
| LO1 | **Do not signal observers while holding `COMPLETIONS.write()`.** Update CQ state, release it, then signal so the Waker chain can enter scheduler locks. |
| LO2 | **lp_scheduler before MASTER_THREAD_TABLE.** Enforced by `add_thread`, `next`, `block_thread`, `abort_thread`. Reversed order deadlocks `RoundRobin::next`. |
| LO3 | **No read-guard held across write-guard.** `abort()` and `sleep()` bind `tid` in a temp scope, drop all guards, then proceed. The `RwLock` is non-reentrant. |
| LO4 | **ALL locks dropped before `switch_ctx`.** Raw pointers captured under lock, dereferenced lock-free. |
| LO5 | **deliver_interrupt (IRQ context) takes NO locks.** Only atomics + MMIO. Deferred wake crosses to thread context via lock-free `ConcurrentQueue`. |
| LO6 | **Timer queue/comparator mutation masks local IRQs.** The timer IRQ handler accesses the queue directly only because exception entry already masks IRQs. |

---

## 9. Idle-Load Behaviour

### When the system should idle

An LP enters `wfi` (idle) when:
1. `run_queue` is empty (no real runnable threads)
2. No threads are `Ready` on this LP
3. No deferred wakes are pending for threads on this LP
4. `is_idle == true` — quantum timer not armed; LP waits for IPI or interrupt

### When the system fails to idle

| Symptom | Root Cause | Fix |
|---------|-----------|-----|
| LP dispatches >100/sec indefinitely | A thread is continuously runnable (busy-spins in `yield_lp()` loop) | Thread must block (`sleep()`, `wait()`, `block_thread()`) instead of yielding |
| `cq_wait` returns immediately after first completion | Historical implementation used undrained ring occupancy as readiness | Fixed: per-CQ work generations represent only new readiness |
| `is_idle` is false but only idle thread runs | `next()` requeues the sole runnable thread instead of selecting idle; thread never blocks | Thread must transition to `Blocked`, not stay in `Ready`/`Running` |
| Timer never fires after LP goes idle | `clear_ctx_switch_pending()` skips quantum re-arm when `is_idle`; next wake depends on IPI or device IRQ | This is CORRECT behaviour — idle LP is woken by admission IPI |
| Timer events stranded, LP stays in `wfi` forever | Queue/comparator state diverged, or an immediately due IRQ re-entered queue mutation | Keyed quantum ownership plus interrupt-masked enqueue and idle reconciliation |

### Expected long-lived services after boot tests

The remaining service threads should block rather than remain continuously
runnable:

| Thread | Source | State after test | Fix applied |
|--------|--------|-----------------|-------------|
| Raft nodes (2) | `raft.elf` | `wait(timer, 25ms)` — properly blocks; ~80 wakes/sec total | Yes |
| Echo gen-3 | `echo.elf` | `ipc_recv_block` — blocks until message arrives | Yes |
| UART driver | `uart.elf` | Killed by teardown (does NOT persist) | Yes (10ms timer) |
| Name services (multiple) | `ns.elf` | `ipc_recv_block` — blocks until message arrives | Already correct |
| Raft verifier | kernel | `sleep(10ms)` — exits after SUCCESS | Yes |
| Other verifiers | kernel | Blocking wait or bounded polling, then return through trampoline | Exit after SUCCESS |

The system should reach low CPU in steady state. The principal periodic work is
the Raft nodes' timer wakeups; idle LPs carry no round-robin quantum. The
bounded runner also requires the 128-cycle scheduler timer/affinity lifecycle
gate.
