# Async-syscall demonstration and scheduler findings

> This note records the end-to-end async-syscall demonstration
> (`crates/catten/src/demo.rs`) and the scheduler bugs it surfaced and fixed.
> It complements `docs/async-syscall-abi.md` (the ABI) and
> `docs/sitas-runtime-model.md` (the collaboration context).

## What the demo shows

`demo::spawn_async_syscall_demo()` is spawned from `bsp_main` (after the
scheduler is active) and exercises the completion ABI's core loop with real
kernel threads across real context switches:

1. **Coordinator** thread: opens a per-address-space completion table,
   `submit`s an operation (→ a `CompletionCap`), spawns a **worker**, then
   **blocks** in `completion::wait` on the capability.
2. **Worker** thread: performs asynchronous work (interleaved with other
   threads via `yield_lp`), then calls `completion::complete`, which signals the
   capability's observers and **wakes the coordinator**.
3. **Coordinator** resumes exactly where it blocked, `poll`s the result, and
   reports `COMPLETION RECEIVED, result Ok(42)` / `SUCCESS`.

Verified on QEMU AArch64: the loop completes with zero panics, and the boot
continues normally afterward (self-tests all still pass).

This is the "submit → async work → complete → wake" loop the whole async-syscall
ABI is built around, demonstrated for the first time with genuine blocking and
cross-thread wakeups — not just the single-threaded self-tests.

## Scheduler bugs found and fixed

Building the demo exercised the *block → yield → wake → resume* path for the
first time and surfaced two latent bugs that never fired before because no
kernel thread had ever blocked and later resumed:

### 1. Blocked threads lost their execution context (fixed)

`wait()` (and `sleep()`) block via `SystemScheduler::block_thread`, which called
`RoundRobin::remove_thread`. For the *currently running* thread that clears
`current_handle`, so the subsequent `cond_yield_lp` read `curr_tid = get_tid()`
as `None` and called `switch_ctx(null, next)` — which **skips saving the
blocking thread's context**. When the thread was later woken it restarted from
its entry point instead of resuming after `wait()`.

**Fix:** the currently-running thread that blocks itself now stays
`current_handle` until the yield saves its context; `RoundRobin::next` declines
to re-queue or re-`Ready` a `Blocked` thread. (See
`system_scheduler::block_thread` and `round_robin::next`.)

### 2. `abort_thread` double-locked `MASTER_THREAD_TABLE` (fixed)

`abort_thread` held a `MASTER_THREAD_TABLE.write()` guard across a *second*
`.write()`, a self-deadlock on the non-reentrant RwLock. It never fired because
no kernel thread had ever returned (they all loop). **Fix:** read the thread's
LP under a short-lived read lock, then take the write lock only for the removal.

## Known remaining issues (worked around in the demo)

The demo threads `park` (loop-yield) rather than returning, and simulate async
work with `yield_lp` rather than `sleep`, because two deeper issues remain:

### A. The per-LP timer stops firing when only one thread is runnable

The quantum timer is re-armed inside `RoundRobin::clear_ctx_switch_pending`,
which historically ran only on an actual context switch. Once the coordinator
and worker both block, `probe_device_topology` is the sole runnable thread, so
`cond_yield_lp` took the "no switch" path and never re-armed the timer; it fired
a few times and stopped. `cond_yield_lp` now clears/re-arms even when not
switching, but empirically the timer still stops after a few fires with a
non-empty queue — so `sleep` and preemptive scheduling of a CPU-bound thread do
not yet work reliably. Root-causing the timer subsystem
(`timers::TimerQueue::process_events` + the ISA `LpTimer` arm/stop path) is
follow-up work.

### B. Thread teardown via `abort` hangs after the fix

With the double-lock fixed, a returning thread reaches `abort_thread` and is
removed, but the subsequent switch does not resume the woken thread (the demo
hangs after "Thread N is aborting execution"). The remove-current-thread +
yield-with-`curr_tid == None` path needs more work (likely the interaction
between dropping the `Thread` while executing on its kernel stack and the
context switch). Until then, demo threads park instead of returning.

## Follow-up

- Root-cause issue A (timer stops) so `sleep`/preemption work; then the worker
  can perform genuinely time-based async work.
- Fix issue B (clean thread exit) so worker completion can fire from the
  thread's exit-observer (`Thread::Drop`), matching the ABI's intended design
  (`scheduler/threads/mod.rs:63-69`), instead of an explicit `complete` call.
</content>
