# CharlotteOS Scheduler State Machines

This document defines the interacting state machines that govern thread
scheduling, completion-queue (CQ) wait semantics, and the idle-load
properties of the system.  It is intended as a reasoning tool: every
scheduler change should be checked against these invariants.

---

## 1. Thread State Machine (`ThreadState`)

**Source:** `crates/catten/src/cpu/scheduler/threads/mod.rs:95-101`

```
                        spawn_thread()
                             │
                    ┌────────▼──────────┐
                    │ NeedsLpAssignment │
                    └────────┬──────────┘
                             │ submit_ready_thread()
                             │  → get_least_loaded_lp()
                             │  → add_thread()
                    ┌────────▼──────────┐        block_thread(self)
                    │   Ready(lp_id)    │◄─────────────────────────┐
                    └──┬────────────▲───┘                          │
          next()       │            │ block_thread(remote)         │
    pops from run_queue│            │ remove_thread()              │
                       │            │                              │
               ┌───────▼────────┐   │    ┌───────────────────────┐ │
               │ Running(lp_id) ├───┘    │   Blocked(Arc<Waker>) │─┘
               └───┬───────┬────┘         └───────────┬───────────┘
                   │       │                         │
       next()      │       │ abort_thread()          │ Waker::notify()
    requeue as     │       │   → stage_dead          │  → submit_woken_thread()
    Ready(same LP) │       │   → reap later          │  → validate generation
                   │       ▼                         │  → add_thread()
                   │   ╔══════╗                      │
                   │   ║ DEAD ║                      │
                   │   ╚══════╝                      │
                   │                                 │
                   └─────────────────────────────────┘
```

### Invariants

| # | Invariant |
|---|-----------|
| T1 | `Running(lp)` ⇒ thread IS `current_handle` of LP `lp`. |
| T2 | `Ready(lp)` ⇒ thread IS in `run_queue` of LP `lp`; NOT any `current_handle`. |
| T3 | `Blocked` ⇒ thread is NOT in any `run_queue`. Transiently it may still be `current_handle` (during the `block_thread → cond_yield_lp` window). |
| T4 | `idle_tid` is never in `run_queue` and never `Blocked`. |
| T5 | Every scheduling op validates `generation` to prevent stale-handle-after-slot-reuse. |
| T6 | `add_thread()` for an already-`Running` or already-`Ready` thread is a benign no-op (aggregated wakes before the thread parks). |

### Lock order for transitions

```
spawn_thread:         MASTER_THREAD_TABLE.write() → SYSTEM_SCHEDULER.read() → lp_scheduler.lock()
block_thread (self):  MASTER_THREAD_TABLE.read() → SYSTEM_SCHEDULER.read() → MASTER_THREAD_TABLE.write()
block_thread (Ready): SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
submit_*:             SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
abort_thread:         MASTER_THREAD_TABLE.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
cond_yield_lp:        SYSTEM_SCHEDULER.read() → lp_scheduler.lock() → MASTER_THREAD_TABLE.write()
                      ALL DROPPED before switch_ctx (otherwise deadlock — abandoned stack).
```

The global lock order is: **lp_scheduler → MASTER_THREAD_TABLE**.  `block_thread` for self-block goes `MASTER_THREAD_TABLE` first (no LP lock needed), then `yield_lp` re-acquires in the canonical order. The `abort()` and `sleep()` callers bind `tid` in a temporary scope, drop all guards, then proceed — this avoids holding a read guard across a write for the non-reentrant `RwLock`.

---

## 2. LP Scheduler State (`RoundRobin`)

**Source:** `crates/catten/src/cpu/scheduler/lp_schedulers/round_robin.rs:64-82`

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

### Quantum timer sub-state (`TimerEventObserver`)

```
  armed=false ── set_next_timer_event() ──► armed=true
       ▲                                        │
       │       TimerEventObserver::notify()     │
       └────────────────────────────────────────┘
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
| L3 | At most one quantum event is in flight per LP (CAS on `armed`). |
| L4 | `cond_yield_lp` with `curr_tid == next_tid` (sole runnable) still calls `clear_ctx_switch_pending()` to re-arm the timer. Without this, `sleep()` freezes. |
| L5 | The idle loop (`lp_idle_loop`) calls `drain_deferred_wakes()` and `process_events()` before `cond_yield_lp()` — so a deferred wake that admits a same-LP thread is processed before entering `wfi`. |

### What makes an LP idle vs busy

| LP goes IDLE | LP goes BUSY |
|---|---|
| `next()` selects `idle_tid` because `run_queue` is empty | `submit_*()` or IPI admits a thread, sets `pending`, `next()` picks it |
| All real threads have blocked, aborted, or been removed | `drain_deferred_wakes()` → `completion::wake()` → `submit_woken_thread()` |

---

## 3. Completion Operation Lifecycle (`OpState`)

**Source:** `crates/catten/src/completion/mod.rs:158-168`

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

**Source:** `crates/catten/src/completion/cq.rs:63-69`

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
    ┌── take_wake(asid, cq) || cq_pending(asid, cq) >= min_complete
    │   → return immediately  (ring entries count as "pending")
    │
    ├── block_thread(tid, &observable)
    │
    ├── guard: peek_wake(asid, cq) || cq_pending(...) >= min_complete
    │   → submit_ready_thread(tid)   [lost-wake guard]
    │
    └── yield_lp() → blocks until woken → loop
```

| Condition | Returns? | Consumed by |
|-----------|----------|-------------|
| `take_wake()` is true | Yes | `wake()` sets `wake_pending`; consumed once per waiter pass |
| `cq_pending()` >= min_complete | Yes | Ring entry count (includes undrained entries!) |
| Neither | Blocks | Thread sleeps; woken by CQ observer signal |

### The undrained-ring problem

Every `complete()` posts a ring entry. If the caller polls the completion via
`poll(cap)` and never reads the shared ring, the entry persists.  `cq_pending()`
returns the undrained count.  After the first completion, `cq_wait` **always**
finds `pending() >= 1` and returns immediately — the event loop degenerates
into a busy-spin.

**Known workaround:** Services that poll individual completions must NOT use
`cq_wait`.  Use `COMPLETION_WAIT` on a specific timer or `ipc_recv_block` on
the endpoint.

**Proposed kernel fix (pending):** Track a `last_pending` watermark per CQ.
Only entries that arrived *since* the last `cq_wait` return count as "new."
Reset the watermark when userspace drains the ring (observed as
`pending < last_pending`).

### Invariants

| # | Invariant |
|---|-----------|
| Q1 | `capacity >= 2` (one sentinel slot to distinguish full from empty). |
| Q2 | At most `capacity - 1` entries are in-flight simultaneously. |
| Q3 | Ring write happens-before observer notification (`complete()` posts ring, then signals). |
| Q4 | `drain()` (`tail = head`) requires `&mut self` — exclusive access. |

---

## 5. CQ Waiter State (`CqState`)

**Source:** `crates/catten/src/completion/mod.rs:406-423`

### `wake_pending` — consume-on-wait semantics

```
  false ── wake() ──► true ── take_wake() ──► false
    ▲                   │
    │   wake()          │  (idempotent if already true)
    └───────────────────┘
```

- `wake()`: sets `wake_pending = true` + calls `signal_cq()` (wakes blocked waiters)
- `take_wake()`: atomically consumes the flag (once per waiter pass)
- `peek_wake()`: non-consuming read for lost-wake guard

### Lost-wake race closure

```
1. take_wake() → false        [fast-path miss]
2. block_thread()             [register Waker]
3.  ◄── wake() fires here ──  [race window]
4. yield_lp()                 [would sleep forever]
5.  BUT: guard at (3): peek_wake() → true → submit_ready_thread()
   → thread re-admitted, doesn't yield, loops back to (1) → take_wake() → true
```

### Invariants

| # | Invariant |
|---|-----------|
| W1 | `wake_pending == true` ⇒ every subsequent `wait_on_cq` iteration returns immediately (short-circuit). |
| W2 | Coalescing: N `wake()` calls between two waiter passes → exactly 1 `take_wake()` success. |
| W3 | Lost-wake guard (`peek_wake`) observes a wake posted during the `block_thread` → observer-registration window. |

---

## 6. Device Interrupt Delivery

**Source:** `crates/catten/src/device/mod.rs:169-178,507-554`

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
        → CqState.wake_pending = true   [COMPLETIONS.write()]
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

**Source:** `crates/catten/src/cpu/isa/aarch64/lp/ops.rs:174-251`

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
Thread T on LP A                    Thread T migrating to LP B
─────────────────                   ──────────────────────────
switch_ctx saves T's context
  writes *curr_sp = saved SP
  stlrb wzr, [curr_on_cpu] ──────────►   ldaxrb + loop until 0
  (release-store 0)                          (acquire-wait)
  [T is now free]                        stxrb 1, [next_on_cpu]
                                         (claim ownership)
                                         restore from *next_sp
```

---

## 8. Lock Ordering Rules

The canonical lock order (outermost first):

```
COMPLETIONS.write()
  → DEVICES.lock()
    → SYSTEM_SCHEDULER.read()
      → lp_scheduler.lock()
        → MASTER_THREAD_TABLE.write()
          → DEAD_THREADS.write()
```

| Rule | Description |
|------|-------------|
| LO1 | **COMPLETIONS before SYSTEM_SCHEDULER.** `wake()` takes `COMPLETIONS.write()` then Waker chain enters scheduler locks. |
| LO2 | **lp_scheduler before MASTER_THREAD_TABLE.** Enforced by `add_thread`, `next`, `block_thread`, `abort_thread`. Reversed order deadlocks `RoundRobin::next`. |
| LO3 | **No read-guard held across write-guard.** `abort()` and `sleep()` bind `tid` in a temp scope, drop all guards, then proceed. The `RwLock` is non-reentrant. |
| LO4 | **ALL locks dropped before `switch_ctx`.** Raw pointers captured under lock, dereferenced lock-free. |
| LO5 | **deliver_interrupt (IRQ context) takes NO locks.** Only atomics + MMIO. Deferred wake crosses to thread context via lock-free `ConcurrentQueue`. |

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
| `cq_wait` returns immediately after first completion | CQ ring entries are never drained; `cq_pending()` is always > 0 | Service must use `COMPLETION_WAIT` on a specific timer, or use `ipc_recv_block`, or the kernel must drain stale ring entries |
| `is_idle` is false but only idle thread runs | `next()` requeues the sole runnable thread instead of selecting idle; thread never blocks | Thread must transition to `Blocked`, not stay in `Ready`/`Running` |
| Timer never fires after LP goes idle | `clear_ctx_switch_pending()` skips quantum re-arm when `is_idle`; next wake depends on IPI or device IRQ | This is CORRECT behaviour — idle LP is woken by admission IPI |
| Timer events stranded, LP stays in `wfi` forever | Software timer queue had an event but hardware comparator wasn't re-programmed | Idle loop calls `process_events()` before `wfi` to reconcile |

### Current persistent-CPU sources (post-fix)

After the echo/raft/uart fixes (commit `bddab44`), the remaining threads that
stay runnable after their test completes:

| Thread | Source | State after test | Fix applied |
|--------|--------|-----------------|-------------|
| Raft nodes (2) | `raft.elf` | `wait(timer, 25ms)` — properly blocks; ~80 wakes/sec total | Yes |
| Echo gen-3 | `echo.elf` | `ipc_recv_block` — blocks until message arrives | Yes |
| UART driver | `uart.elf` | Killed by teardown (does NOT persist) | Yes (10ms timer) |
| Name services (multiple) | `ns.elf` | `ipc_recv_block` — blocks until message arrives | Already correct |
| Raft verifier | kernel | `sleep(10ms)` — exits after SUCCESS | Yes |
| Other verifiers | kernel | `yield_lp()` polling → return → trampoline calls `abort()` | Exit after SUCCESS (F16 resolved) |

The system should now reach near-zero CPU in the steady state after all
tests complete.  The only active work is the raft nodes' 25ms timer wakeups.
