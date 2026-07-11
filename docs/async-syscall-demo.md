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

*(Both prior limitations are now fixed — see below.)*

### Fixed: adjacent-stack guard overlap

`find_free_region` does not account for the (unmapped) guard pages, so
back-to-back stack allocations shared a guard region (one stack's upper guard =
the next stack's lower guard); freeing the first removed the shared guard and
broke the second's bookkeeping, leaking its stack. Fixed by **reference-counting
guard pages** (`KERNEL_GUARD_PAGES: BTreeMap<VAddr, usize>`): a shared guard now
survives until both stacks are freed. All demo stacks are now freed cleanly.

### Fixed: x86_64 demo parity

Two x86_64-specific fixes bring it to parity with AArch64:
- `cond_yield_lp` now re-arms the quantum timer even when there is nothing to
  switch to (as on AArch64).
- `kernel_thread_trampoline` now **calls `abort`** when a thread's entry point
  returns, instead of halting the CPU forever (`hlt` loop). Previously a
  returning kernel thread froze the LP.

The demo now runs to completion on **both** AArch64 and x86_64: worker sleeps →
completes → coordinator resumes with `Ok(42)` → `SUCCESS`, and both threads exit
cleanly and are reaped, with zero panics and no leaked stacks.

## Follow-up

- Fire completions from the worker thread's exit-observer (`Thread::Drop`) now
  that clean thread exit works, matching the ABI's intended design
  (`scheduler/threads/mod.rs:63-69`), instead of an explicit `complete` call.
</content>
