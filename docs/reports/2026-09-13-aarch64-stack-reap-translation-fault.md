# 2026-09-13 - AArch64 exception return on a reclaimed kernel stack

## Summary

A sustained EL0 load test (the CharlotteOS Kafka broker deployed through the
signed `CDEPLOY5` path and exercised from the host by an independent
kafka-python client) triggered a kernel panic after roughly 13 minutes. The
fault is a level-3 translation fault inside the synchronous-exception return
path, which indicates that a kernel stack page was unmapped while an exception
frame still referenced it.

The soak is doing its job: this is a kernel thread-lifecycle race, exposed by
the broker's short-lived per-connection handler threads.

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
`bl sync_dispatcher` returns. `FAR` is page-aligned and `SP` at panic time was
`0xffff8100000cb9e0`, so the return path read a page immediately above the
live stack that had already been unmapped.

## Why this looks like deferred reaping

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
abort/re-admission race. The new failure is consistent with a remaining hole in
that area:

- the guard proves only that the reaping LP is not currently executing on the
  stack being freed; it does not prove that no saved context, timer event, or
  re-admission path can still resume on it;
- `abort_thread_generation` now avoids freeing a remote thread's stack until
  its owner LP retires it, but a blocked thread that was already re-admitted
  when the abort arrives, or a self-exit staged on an LP that then reaps while
  an exception frame is being restored, are not distinguishable from the
  current check alone.

The broker is a good trigger because every accepted TCP connection spawns a
kernel thread that exits when the connection closes; kafka-python reconnects
for metadata refreshes, producing continuous start/exit churn on one LP.

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

## Second reproduction (2026-09-13, aarch64)

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

This refines the earlier hypothesis. The stack page is not merely reaped by
the deferred thread list; it is unmapped inside a synchronous syscall that is
still executing on it. The likely fault domain is the interaction between
per-address-space frame accounting/direct-map unmapping and live kernel
stacks: if a frame still backing a running kernel stack is released and the
allocator unmaps it, the next stack access faults exactly this way. EL0 stack
growth and the broker's thread churn are the stress that reaches it.

The next investigation should therefore:

1. instrument frame allocation/release with the owning ASID, frame, and
   whether the frame is currently a kernel stack, and reproduce;
2. audit the page-mapping and stack-growth paths (`memory/mod.rs` around
   `map_page`/`note_owned_frame`, `grow_current_user_stack`) for releasing or
   remapping a frame that still backs the running kernel stack;
3. audit address-space teardown for the same hazard independent of the
   deferred thread list.

## Suggested investigation

1. Instrument staging and reaping with `(tid, generation, stack range,
   current_sp, abort_requested, is_on_cpu)` and reproduce to identify the
   thread whose stack was freed.
2. Audit the `switch_ctx`/`cond_yield_lp` coroutine interaction against remote
   abort and wake re-admission, including the case where a waker fires between
   abort marking and retirement.
3. Consider a stronger deferral rule (for example, never reclaim a staged
   context that believes it is on CPU, or that still has a pending waker
   admission) before removing the current `SP`-based guard.
4. Add a scheduler self-test that churns short-lived kernel threads on one LP
   while taking synchronous syscalls, so a regression fails a boot self-test
   instead of an overnight soak.
