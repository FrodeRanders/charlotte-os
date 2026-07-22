# CharlotteOS Scheduler Investigation — Full Report

## Timeline

1. **CQ ring undrained-entry problem** — `cq_wait` returns immediately after first completion because `complete()` posts ring entries that are never drained by callers using `poll(cap)`.  Fixed by replacing `wake_pending`/`cq_pending()` in `wait_on_cq` with a per-CQ monotonic work-generation counter (`work_generation`/`last_seen_generation`).  Every `complete()`, `complete_detached()`, and `wake()` bumps the counter; `wait_on_cq` blocks until the counter advances.

2. **Cap-0 rejection bug** — `if election_timer != 0` treated cap 0 (a valid `IdTable` slot) as "no timer."  The raft nodes silently skipped every timer tick.  Fixed by removing the zero-check: `poll()` correctly returns `u64::MAX` on an empty table.

3. **Raft election liveness** — `handle_vote_request` manually set `state = Follower` without calling `step_down()`, leaving `timeout_at_millis` stale.  The node immediately timed out again, causing rapid election cycling with neither reaching quorum.  Fixed by calling `step_down(req.term, current_millis)` which properly resets the deadline.

4. **LP affinity** — `submit_woken_thread` and `submit_ready_thread` were picking `get_least_loaded_lp()` on every re-admission, bouncing ordinarily admitted threads across LPs. This caused timer-on-wrong-LP bugs and TLB churn. Fixed by adding soft `affinity_lp` placement, set on first ordinary assignment and preferred on wake. Explicit `submit_to_lp()` now also records a separate hard `pinned_lp` for future placement policy.

5. **Orphan EL0 test threads** — device probe thread (`probe_device_topology`) had `loop { yield_lp() }` after logging.  Fixed by removing the loop (trampoline calls `abort()` on return).  Two EL0 payload threads (TID=4, TID=25) appeared not to reach `svc #8 (THREAD_EXIT)` despite having the correct instruction encoding.  Verifiers now perform idempotent teardown after observing the committed result. This prevents a payload exit failure from becoming permanent scheduler load without claiming to explain the underlying exit anomaly.

6. **Rebalancing remains disabled** — a transactional `Ready`-only version was tested after the timer repair, but a repeat HVF boot stopped at 0.093 seconds when admission order changed. `Ready` is not proof that a thread has no LP-local CQ, endpoint, or device relationship. Automatic migration was removed again; it must wait for explicit migratability/resource-ownership metadata. Stable affinity and hard pin semantics remain.

## Investigation Methodology

### Scheduler instrumentation (`SCHED_TRACE`)

Per-file compile-time trace guards added to:
- `system_scheduler/mod.rs` — admission decisions (TID, LP, load before/after)
- `round_robin.rs` — ready-queue operations (add, dispatch, requeue decisions)
- `scheduler/mod.rs` — yield_lp drain point
- `threads/waker.rs` — wake notifications
- `device/mod.rs` — interrupt delivery and deferred wakes

Toggled by `const SCHED_TRACE: bool = true/false;` per file.  Enabled on-demand for investigation, disabled for normal runs to avoid serial-port perturbation.

### In-memory debug trace buffer (`debug_trace.rs`)

A lock-free ring buffer (16384 entries, `#[repr(C)]`) in `.bss` at symbol `DEBUG_TRACE`.  Each entry records:
- `tick: u64` — ARM Generic Timer CNTPCT_EL0 (62.5 MHz, never stops)
- `lp: u64` — which LP captured the event
- `tag: u64` — well-known event tag
- `a, b, c: u64` — context-dependent payload

Well-known tags:
| Tag | Meaning | a | b | c |
|-----|---------|---|---|---|
| `CQ_WAIT_ENTER` | About to block in `wait_on_cq` | asid | work_generation | last_seen_generation |
| `CQ_WAIT_RESUME` | Woke from `yield_lp` | asid | work_generation | last_seen before update |
| `CQ_WAIT_FAST` | Fast-path return (no block) | asid | work_generation | last_seen before update |
| `COMPLETE` | Timer/worker completion | asid | new work_generation | cap |
| `WAKE` | Explicit `completion::wake()` | asid | new work_generation | cq |
| `SIGNAL_CQ` | `signal_cq()` called | asid | cq | observer count |
| `WAKER_NOTIFY` | Thread Waker fired | tid | generation | 0 |
| `SUBMIT_TIMER_OK` | Timer submitted successfully | asid | cap | timeout_ms |
| `TIMER_FIRED` | Timer event processed | 0 | 0 | 0 |
| `TIMER_ARMED` | Hardware comparator armed | 0 | deadline | 0 |
| `TIMER_STOPPED` | Hardware comparator stopped | 0 | 0 | 0 |

Events are read post-mortem via `dump_after(ms)` which spawns a kernel thread that sleeps for `ms` milliseconds, then prints the buffer via `logln!()`.  Serial writes perturb timing, so the dump is deferred until after the test burst.

This tool was instrumental in diagnosing:

### Discovery 1: Cross-LP timer migration (run 2)

The trace showed LP migration causing observer-not-found races:

```
tick=373M lp=0  CQ_WAIT_ENTER   a=0x19 b=0x0 c=0x0   ← enters wait on LP0
tick=373M lp=0  COMPLETE        a=0x19 b=0x1 c=0x0   ← timer fires on LP0
tick=373M lp=0  SIGNAL_CQ       a=0x19 b=0x0 c=0x1   ← 1 observer notified
tick=373M lp=3  CQ_WAIT_RESUME  a=0x19 b=0x1 c=0x0   ← wakes on **LP3** (migrated!)
tick=374M lp=3  COMPLETE        a=0x19 b=0x2 c=0x1   ← second timer on LP3
tick=374M lp=3  SIGNAL_CQ       a=0x19 b=0x0 c=0x0   ← **0 observers** on LP3
tick=375M lp=0  CQ_WAIT_FAST    a=0x19 b=0x2 c=0x1   ← migrated back to LP0
tick=375M lp=0  CQ_WAIT_ENTER   a=0x19 b=0x2 c=0x2   ← blocks forever
```

The thread migrated from LP0→LP3 between the first and second timer ticks.  The Waker registered on LP0 was consumed by the first wake; on LP3, `signal_cq` found zero observers because no new Waker had been registered yet.  The thread eventually migrated back to LP0 and registered a new Waker, but by then the second timer's signal was lost and the third timer never fired.  This led directly to LP affinity.

### Discovery 2: All LP comparators stop (run 3)

After applying LP affinity, cross-LP migration was eliminated, but the trace revealed a deeper problem:

```
LP0 COMPLETE events:  tick=359M..375M  (0.26s window)
LP1 COMPLETE events:  tick=369M..471M  (1.6s window)
After tick 376M:      ZERO COMPLETE events on any LP
```

All LPs' generic timer comparators stopped firing after ~0.4s.  New timers submitted by the raft nodes (via `submit_timer`) were adding events to the queue, but the PPI never fired to process them.  The `COMPLETE` trace events (which fire inside `complete()`) stopped appearing.  The `SUBMIT_TIMER_OK` trace (which fires inside the `submit_timer` syscall) confirmed the timer events WERE being submitted.  They just never fired.

The `TIMER_FIRED`/`TIMER_ARMED`/`TIMER_STOPPED` traces were added to narrow this down.  The traces initially flooded the buffer (quantum ticks fire every 10ms on every LP, ~400 events/sec), so the buffer size was increased from 4096 to 16384 entries and the dump delay was reduced from 10s to 4s.

### Discovery 3: Cap-0 rejection (run 4)

With the timer traces in place, the raft nodes showed:

```
tick=370M lp=0 SUBMIT_TIMER_OK  a=0x19 b=0x0 c=0x19   ← cap 0, timeout 25ms
tick=372M lp=0 SUBMIT_TIMER_OK  a=0x1a b=0x0 c=0x19   ← cap 0
tick=372M lp=0 COMPLETE         a=0x19 b=0x1 c=0x0    ← timer fires
tick=374M lp=0 COMPLETE         a=0x1a b=0x1 c=0x0    ← timer fires
--- NO MORE EVENTS for asid=0x19 or 0x1a ---
```

The timers were being submitted with cap 0 — a valid `IdTable` slot when the table is empty.  But the raft node's loop checked `if election_timer != 0` before polling.  `0 != 0` is false, so the timer was never processed, time never advanced, and no new timer was submitted.  The first timer from `submit_timer(LOOP_TICK_MS)` before the loop DID fire (the COMPLETE events above), but the loop never acknowledged it.

### Discovery 4: EL0 payload exit failure (steady-state dispatch)

With SCHED_TRACE enabled, the dispatch pattern at +23s post-boot showed:

```
LP0: TIDs 4, 27, 49 — all requeue=true, never blocking
LP1: TIDs 25, 47
LP2: TID 8
LP3: TIDs 43, 48
~270 dispatches/sec across 3 busy LPs
```

These are EL0 test payload threads that never exit.  The `probe_device_topology` thread (TID=44) had an explicit `loop { yield_lp() }` — fixed by removing the loop (the thread returns, trampoline calls `abort()`).

For the hand-written assembly payloads, instrumentation was added to `sys_thread_exit` and `abort()`:

```
THREAD_EXIT syscall: asid=6 tid=Some(8)   ← EL0 IPC test payload: EXITS ✓
THREAD_EXIT syscall: asid=7 tid=Some(10)  ← EL0 IPC block test: EXITS ✓
--- NO THREAD_EXIT syscall for asid=5 --- ← EL0 SVC test (TID=4): NEVER EXITS ✗
```

The `svc #8` encoding was verified correct — both the hand-written bytes (`0x08, 0x01, 0x00, 0xd4`) and the Rust inline asm (`asm!("svc #8", ...)`) produce the identical instruction `0xd4000108`.  The `svc #1` (SUBMIT) instruction in the same payload DOES reach the kernel (the verifier confirms the result page was written).  The thread reaches `svc #1`, writes the result page, but never executes the `svc #8` that follows.

**Hypothesis:** The `str` instruction between `svc #1` and `svc #8` writes to a user page at EL0.  On QEMU TCG, this may trigger a recoverable translation fault.  The AArch64 abort handler recovers by invalidating the TLB and retrying the faulting instruction.  The write eventually succeeds (the verifier confirms it), but the CPU state after recovery may not correctly advance to the next instruction.

### Discovery 5: Verifier cleanup experiment eliminates all CPU load

After adding `abort_thread()` calls to the verifier functions (killing the EL0 payload threads after confirming test success), the steady-state dispatch rate dropped to **zero on all LPs**:

```
LP0 (t>10s): {}   ← no dispatches
LP1 (t>10s): {}
LP2 (t>10s): {}
LP3 (t>10s): {}
```

This demonstrated that those payloads accounted for the residual load. The
cleanup is test teardown, not evidence that the payload's own `THREAD_EXIT`
lifecycle is correct; the exit anomaly remains a separate investigation item.

## Final State

All 15 self-tests passed in the instrumented investigation run. Key changes
retained on `dev` are:

| Component | Before | After |
|-----------|--------|-------|
| `cq_wait` blocking mechanism | `cq_pending()` checks undrained ring → busy-spin | Per-CQ work-generation counter, decoupled from ring |
| Thread admission | `get_least_loaded_lp()` on every wake → LP migration | `pick_lp_for()` uses affinity LP set at first assignment |
| Raft timer polling | `if election_timer != 0` rejects valid cap 0 | Always poll; `poll(0)` returns u64::MAX on empty table |
| Raft election timeout | Manual field reset without `step_down()` → stale deadline | `step_down()` properly resets `timeout_at_millis` |
| Device probe thread | `loop { yield_lp() }` → permanent CPU burn | Returns normally; trampoline calls `abort()` |
| EL0 test payloads | Some payloads appeared not to exit | Idempotent verifier teardown; lifecycle cause remains open |
| LP idle load | 3 LPs at ~270 dispatches/sec in the affected run | Zero in the instrumented teardown run; revalidate after scheduler changes |

## Files Modified

| File | Change |
|------|--------|
| `completion/mod.rs` | `CqState` with `work_generation`/`last_seen_generation`; `wait_on_cq` uses generation comparison |
| `completion/cq.rs` | `drain()` method on `CompletionQueueRing` |
| `scheduler/threads/mod.rs` | `affinity_lp: Option<LpId>` on `Thread` |
| `scheduler/system_scheduler/mod.rs` | `pick_lp_for()`, affinity-aware `submit_ready_thread`/`submit_woken_thread` |
| `scheduler/mod.rs` | Scheduler yield and lifecycle handling |
| `syscall/mod.rs` | Timer submission and completion ABI |
| `catten-graft/src/node.rs` | `step_down()`, `become_leader()` (no-op entry), `handle_vote_request`, `handle_append_entries` |
| `catten-services/src/bin/raft.rs` | Cap-0 fix (remove `!= 0` guard) |
| `main.rs` | Remove infinite yield loop from `probe_device_topology` |
| `self_test/el0.rs` | Idempotent verifier teardown of the completed payload |
| `self_test/el0_demo.rs` | Idempotent verifier teardown of the completed coordinator |
| `debug_trace.rs` | Lock-free in-memory trace buffer (development tool, no-op stub on dev) |
| `docs/scheduler-state-machines.md` | Formal state machine reference |
| `scripts/run-aarch64.sh` | Always rebuild EL0 services before kernel build |
| `scripts/build-catten-services.sh` | `--clean` flag |

## Follow-up: quantum ownership and migration boundary (2026-07-22)

A later HVF run stopped with several genuinely runnable threads on every LP.
The immediate defect was split ownership of the round-robin quantum: an
`armed` atomic could remain true after the corresponding `TimerEvent` was no
longer in the per-LP queue. Subsequent dispatches then declined to queue a new
quantum, stranding ready work indefinitely.

The timer queue now owns the invariant. Scheduler quanta are keyed singleton
events: `ensure_event` adds one only when absent, preserving its deadline over
voluntary yields, and idle transition removes it. There is no second `armed`
state that can disagree with the queue.

Placement now has two explicit levels:

- `affinity_lp` is a soft home assigned on first admission. Blocked threads
  wake there, keeping per-LP timer ownership and cache locality stable.
- `pinned_lp` is a hard constraint established by `submit_to_lp` for shard and
  other LP-local work.
- Automatic migration remains disabled. A `Ready` thread can still retain an
  LP-local CQ, endpoint, or device relationship, so state alone cannot prove
  migratability.

With timer liveness restored, low-frequency reply/test polling was converted
from runnable yield loops to 1 ms blocking timer waits. A 4-LP AArch64 HVF run
then observed every required deferred marker: UART at 0.131 s, Raft at 0.364 s,
and CQ wait at 0.506 s. The diagnostic snapshot used during development showed
an LP entering the real idle path; it was removed after validation. A later
repeat exposed the migration limitation above, so automatic rebalancing was
removed before the final validation series.

A dedicated scheduler-lifecycle gate performs 128 one-millisecond timer
block/wake cycles while checking LP affinity. Initially, merely adding that
worker reliably reproduced the early stall at approximately 0.09 seconds.
This first appeared to be interrupt re-entry during timer queue mutation.
Closer inspection showed that the queue's kernel `RwLock` guard already saves
and masks local IRQ state, however, so interrupt re-entry through that guard is
not an established root cause.

All non-scheduler timer insertion now goes through `enqueue_event`, which
preserves the caller's interrupt state and masks local interrupts across the
queue mutation and comparator programming. This makes the required critical
section explicit and is useful defensive synchronization, but duplicates the
current lock guard's masking and must not be treated as the causal fix. With
that change, the reproducer
completed all 128 wakes on its original LP at 0.235 seconds, and the complete
SMP4 HVF suite also passed. The lifecycle marker is now required by the bounded
AArch64 runner.

The resulting per-LP timer design follows the classic one-hardware-timer,
many-logical-timers model: logical events remain sorted by absolute deadline;
the hardware comparator represents the queue head; an interrupt removes and
signals every event due at the sampled current time; and the comparator is
then armed from the new head or stopped for an empty queue. Queue mutation and
comparator programming are one local interrupt-masked transaction. The idle
loop uses the same protected reconciliation wrapper; only the timer IRQ handler
calls `process_events` directly, because exception entry already masks IRQs.

An attempted extension of the scheduler lifecycle gate armed four logical
events at 2, 4, 7, and 11 milliseconds on one LP. With per-interrupt queue and
comparator tracing enabled, all four events fired in deadline order and the
complete deferred test suite passed. The same binary rebuilt without tracing
occasionally stopped making progress around 90 milliseconds, before the four
events were armed; only the earliest EL0 marker was then present. Consequently
the extension was not retained as a gate. This is evidence that the sorted
timer queue can drive one comparator through multiple deadlines, but it also
exposes a remaining timing- or layout-sensitive early scheduler liveness issue.
The tracing changes scheduling enough to hide that issue, so a future
investigation should use a bounded in-memory trace rather than synchronous
serial logging.

A subsequent uninstrumented run with only the committed lifecycle gate reached
its 128-wake success marker and Raft success, then remained silent with only
`[cq wait] SUCCESS` missing. This confirms that the liveness problem is not
created by the four-deadline extension. It can occur both during early boot and
later while the CQ driver is progressing through repeated one-millisecond
sleeps. The current HVF gate must therefore still be considered timing
sensitive despite successful runs; the next diagnostic should capture timer
head, comparator state, current TID, and ready-queue transitions into a fixed
memory ring that can be dumped after an independent watchdog trigger.
