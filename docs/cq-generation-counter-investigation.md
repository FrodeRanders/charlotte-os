# CQ Generation-Counter Investigation Report

## Branch: `sched/cq-generation-counter` (commit `a1d49e4`)

## Background

The `dev` branch uses service-level workarounds to prevent busy-spinning
in services that call `cq_wait` with undrained CQ ring entries (raft uses
`wait(timer)`; echo uses `ipc_recv_block`). The `sched/cq-generation-counter`
branch attempts a kernel-side fix: replace `wake_pending` + `cq_pending()`
in `wait_on_cq` with a per-CQ monotonic work-generation counter. Every
`complete()`, `complete_detached()`, and `wake()` bumps the counter;
`wait_on_cq` blocks until the counter advances past the value seen at
the last return.

This eliminates the undrained-ring problem at the root — services can
use the original `cq_wait` event-loop without busy-spinning.  The raft
nodes' timer completions advance the counter; endpoint messages (via
`wake()`) do as well.

## Problem

On the `sched/cq-generation-counter` branch, 14 of 15 self-tests pass,
but the Raft test fails: both nodes remain stuck as Followers
(`states 1/1`) indefinitely.  The Raft election fix (proper `step_down()`
on higher-term vote request) is present on this branch (cherry-picked
from `dev`, commit `af81268`), so the election logic itself is correct.
The issue is that the raft nodes' `cq_wait` calls never return after
the first timer tick — the second timer completion never wakes the thread.

## Investigation tool: `debug_trace` module

**File:** `crates/catten/src/debug_trace.rs`

A lock-free, in-memory ring buffer that records timestamped key-value
events without serial-port contention.  Implemented as a static `DebugTrace`
in .bss with an atomic write index.

**Capacity:** 4096 entries (trace events wrap silently when full).

**Entry format** (32 bytes, `#[repr(C)]`):
```
tick: u64    — ARM Generic Timer CNTPCT_EL0 (62.5 MHz, never stops)
tag:  u64    — well-known event tag (see constants)
a:    u64    — context-dependent payload
b:    u64    — context-dependent payload
c:    u64    — context-dependent payload
```

**Event tags and their payloads:**

| Tag | Meaning | a | b | c |
|-----|---------|---|---|---|
| `CQ_WAIT_ENTER` | About to block in `wait_on_cq` | asid | work_generation | last_seen_generation |
| `CQ_WAIT_RESUME` | Woke from `yield_lp` | asid | work_generation | last_seen before update |
| `CQ_WAIT_FAST` | Fast-path return (no block) | asid | work_generation | last_seen before update |
| `CQ_WAIT_GUARD` | Lost-wake guard fired | asid | work_generation | last_seen |
| `COMPLETE` | Timer/worker completion | asid | new work_generation | cap |
| `COMPLETE_DETACHED` | Detached completion | asid | new work_generation | operation |
| `WAKE` | Explicit `completion::wake()` | asid | new work_generation | cq |
| `SIGNAL_CQ` | `signal_cq()` called | asid | cq | observer count |
| `WAKER_NOTIFY` | Thread Waker fired | tid | generation | (unused) |

**Instrumentation points:** `wait_on_cq` (all branches), `complete()`,
`complete_detached()`, `wake()`, `signal_cq()`, `Waker::notify()`.

**Dump mechanism:** `dump_after(ms)` spawns a kernel thread that sleeps
for `ms` milliseconds, then prints the entire trace buffer to the serial
console via `logln!()`.  The dump is deferred until after the test burst
(10 seconds) so serial writes do not perturb test timing.

## Method

1. Cherry-pick the Raft election fix (`af81268`) from `dev` to
   `sched/cq-generation-counter`.
2. Build and embed the EL0 service bundle with the `cq_wait`-based
   raft event loop.
3. Run with `dump_after(10000)` — the trace thread dumps at +10s.
4. Search the dump for the raft nodes' address space IDs (AS=0x19
   and AS=0x1a in the captured run).

## Results

The trace contained 1009 events (capacity 4096, no wrap).  Filtering
for the raft nodes' ASIDs produced:

```
WAKER_NOTIFY   a=0x19 b=0x1a          ← thread TID=25 woken
CQ_WAIT_ENTER  a=0x19 b=0x0 c=0x0     ← enters first wait: gen=0, last=0
COMPLETE       a=0x19 b=0x1 c=0x0     ← first timer fires, gen bumped to 1
SIGNAL_CQ      a=0x19 b=0x0 c=0x1     ← 1 observer signalled (the Waker)
CQ_WAIT_RESUME a=0x19 b=0x1 c=0x0     ← wakes: gen=1, last was 0
CQ_WAIT_ENTER  a=0x19 b=0x1 c=0x1     ← second wait: gen=1, last=1 → blocks

[NO MORE EVENTS for AS=0x19]
```

The same pattern holds for AS=0x1a (the second raft node).

## Interpretation

1. **The generation counter correctly blocks `cq_wait`.**  The first
   call enters with `work==last_seen==0`, blocks, wakes on the first
   timer (`COMPLETE` bumps gen to 1), returns, and updates `last_seen`
   to 1.  The second call enters with `work==last_seen==1` and blocks
   as intended.

2. **The second timer never fires.**  After the raft node polls the
   first timer, closes it, and calls `election_timer = submit_timer(25)`,
   a new `TimerEvent` should be placed on the local LP's timer queue.
   25 ms later the PPI should fire, `process_events` should run, the
   `CompletionTimerObserver` should call `complete()`, which bumps the
   generation and traces `COMPLETE`.  This `COMPLETE` trace event does
   not appear for either raft node.

3. **The `complete()` path works for other address spaces.**  The trace
   contains `COMPLETE` and `COMPLETE_DETACHED` events for many other
   ASIDs throughout the 10-second window.  The infrastructure is
   functional.

4. **Other tests pass.**  14 of 15 self-tests succeed, including the
   `cq_wait` self-test (which exercises the generation-counter path
   directly).  Only Raft fails.

## Hypotheses for the missing second timer

1. **`submit_timer` fails silently (returns cap 0).**  If the completion
   capability table is at capacity (`WouldBlock`) or the address space
   has no table, `submit_timer` returns 0.  The raft node's code treats
   `election_timer == 0` as "no timer" and never polls it.  No new timer
   is submitted, and `cq_wait` blocks forever.

2. **The timer event is placed on the wrong LP's queue.**  `submit_timer`
   calls `TIMER_QUEUES.try_get_mut()` which depends on the calling LP.
   If the raft node was migrated to a different LP between the
   `CQ_WAIT_RESUME` and the `submit_timer` call, the timer goes to the
   new LP's queue.  If that LP's timer infrastructure is not properly
   armed, the event silently never fires.

3. **`try_get_mut` fails and the unwrap panics.**  If the local LP's
   timer queue mutex is held (e.g. by `process_events` in an interrupt
   that preempted the raft node), `try_get_mut` returns `Err`, and
   `unwrap_unchecked()` would be undefined behavior — potentially
   silently dropping the timer event.

## Recommended next steps

1. **Add a `TAG_SUBMIT_TIMER` trace point** in `submit_timer` to
   record whether the timer was successfully submitted and what LP
   it was placed on (`get_lp_id()`).

2. **Add a `TAG_TIMER_FIRED` trace point** in `CompletionTimerObserver`
   or `process_events` to record when a timer event is actually
   dispatched.

3. **Trace the raft node's return value from `submit_timer`** in the
   EL0 service (write it to the config page).

4. **Reduce the trace dump delay** to 3-4 seconds (after the test burst
   but before timer events accumulate enough to wrap the buffer) and
   confirm the buffer is not wrapping.

With these additional trace points, the exact point of failure in the
second-timer lifecycle can be pinpointed.
