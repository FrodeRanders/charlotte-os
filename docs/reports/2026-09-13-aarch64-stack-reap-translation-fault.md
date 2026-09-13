# 2026-09-13 - AArch64 exception-return stack-top translation fault

## Summary

A sustained EL0 load test (the CharlotteOS Kafka broker deployed through the
signed `CDEPLOY5` path and exercised from the host by an independent
kafka-python client) repeatedly triggered a level-3 translation fault while
returning from a synchronous exception. Initial captures suggested that the
active kernel stack had been reclaimed. A later lifecycle-instrumented capture
disproved that hypothesis: the faulting context was the live VirtIO network
driver, its stack had never entered retirement, and exception-entry SP had
advanced exactly to the stack's unmapped upper guard page.

The root cause was AArch64's same-thread `cond_yield_lp` path enabling IRQs
even when its caller entered with IRQs masked. Synchronous syscalls can yield
with their vector frame still live; allowing a nested IRQ in that state can
unbalance the outer exception return. The fix preserves the caller's interrupt
state, matching x86-64. A five-minute broker soak completed with 2,747 produced,
2,746 consumed, no gaps, no client errors, and no kernel/watchdog fault.

## Evidence

Panic from the aarch64 guest (`--deployment-ingress-test`, broker generation 7):

```text
KERNEL DATA/INST ABORT: ESR=96000007 ELR=ffffffff800006d8 FAR=ffffffff8100000cc000
Kernel panic:
panicked at crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs:327:5:
Unhandled synchronous exception: EC=0x25, ESR_EL1=0x96000007,
ELR_EL1=0xffffffff800006d8, FAR_EL1=0xffffffff8100000cc000
```

ESR `0x96000007`: EC `0x25` (data abort taken without an exception-level
change), ISS `0x07` = translation fault, level 3.

Symbolized with the matching aarch64 kernel symbols:

```text
#00 0xffffffff80100b8c  __rustc::rust_begin_unwind+0x74
#01 0xffffffff800faf2c  core::panicking::assert_failed::<usize, usize>
#02 0xffffffff8002f0ac  sync_dispatcher+0x1380
#03 0xffffffff800006d8  sync_common+0x54
```

`sync_common+0x54` is `ldp q0, q1, [sp], #0x20`, the first SIMD reload after
`bl sync_dispatcher` returns. `FAR` is page-aligned. At this point the initial
capture could not distinguish an improperly reclaimed page from SP reaching a
valid stack's guard page; the later lifecycle capture makes that distinction.

## Initial, superseded deferred-reaping hypothesis

`crates/catten/src/cpu/scheduler/threads/mod.rs` stages exited threads per LP
in `DEAD_THREADS` and frees their stacks from `reap_dead_threads`, which runs
from `cond_yield_lp` after a context switch. The existing guard defers any
staged thread whose stack contains the reaping LP's *current* `SP`:

```rust
let current_sp = current_stack_pointer();
let (deferred, reclaimable) =
    dead.into_iter().partition(|thread| thread.context.kernel_stack_contains(current_sp));
```

The comment already records a previous use-after-free that "manifests as a
translation fault on the next timer-IRQ return" and notes a remote
abort/re-admission race. Before stack identities were captured, the failure
appeared consistent with a remaining hole in that area:

- the guard proves only that the reaping LP is not currently executing on the
  stack being freed; it does not prove that no saved context, timer event, or
  re-admission path can still resume on it;
- `abort_thread_generation` now avoids freeing a remote thread's stack until
  its owner LP retires it, but a blocked thread that was already re-admitted
  when the abort arrives, or a self-exit staged on an LP that then reaps while
  an exception frame is being restored, are not distinguishable from the
  current check alone.

The broker traffic correlated with the failure, but this capture did not prove
that kernel-thread churn is the trigger. The last `[thread] abort` record is at
7.788 seconds, before the broker is launched as ASID 45 at 20.756 seconds; no
further thread-abort record appears before the panic at 338.560 seconds. The
connection handlers may therefore be application-level tasks within the broker
domain, or their lifecycle may simply not be visible through the current
logging. The decisive capture below subsequently rejected this hypothesis for
the reproduced fault.

## Reproduction

On any host with Docker and QEMU:

```sh
# in the broker repository, with CHARLOTTE_OS_DIR pointing at a full checkout
CHARLOTTE_OS_DIR=../charlotte-os tools/soak/run_soak.sh --duration 1200 --rate 20
```

The guest serial log and kernel symbols needed for offline symbolization are:

```sh
scripts/symbolize-kernel-panic.py \
  --symbols target/aarch64-unknown-none-catten/debug/catten.symbols \
  /tmp/charlotte-serial.log
```

The failure is intermittent; the first observed run lasted 781 s of client
traffic (about 6,800 records) before the panic.

## Second reproduction and misleading proximity (2026-09-13, aarch64)

A later run reached 2,740 records (about 300 s) before the same fault class,
now with the faulting instruction inside the dispatcher itself:

```text
KERNEL DATA/INST ABORT: ESR=96000007 ELR=ffffffff800a289c FAR=ffffffff8100000cc1a0
Kernel backtrace: sp=0xffff8100000cb9e0 fp=0xffff8100000cba60
  #00 0xffffffff800cb080  __rustc::rust_begin_unwind+0x74
  #01 0xffffffff800f4894  core::panicking::assert_failed::<usize, usize>
  #02 0xffffffff800a3154  sync_dispatcher+0x1380
  #03 0xffffffff800006d8  sync_common+0x54
```

Disassembly of `ELR = sync_dispatcher+0xac8`:

```text
...  bl  note_owned_frame
...  bl  inval_range_user
...  ldp x20, x19, [sp, #0x1a0]      <- faulting instruction
```

`FAR = 0xffff8100000cc1a0` is exactly `SP + 0x1a0` with `SP` page-aligned at
`0xffff8100000cc000`, so the dispatcher's own stack frame was reloaded from a
page that had just become unmapped. The instructions immediately before the
epilogue are the inlined tail of the page-mapping/accounting path in
`crates/catten/src/memory/mod.rs:301-302` (`note_owned_frame` followed by
`inval_range_user`), which runs when the kernel maps a frame into a user
address space, for example while growing an EL0 stack.

The proximity to `note_owned_frame` and `inval_range_user` originally suggested
that page mapping had unmapped the active kernel stack. The lifecycle capture
below shows that this was misleading instruction proximity: the live stack was
not retired, while SP itself reached its guard boundary.

## Diagnostic instrumentation

1. Reproduce with `--scheduler-trace`. The dedicated lifecycle flight recorder
   captures every stage, reap decision, and impending kernel-stack
   deallocation as one atomic record containing `(LP, reap LP, tid, generation,
   ASID, stack range, current SP, on_cpu, abort owner)`. At timeout,
   `scripts/run-aarch64.sh` extracts and decodes it to
   `/tmp/charlotte-thread-lifecycle-trace.log`. Find the record whose stack
   starts at `0xffff8100000cc000`; its preceding stage and reap records identify
   the thread and the lifecycle decision that released the live mapping.
2. Kernel same-EL abort diagnostics record reconstructed exception-entry SP and
   the lock-free current LP/TID/generation/ASID snapshot.
3. The watchdog prints the sparse lifecycle trace before the ordinary scheduler
   trace, so a host client that terminates QEMU cannot discard the evidence.

## Third reproduction and capture lesson (2026-09-13, aarch64)

A scheduler-traced broker soak reproduced the exception-return fault after
about 29 seconds of traffic. The host client reported a 30-second produce
timeout and then `NoBrokersAvailable`; the serial log shows that these were
consequences of an earlier kernel failure, not Kafka protocol errors:

```text
[+    53.688928] KERNEL DATA/INST ABORT: ESR=96000007
                  ELR=ffffffff80000724 FAR=ffff8100000dd000
Kernel backtrace: sp=0xffff8100000dc9e0 fp=0xffff8100000dca60
  #00 0xffffffff800f4cd0  __rustc::rust_begin_unwind+0x74
  #01 0xffffffff800a8844  core::panicking::assert_failed::<usize, usize>
  #02 0xffffffff80022f34  sync_dispatcher+0x1380
  #03 0xffffffff800006d8  sync_common+0x54
```

For kernel SHA-256
`07457732ff0b4563354543ad0adb7f4a20eb6480e137b23150889182deaed471`,
`ELR = sync_common+0xa0` is the first `pop_volatile_regs` instruction:

```text
ffffffff80000718  ldp x9, x10, [sp], #0x10
ffffffff8000071c  msr FPCR, x9
ffffffff80000720  msr FPSR, x10
ffffffff80000724  ldp x0, x1, [sp], #0x10   <- fault
```

This is the same exception-return failure class as the first capture. LP 0's
last retained scheduler transition dispatched TID 12; it then stopped while
LPs 1--3 continued. No `STACK_ARENA_*` operation appears in the retained
ordinary trace immediately before the fault. That absence weakens the narrow
hypothesis that a stack deallocation happened immediately before this
particular exception return, but it cannot exclude an older deallocation or a
stale/restored stack pointer. The lifecycle records are needed to distinguish
those cases.

Although this run enabled `--scheduler-trace`, its lifecycle memory image was
not recovered. The soak client exited on its Kafka timeout, and the soak
wrapper killed QEMU before `run-aarch64.sh` reached its timeout-time LLDB
extraction. The watchdog now emits the sparse lifecycle recorder to serial
*before* the much larger ordinary scheduler trace. Kernel-abort diagnostics
also report the exception-entry SP and the lock-free current
LP/TID/generation/ASID snapshot. A subsequent client-triggered early teardown
will therefore retain the decisive evidence in `charlotte-serial.log` without
depending on a debugger attachment.

## Decisive lifecycle capture and corrected diagnosis

A subsequent instrumented soak failed at 161.108 seconds with the additional
fault context requested above:

```text
KERNEL DATA/INST ABORT: ESR=96000007 ELR=ffffffff80000724
FAR=ffff8100000dd000 exception_sp=0xffff8100000dd000
lp=0 tid=12 generation=14 asid=10
```

This changes the diagnosis. ASID 10 is the long-lived VirtIO network driver,
not a short-lived broker handler. Its TID 12, generation 14 context never
appears among all 246 captured lifecycle events: it was not staged, reaped, or
deallocated. Its 16-page kernel stack occupies the arena range ending at
`0xffff8100000dd000`; both the reconstructed exception-entry SP and FAR equal
that upper guard-page address. This is therefore stack-pointer over-advance in
the exception return path, not a live stack being reclaimed.

The retained scheduler trace also contains repeated same-thread dispatches for
TID 12 (`SCHED_DISPATCH a=0xc b=0xc`). AArch64's `cond_yield_lp` had a
same-thread `force_unmask` path: if a yield began with IRQs masked, it enabled
them before returning. The code already excluded the IRQ-tail entry because a
nested IRQ below a live vector frame had previously produced exactly this
signature, but synchronous syscalls still used the force-unmasking entry. The
network driver's frequent CQ and device syscalls made it an effective trigger.

The fix removes forced unmasking. `cond_yield_lp` now restores IRQs only when
they were enabled by its caller, matching the x86-64 implementation. Fresh
thread trampolines and the idle loop already enable interrupts explicitly, and
the boot continuation that originally motivated forced unmasking has since
been corrected to enter the scheduler with interrupts enabled. Thus no
scheduler-progress path requires violating a masked caller's exception
context.

## Validation after the fix

A fresh broker build and signed deployment completed a 300-second soak at a
requested rate of 20 messages/s using kernel SHA-256
`0baee06e23512f5d6198eefe6753d0e6c854efd8e2bd594d940dce470c209094`:

```text
soak stop produced=2747 consumed=2746 gaps=0 errors=0 elapsed=300s
```

The effective acknowledged rate was 9.1 messages/s. Consumer lag stayed at one
or two records throughout. The guest serial log remained active beyond 324
seconds, all 20 boot self-tests passed, and neither a kernel panic nor a
watchdog stall was reported. This exceeds the 29-second and approximately
135-second client-load failure windows from the two immediately preceding
reproductions. It is strong targeted evidence for the interrupt-state fix,
though the intended overnight soak remains the final endurance validation.
