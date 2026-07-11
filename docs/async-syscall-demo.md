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

## Scheduler / timer / lifecycle bugs found and fixed

Building the demo exercised the *block → yield → wake → resume* and the
*thread-exit* paths for the first time and surfaced a series of latent bugs
(none fired before because no kernel thread had ever blocked-and-resumed,
slept, or returned). All are now fixed; the demo runs with real timer-based
`sleep` and clean thread exit, and reports `Ok(42)` / `SUCCESS` with zero
panics on QEMU AArch64.

### Blocked threads lost their execution context

`block_thread` cleared the LP scheduler's `current_handle` for the running
thread, so `cond_yield_lp` saw `curr_tid == None` and `switch_ctx(null, ..)`
**skipped saving the blocking thread's context** — it restarted from its entry
on wake. Fix: the self-blocking running thread stays `current_handle` until the
yield saves its context; `RoundRobin::next` no longer re-queues or re-`Ready`s a
`Blocked` thread.

### Two RwLock "held across a re-locking call" deadlocks

- `sleep()` held a `SYSTEM_SCHEDULER.read()` guard (plus the LP scheduler lock)
  across `SYSTEM_SCHEDULER.write().block_thread(..)` — a read-then-write
  self-deadlock. This was the real cause of the earlier "sleep heisenbug"
  (logging happened to bind the tid first, releasing the guard).
- `abort()` held the LP scheduler lock across `abort_thread`, which re-locks it.
- `abort_thread` itself held `MASTER_THREAD_TABLE.write()` across a second
  `.write()`.

Fix: bind the value out of the scrutinee first so the temporary guards drop
before the re-locking call.

### Quantum timer grew the timer queue without bound

`clear_ctx_switch_pending` enqueued a *new* quantum `TimerEvent` on every call,
but it is called on every `yield_lp` (not just on quantum expiry), so manual
yields accumulated quantum events forever. Fix: an `armed` flag keeps exactly
one quantum event in flight (the quantum's own firing clears it). Also,
`cond_yield_lp` now re-arms even when there is nothing to switch to, and
`TimerQueue::process_events` stops the (level-triggered) timer when the queue
drains.

### A thread could not free its own kernel stack

`ThreadContext::drop` frees the thread's kernel stack, but a returning thread is
still *executing on that stack* when `abort` drops it. Fix: a **deferred
reaper** — `abort` moves the dying thread to `DEAD_THREADS`
(`IdTable::take_element` extracts it without dropping), and `cond_yield_lp`
calls `reap_dead_threads()` *after* switching away, so the stack is freed from a
different thread's context.

### The stack allocator never registered guard pages

`allocate_stack` never populated `KERNEL_GUARD_PAGE_SET`, so `deallocate_stack`
always failed. Fix: register the lower/upper guard pages on allocation and
rewrite `deallocate_stack` to derive the page count from them.

## Known remaining limitations

- **Adjacent-stack guard overlap.** `find_free_region` does not account for the
  (unmapped) guard pages, so back-to-back stack allocations can share a guard
  region; freeing one can then invalidate the other's guard bookkeeping. This
  leaks a stack rather than crashing (`ThreadContext::drop` logs and continues).
  Proper fix: reserve guard pages in the free-region search.
- **x86_64 demo.** On x86_64 the worker's `sleep` and completion both work
  (it reaches "work finished, completing capability"), but the final coordinator
  resume needs the x86_64 `cond_yield_lp` to get the same treatment as AArch64.
  x86_64 self-tests all pass.

## Follow-up

- Reserve guard pages in `find_free_region` so adjacent stacks don't share
  guard regions (removes the last stack leak on thread exit).
- Bring the x86_64 `cond_yield_lp` block/wake path to parity with AArch64 so the
  demo's coordinator resumes there too.
- Fire completions from the worker thread's exit-observer (`Thread::Drop`) now
  that clean thread exit works, matching the ABI's intended design
  (`scheduler/threads/mod.rs:63-69`), instead of an explicit `complete` call.
</content>
