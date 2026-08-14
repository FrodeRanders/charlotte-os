# Investigation: Live-Upgrade Stall — Interrupt Masking, Timer Queues, and Thread-ID Recycling (AArch64)

> **Historical investigation:** preserved as debugging evidence and design
> rationale. See the [documentation index](../../README.md) for current status.

Status: **FIXED AND COMMITTED** (`83c68b1`). This document records the architectural
insights collected while debugging the intermittent live-upgrade self-test stall
(`--live-upgrade-test --reuse-storage`), the GIC/timer evidence, the fixes, and the
kernel-design lessons they imply. It is intended as a reference for anyone touching
interrupt masking, the scheduler's yield path, timer queues, or thread identity.

---

## 1. Symptom

The live-upgrade self-test intermittently stalled after the object store served ELFs.
The system neither panicked nor failed an assertion: the harness reported
`error: authoritative self-test result was not produced within 40s`. Repeatable
signature in the serial log:

```
[+ 0.550584] [timer] LP0 PPI #20          ← LP0's timer PPI counter FREEZES here
...
[+ 1.287754] [cqwt] tid=8 enqueue timer deadline=443487687 ms=10   ← last CQ watchdog ever
...then NOTHING on LP0 for 40 s, while LP1/2/3 PPIs keep climbing (#101, #201, ...)
[+ 45.x]     [rr] set_next_timer_event lp=0 tid=6 is_idle=false     ← boot continuation spinning
```

The NVMe driver (tid=8, on LP0) had submitted a read; the device never completed it,
**and** the 10 ms CQ-wait watchdog never fired. Every waiter upstream of that read
(store → objstore → client → upgrade) deadlocked.

---

## 2. Root cause: DAIF is per-LP CPU state, not per-thread

This is the central insight of the investigation, and it is easy to get wrong because
the scheduler *looks* like it tracks per-thread interrupt state.

### 2.1 What the code does

- Interrupts are masked/unmasked by writing the CPU's `DAIF` register — one register
  per logical processor, shared by every thread that runs on it.
- `switch_ctx` (AArch64 context switch, `lp/ops.rs`) saves and restores only general
  registers. **It does not save or restore `DAIF`.**
- The only mechanism that restores a thread's interrupt state across a yield is
  `cond_yield_lp`:
  ```
  interrupts_were_enabled = get_int_state();   // captured at entry
  mask_interrupts!();                           // protect scheduler locks
  ... possibly switch_ctx ...
  if interrupts_were_enabled { unmask_interrupts!(); }   // restore at exit
  ```

### 2.2 The failure chain

1. `finish_boot` (the boot continuation, tid=6) executed
   `mask_interrupts!()` and ran the whole deferred self-test suite masked —
   a deliberate "one transaction" design that predates kernel preemption.
2. Because `DAIF` is CPU state, the continuation's *restored* state after any
   `cond_yield_lp` was "masked" (`interrupts_were_enabled == false`), even after it
   resumed from a switch.
3. When the masked continuation switched to another kernel thread on LP0 (e.g. the
   NVMe driver), that thread resumed with the CPU's `DAIF` **still masked**, and its
   own next `cond_yield_lp` captured `interrupts_were_enabled == false` — so it also
   restored masked.
4. **The mask propagated to every kernel thread that ever ran on LP0**, permanently.

### 2.3 Why this wedges the LP

- The quantum timer PPI is a Group-1 interrupt. With `DAIF.I=1` the CPU never takes
  it, even though the GIC has it pending.
- `cond_yield_lp`'s "only runnable thread is current" branch re-arms the quantum timer
  every time — but re-arming is pointless while masked: the PPI can never be
  delivered, so neither a timer wake nor a pending device interrupt can ever make
  another thread runnable. The LP busy-spins masked forever.
- Every timeout enqueued on the wedged LP (see §4) dies with it.

### 2.4 GIC-level evidence (from `dump_cpu_interface_state` at stall time, LP0)

```
cpu-interface: CTLR=0x8c00 AP1R0=0x0 RPR=0xff PMR=0xf8 HPPIR=0x50 IGRPEN1=0x1 DAIF=0x3c0
cntv:          ctl=0x5 cval=... now=...        ← timer ENABLED, compare met (ISTATUS=1)
redistributor: timer PPI 27 en=1 pend=1 act=0   ← PPI PENDING at the GIC, never taken
```

- `HPPIR=0x50` = INTID 80 (NVMe MSI) pending at the CPU interface, never acknowledged.
- `RPR=0xff` (idle), `AP1R0=0`, `IGRPEN1=1`, `PMR=0xf8` (normal 5-bit GIC readback —
  not a bug).
- Conclusion: the interrupt is enabled, pending, and routed — the **CPU** is refusing
  it because `DAIF.I=1`.

### 2.5 Why the failure was a lottery

`spawn_thread` → `pick_lp_for` → least-loaded LP decides where the NVMe driver thread
lands. If it landed on LP1/2/3 the test usually passed; if it landed on LP0 — where
every kernel thread was poisoned by the propagated mask — it stalled. The same binary
flipped between pass and stall run to run.

### 2.6 The fix

Remove the mask from `finish_boot` and run the deferred self-test suite like any
kernel thread (unmasked). The "one transaction" atomicity the mask provided was
already vestigial: the suite's waits are event-driven, its state is continuation
local, and the explicit yields still switch to EL0/services as before.
Verified by the probe: `yield_lp LP0 tid=6 DAIF=0x0` at every entry, LP0 PPI counter
climbing past #400, 60/60 consecutive passes.

**Lesson: if a kernel continuation must be non-preemptible, it must not be
implemented as `mask_interrupts!()` across `yield_lp`. Either block genuinely
(event-driven, see §6) or hold an explicit scheduler-level non-preemption token —
never a CPU interrupt mask, because the mask leaks to every thread on the LP.**

---

## 3. Secondary fix: `force_unmask` in `cond_yield_lp` (with a depth guard)

The root-cause fix (§2.6) removed the *source* of the mask. A defensive fix was also
added to `cond_yield_lp` so a masked thread that finds itself the only runnable thread
does not busy-spin with a re-armed-but-undeliverable quantum:

```
same-thread branch:
  clear_ctx_switch_pending();               // re-arms the quantum timer
  if !interrupts_were_enabled
     && get_interrupt_depth() == 0 {        // pure thread context ONLY
      force_unmask = true;
  }
...
if interrupts_were_enabled || force_unmask { unmask_interrupts!(); }
```

### 3.1 The depth guard matters

An earlier version omitted the `get_interrupt_depth() == 0` condition. That caused a
**new** crash signature:

```
KERNEL DATA/INST ABORT: ESR=96000007 ELR=ffffffff800006cc FAR=ffff810000033000
  current tid=2 sp=... spsr=0x20000005 saved_sp=... stack_buf=0xffff810000023000
```

- `ELR=0x6cc` is the 8th `ldp` of `pop_volatile_regs` in `ivt.asm`'s `irq_common`.
- `spsr=0x20000005` has the I bit clear — **interrupts were enabled inside the IRQ
  handler**, which is only possible if something unmasked them mid-handler.
- `FAR=0x33000` is exactly the top of the interrupted thread's kernel stack, and the
  reconstructed IRQ entry SP was 0x40 bytes *above* it.

Mechanism: `irq_dispatcher` calls `cond_yield_lp` at its tail. Without the depth
guard, the same-thread branch unmasked interrupts **before** `irq_common` finished
`pop_return_state`/`pop_volatile_regs`. A nested IRQ taken in that window pushed its
frame at a SP that the outer pop was about to walk past — the restore read past the
stack top into the guard page.

The tail's `eret` restores the interrupted thread's saved `PSTATE` anyway, so
unmasking in the IRQ tail is never necessary. The depth guard confines
`force_unmask` to genuine thread context.

**Lesson: code that runs between "interrupts were re-enabled" and "eret" must be
nested-IRQ safe. The IRQ tail is such code; only thread context may unmask.**

---

## 4. Insight: timer queues are per-LP; a timeout lives and dies on its LP

- `TIMER_QUEUES` is `PerLp<TimerQueue>` (`timers/mod.rs`).
- `enqueue_event` inserts into **the current LP's** queue.
- Each LP's timer PPI (INTID 27) runs `process_events` on **its own** queue.

Consequences:

1. **Every timeout of a thread is queued on the LP the thread is on at enqueue
   time.** The NVMe driver and the boot continuation were both on LP0, so both their
   timeouts died with LP0's wedged PPI.
2. Old and new echo service tids live on different LPs (`spawn_thread` →
   least-loaded), so their timers are independent — but a *blocking wait* upstream
   that lives on the wedged LP still hangs regardless of which LP the echo runs on.
3. There is no per-thread or global timer queue; an LP whose PPI delivery is wedged
   silently starves every wait whose watchdog is on that LP. This is why
   event-driven waits with their own timers are only as reliable as the LP they run
   on.

**Lesson: a blocked thread's timeout is only as healthy as the LP it happens to be
on. Fixes to wait paths must not assume "the timer works" without verifying the
specific LP's PPI is being delivered.**

---

## 5. Insight: thread IDs are recycled; (asid, tid) is not a stable identity

`IdTable` (`klib/collections/id_table.rs`) reuses freed slots LIFO
(`available_ids.pop()`), incrementing a slot generation. In the live-upgrade test:

- gen-1 echo: tid=10, exits at ~1.70 s.
- gen-2 echo: spawned at ~1.78 s → **recycles tid=10**, exits via `OP_HANDOFF` at
  ~2.48 s.
- gen-3 echo: spawned by the manager *before* it replies to the upgrade request →
  **recycles tid=10 again**.
- The verifier then called `wait_domain_exit(&e2)` → `observe_thread_exit(asid=5,
  tid=10)` → the master table lookup found a *live* thread at tid=10 (gen-3 echo)
  and registered the exit observer on **it** — which never exits → 10 s deadline
  panic (`[supervisor] domain did not exit before deadline (asid=5)`).

The pre-existing "already reaped" fallback (lookup fails → complete immediately)
could not distinguish "the thread is gone" from "the slot now holds a different
thread".

### 5.1 The fix: generation-bound observers

`Thread` carries a monotonic, never-reused `generation`
(`NEXT_THREAD_GENERATION.fetch_add`), and `ServiceDomain` already recorded the
spawn-time generation ("reuse-safe identity"). Added:

- `scheduler::observe_thread_exit_with_generation(tid, expected_generation, obs)`
- `completion::observe_thread_exit_with_generation(asid, tid, Some(gen))`
- `supervisor::wait_domain_exit` and both client-exit waits now pass the domain's
  generation.

If the slot holds a different generation, the registration returns `Err` and the
capability completes immediately — correct, because the observed thread is
guaranteed gone.

**Lesson: any kernel API that takes a `ThreadId` and later resolves it through the
master table is exposed to recycling. Carry a generation from the point the handle
was captured. The `IdTable` slot generation exists for exactly this purpose; the
thread's own `Thread.generation` is the right value to store in long-lived handles.**

---

## 6. Insight: event-driven waiting is the correct shape for kernel waits

The verifier paths originally busy-polled: `loop { check; yield_lp(); }` with a
deadline assert. This is fragile for three reasons:

1. A `yield_lp` poll consumes scheduler capacity and makes no progress guarantee —
   the deadline check only runs when the thread is scheduled.
2. It depends on the timer/device wake path being healthy (the very thing under
   test), and a wedged LP makes the deadline assert unreachable.
3. It is unnecessary: the kernel already has a complete event-driven wait mechanism
   (`block_thread` + observers + timer watchdogs) used by `sleep`, `wait_reply`,
   and `cq_wait`.

### 6.1 The primitives added

- `scheduler::block_until(observable, timeout_ms, condition) -> bool` — parks the
  thread in `Blocked` state with its waker registered on the observable, plus a timer
  watchdog that re-admits the thread on expiry so the caller's condition re-check
  (and deadline assert) always runs. Includes a lost-wake guard that re-admits if the
  condition became true during registration.
- `ipc::wait_reply_timeout(caller, call, timeout_ms)` — `block_until` on the
  pending-call observable.
- `completion::wait_timeout(asid, cap, timeout_ms)` — `block_until` on a completion.
- `results::wait_until_resolved(id, timeout_ms)` + a `RESULTS_OBSERVERS` queue
  notified from `pass`/`fail`; the results coordinator itself parks on the observable
  with a 1 s watchdog instead of spinning.
- `supervisor::wait_domain_exit` — generation-bound exit observer + `wait_timeout`,
  then a short `sleep_millis(10)` settle loop re-checking full address-space drain.

All self-test verifier poll loops (objstore registration, client exit, objdone
completion, service lookups, upgrade replies) were converted to these.

**Lesson: busy-wait loops that check a shared variable should instead block on the
event source that changes that variable. The kernel's observable/observer machinery
already supports it; a watchdog timer on the same observable makes missed
notifications fail loudly instead of hanging.**

---

## 7. Related fixes in the same change

- **RwLock writer preference** (`cpu/multiprocessor/spin/rwlock.rs`) — added
  `writer_waiting` so a queued writer is not starved by a continuous stream of
  readers (the IPC registry probe around a reply).
- **Memory-object lock ordering** (`memory/object.rs`) — `map` no longer holds the
  memory-object registry while taking the address-space table; the registry,
  address-space table, and frame allocator locks form an AB-BC-CA cycle, and the
  registry-across-table acquisition closed it.
- **`start_domain` panic message** now includes the tid and asid for the (still
  pre-existing, rare) spawn-vs-retire race.

---

## 8. Validation

- `scripts/run-aarch64.sh debug --live-upgrade-test --reuse-storage --timeout 40`:
  **60/60 consecutive passes** after the fixes (previous best was ~8/10 with
  frequent stalls/fails).
- Log signature checks: `DAIF=0x0` at every yield entry for the boot continuation;
  LP0 timer PPI counter no longer freezes.
- Builds: `cargo build` (live-upgrade and default features) and `cargo clippy` clean.
- Both `cargo fmt` (workspace) and the excluded crates (`catten-services`,
  `catten-user`) verified formatted.

---

## 9. Key files

- `crates/catten/src/cpu/isa/aarch64/lp/ops.rs` — `cond_yield_lp` force-unmask with
  interrupt-depth guard.
- `crates/catten/src/main.rs` — `finish_boot` runs the deferred self-test suite
  unmasked.
- `crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs` — `irq_dispatcher`
  interrupt-depth tracking (the depth guard's reference).
- `crates/catten/src/cpu/scheduler/mod.rs` — `block_until`,
  `observe_thread_exit_with_generation`.
- `crates/catten/src/cpu/scheduler/threads/mod.rs` — `Thread.generation`,
  `IdTable` recycling context.
- `crates/catten/src/ipc/mod.rs` — `wait_reply_timeout`.
- `crates/catten/src/completion/mod.rs` — `wait_timeout`,
  `observe_thread_exit_with_generation`.
- `crates/catten/src/self_test/results.rs` — `RESULTS_OBSERVERS`,
  `wait_until_resolved`.
- `crates/catten/src/service/supervisor.rs` — `wait_domain_exit` (generation-bound),
  `start_domain` panic message.
