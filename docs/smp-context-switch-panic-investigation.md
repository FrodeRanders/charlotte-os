# Investigation: intermittent multi‑LP data‑abort panic (AArch64)

Status: **UNRESOLVED**. Several real concurrency hazards were fixed along the way,
but the specific panic still reproduces (~40% of boots) on `-smp 2` and `-smp 4`.
This document records the symptom, the evidence, every hypothesis tested (and how),
the code changes made, the central contradiction that is still unexplained, and a
concrete plan to finish the job.

---

## 1. Symptom

On QEMU AArch64 with more than one logical processor (LP), boot intermittently ends in:

```
DATA ABORT: ESR=96000007 ELR=ffffffff800002d0 FAR=ffff810000033000
panicked at crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs:137:5:
Unhandled synchronous exception: EC=0x25, ESR_EL1=0x96000007, ELR_EL1=0xffffffff800002d0, FAR_EL1=0xffff810000033000
```

- `-smp 1`: **never** reproduces. The async‑syscall demo completes (`[async] SUCCESS`).
- `-smp 2` / `-smp 4`: reproduces intermittently, always with the **same** `ELR` and `FAR`.

The panic originates in `sync_dispatcher` (`crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs`),
in the data‑abort branch that treats any non‑SVC synchronous exception as fatal.

---

## 2. How to reproduce / tools used

Environment: macOS (Apple Silicon), `qemu-system-aarch64`, `mtools`, Rust nightly with
the custom target `target_specs/aarch64-unknown-none-catten.json`.

Build (headless, no C `flanterm` dependency so it links cleanly):

```
cargo build --package catten \
  --target target_specs/aarch64-unknown-none-catten.json \
  --no-default-features --features acpi
```

Boot scripts (in `scripts/`) — **note: these `mcopy` the freshly built kernel into the
`.img`; a raw `qemu` invocation against an old image will silently run a stale kernel
(see §8)**:

- `scripts/boot-smp1.sh` — 1 LP, 60 s, serial to `/tmp/charlotte-serial.log`.
- `scripts/boot-smp2.sh` — 2 LPs, 90 s, serial to `/tmp/charlotte-smp2.log`.
- `scripts/boot-aarch64.sh` — 4 LPs, serial to stdout.
- `scripts/run-aarch64.sh [--gdb]` — builds an ESP image and boots; `--gdb` adds `-s -S`.

Fast manual loop actually used for statistics (rebuild the image first!):

```
K=target/aarch64-unknown-none-catten/debug/catten
IMG=./os-images/charlotte-aarch64-fresh.img
dd if=/dev/zero of="$IMG" bs=1m count=64 status=none
mformat -i "$IMG" -F -v CATOS ::
mmd -i "$IMG" ::/EFI ::/EFI/BOOT
mcopy -i "$IMG" ./limine-binary/BOOTAA64.EFI ::/EFI/BOOT/BOOTAA64.EFI
mcopy -i "$IMG" "$K" ::/catten
mcopy -i "$IMG" ./limine.conf ::/limine.conf

qemu-system-aarch64 -M virt,gic-version=3 -cpu cortex-a710 -smp 4 -m 512M \
  -bios /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
  -drive if=none,file="$IMG",format=raw,id=drive0 -device virtio-blk-device,drive=drive0 \
  -serial stdio -display none -no-reboot
```

Timing notes that cost real debugging time:
- Boot to the "crash window" takes ~15–25 s under (MT)TCG and is **variable**; short kill
  windows (≤ ~20 s) frequently stop *before* the demo/abort phase and produce false "ok".
- `-serial file:` is **buffered**; live‑polling the file for markers does not work. Use
  `-serial stdio` redirected to a file for unbuffered output.
- `-accel tcg,thread=single` is much slower (all vCPUs on one host thread) and could not
  reach the crash window inside a reasonable timeout — the single‑vs‑multi‑threaded TCG
  classification test was **inconclusive** for this reason.

Disassembly (to identify the faulting instruction):

```
SYSROOT=$(rustc --print sysroot); HOST=$(rustc -vV | awk '/^host:/{print $2}')
"$SYSROOT/lib/rustlib/$HOST/bin/llvm-objdump" -d \
  --start-address=0xffffffff80000280 --stop-address=0xffffffff80000320 \
  target/aarch64-unknown-none-catten/debug/catten
```

---

## 3. Decoding the crash

### 3.1 ESR / fault class
`ESR_EL1 = 0x96000007`:
- `EC = ESR>>26 = 0x25` → **Data Abort taken without a change in Exception level** (from EL1).
- `DFSC = ESR & 0x3f = 0x07` → **translation fault, level 3** (the page is not mapped).

`SPSR` captured during a temporary instrumentation run was `0x20000005` → `M[3:0]=0b0101`
= **EL1h** (the interrupted context used `SP_EL1`), confirming a kernel‑mode fault.

### 3.2 The faulting instruction (`ELR=0xffffffff800002d0`)
Disassembly of the interrupt vector table (built from
`crates/catten/src/cpu/isa/aarch64/interrupts/ivt.asm`) shows `0x...2d0` is the **last pop**
of `pop_volatile_regs` in the **IRQ, current‑EL, `SP_ELx`** vector (vector offset `0x280`):

```
0xffffffff800002a8: bl irq_dispatcher
0xffffffff800002ac: ldr  x18, [sp], #0x8      ; pop_volatile_regs begins
...
0xffffffff800002d0: ldp  x0,  x1,  [sp], #0x10 ; <-- faults here (last pop)
0xffffffff800002d4: eret
```

So the fault happens on **interrupt return**, restoring `x0/x1` just before `eret`.

### 3.3 What the SP must have been
`ldp x0,x1,[sp],#0x10` reads `[sp]` and `[sp+8]`; `sp` is 16‑byte aligned so both lie in the
same page. `FAR = sp = 0x033000`. `x0/x1` were the *first* pair pushed by
`push_volatile_regs` (highest address = `entry_sp - 0x10`), so at this pop
`sp = entry_sp - 0x10`. Therefore:

```
entry_sp - 0x10 = 0x0000_0000_0003_3000  →  entry_sp = 0xffff_8100_0003_3010
```

`entry_sp` is the SP when the timer IRQ was taken.

---

## 4. Memory map: which stack is `0xffff810000033000`?

`crates/catten/src/memory/linear/address_map.rs` (48‑bit VA map) puts the **KernelStackArena**
at base `0xffff810000000000`. So the fault is inside a kernel thread stack region.

Kernel stacks are allocated by `crates/catten/src/memory/allocators/stack_allocator.rs`
(`allocate_stack`, 16 usable pages = 64 KiB, one guard page below and one above; adjacent
stacks **share** a guard page). Boot logs (`Mapping a thread stack at VAddr(...)`) show the
deterministic layout:

| region base | usable range            | who (tid)                     | top / upper guard |
|-------------|-------------------------|-------------------------------|-------------------|
| 0x001000    | 0x001000–0x010fff       | user thread's user stack (0)  | 0x011000          |
| 0x012000    | 0x012000–0x021fff       | user thread's kernel stack (0)| 0x022000          |
| **0x023000**| **0x023000–0x032fff**   | **`probe_device_topology` (1)** | **0x033000**    |
| 0x034000    | 0x034000–0x043fff       | async coordinator (2)         | 0x044000          |
| 0x045000    | 0x045000–0x054fff       | cross_lp_demo (3)             | 0x055000          |
| 0x056000    | 0x056000–0x065fff       | async worker (4)              | 0x066000          |

`0xffff810000033000` is the **guard page shared between stack 3's top (`probe`) and stack 4's
bottom (coordinator)**. `entry_sp = 0x033010` is `probe`'s stack top (`0x033000`) **+ 0x10**.

`probe_device_topology` (`crates/catten/src/main.rs`) is the long‑lived "initial thread":

```rust
pub extern "C" fn probe_device_topology() {
    logln!(...); let device_topology = &*DEVICE_TOPOLOGY; logln!(...);
    loop { yield_lp(); }          // never blocks, never exits
}
```

Because it spins in `yield_lp()`, `probe` is the **most frequent context‑switcher** and is the
consistent victim.

---

## 5. Evidence timeline

- `-smp 1`: async round‑trip completes, `[async] SUCCESS`, no panic. (Re‑verified with the
  final kernel via `scripts/boot-smp1.sh`.)
- `-smp 2`: async demo succeeds, then several threads abort/exit, then the panic appears
  right around the **first timer IRQs** and thread teardown:

  ```
  Thread 2 is aborting execution.
  [STKDBG] LP1 freeing stack base=0x034000 lower_guard=0x033000 upper_guard=0x044000 n_pages=16
  Thread 4 is aborting execution.
  [STKDBG] LP0 freeing stack base=0x056000 ...
  DATA ABORT ... FAR=0xffff810000033000
  ```

  Two LPs are tearing stacks down concurrently just before the fault, and the freed stack
  (`0x034000`) has its **lower guard == the fault address `0x033000`**.
- Importantly, in one 2‑LP trace the cross‑LP `try_send` to LP1 was **skipped** (the
  `cross_lp_demo` thread happened to run on LP1, so the `my_lp == 0` send never executed) and
  it **still panicked** → the panic is **not** caused by the demo's IPI/cross‑LP send.

---

## 6. The central contradiction (still unexplained)

The fault is on the interrupt‑return **pop** at `sp = 0x033000`. For the matching
interrupt‑*entry* **push** (`stp x0,x1,[sp,#-16]!` at vector `0x280`) to have succeeded, the
CPU must have written `[entry_sp-0x10] = [0x033000]` **without faulting** — i.e. `0x033000`
was **mapped at push time** but **unmapped by pop time**.

Between an IRQ's push and its pop, the only thing that runs is `irq_dispatcher → cond_yield_lp
→ switch_ctx` (switch away and back). So *something unmapped `0x033000` during that window.*

But `0x033000` is a **guard page** and, because `probe` (stack `0x023000–0x032fff`) lives
forever, `find_free_region` can never hand `0x033000` out as a usable page (its neighbour
`0x032000` stays mapped, so a new stack can't start there). And `unmap_page`
(`crates/catten/src/cpu/isa/aarch64/memory/paging/walker.rs`) **does** issue a broadcast
`tlbi vaae1is` (`crates/catten/src/cpu/isa/aarch64/memory/tlb.rs`), and `probe`'s live pages
keep the shared L3 table (which covers `0x000000–0x1fffff` of the arena, i.e. all of stacks
1–? ) alive, so teardown of stack 4 should not free stack 3's pages.

**Conclusion:** either the mental model of the stack/guard layout under churn is wrong, or the
fault is not what a naïve reading of `ELR`/`FAR` implies (e.g. a lower‑level SP/exception
detail, or `saved_sp` corruption that shifts the whole IRQ‑return unwind by exactly `0x10`).
This contradiction is the crux and is **not yet resolved**.

Corollary observed empirically: a temporary check that validated each thread's `saved_sp`
against its own kernel‑stack bounds **immediately before restore** in `cond_yield_lp`
(`[SPDBG]`) **never fired**, even on crashing boots. So whatever is wrong keeps `saved_sp`
*in bounds* — a subtle ~`0x10` drift, not a wild pointer.

---

## 7. Hypotheses tested

| # | Hypothesis | How tested | Result |
|---|-----------|-----------|--------|
| 1 | Cross‑LP **reap use‑after‑free**: `abort_thread` publishes the still‑running thread to a global `DEAD_THREADS`; another LP's `reap_dead_threads` frees its stack before it leaves it. | Made `DEAD_THREADS` per‑LP (see §9). Re‑ran smp2/4. | Real bug, **fixed**, but panic **persists**. |
| 2 | **`find_free_region` TOCTOU / concurrent teardown** corrupts arena page tables. | Added `STACK_ARENA_LOCK` + IRQ masking around alloc/free (§9). | Real hazard, **hardened**, panic **persists**. |
| 3 | **Dangling `saved_sp` pointer**: `cond_yield_lp` captures `&raw mut thread.context.saved_sp` under lock then uses it lock‑free in `switch_ctx`; a concurrent `spawn` growing `MASTER_THREAD_TABLE`'s `Vec` moves every `Thread`. | Boxed the context (`Box<ThreadContext>`) so it is address‑stable (§9). | Plausible, **fixed**, panic **persists**. |
| 4 | **Wake‑before‑save**: a thread is re‑admitted on another LP before the LP that ran it finished saving it (missing acquire/release on the `saved_sp` handoff). | Implemented an `on_cpu` acquire/release handshake in `switch_ctx` (§9). | Correct fix for a real race, panic **persists** (so not the trigger, at least not alone). |
| 5 | **Double‑dispatch**: the same tid is run on two LPs. | Added a global per‑LP "running tid" table + a check in `RoundRobin::next` that logs `[SCHEDDBG] DOUBLE-RUN` / "ALREADY Running". | Detector **never fired**. (Note: heavy per‑dispatch logging perturbs timing and hides the race — keep detectors log‑silent except on the anomaly.) |
| 6 | **Corrupt `saved_sp` at restore**. | `[SPDBG]` bounds check in `cond_yield_lp`. | **Never fired** (see §6). |
| 7 | Memory‑ordering race (needs barriers) vs. logical race. | Attempt `-accel tcg,thread=single`. | **Inconclusive** — too slow to reach the crash window in budget. |

All of #1–#4 remain in the tree as correct hardening (see §9 and §10).

---

## 8. Testing pitfall that wasted time (read this first next session)

`scripts/boot-*.sh` rebuild the FAT image and `mcopy` the freshly built kernel into it. My
fast iteration loop instead ran `qemu` directly against an existing `.img`, which does **not**
pick up a rebuilt kernel. As a result, several intermediate runs silently executed a **stale
kernel** (proved by a leftover `irq_sp~…` debug print and a panic at `mod.rs:148` from
instrumentation that had already been reverted in source). Conclusions about hypotheses #3 and
#4 were initially drawn against a stale image and had to be redone.

**Always rebuild the image (`mcopy` the fresh kernel) before every test run**, or just use the
`boot-*.sh` scripts.

---

## 9. Changes made (kept in the tree)

All changes compile cleanly for `aarch64-unknown-none-catten` and do **not** regress `-smp 1`
(which still prints `[async] SUCCESS`). None is verified to fix the reported panic.

1. **Per‑LP dead‑thread reaping** — closes a cross‑LP stack UAF.
   - `crates/catten/src/cpu/scheduler/threads/mod.rs`: `DEAD_THREADS` changed from
     `RwLock<Vec<Thread>>` to `RwLock<BTreeMap<LpId, Vec<Thread>>>`; added
     `stage_dead_thread(lp, thread)`; `reap_dead_threads()` now only drains the **current
     LP's** list (so a thread is only ever freed by the LP it died on, after that LP has
     context‑switched off it in `cond_yield_lp`).
   - `crates/catten/src/cpu/scheduler/system_scheduler/mod.rs`: `abort_thread` now calls
     `stage_dead_thread(stage_lp, thread)` instead of pushing to the global list.

2. **Serialized stack arena** — closes the `find_free_region` TOCTOU and concurrent teardown.
   - `crates/catten/src/memory/allocators/stack_allocator.rs`: added `STACK_ARENA_LOCK`
     (`spin::Mutex<()>`) and `with_arena()` which masks interrupts, takes the lock, runs the
     op, releases the lock, then restores interrupts (lock released **before** unmasking to
     avoid a reap‑during‑alloc self‑deadlock). `allocate_stack`/`deallocate_stack` are thin
     wrappers over `*_locked` bodies run under `with_arena`.

3. **Address‑stable thread context** — removes the dangling `saved_sp` hazard.
   - `crates/catten/src/cpu/scheduler/threads/mod.rs`: `Thread.context` is now
     `Box<ThreadContext>`, so `MASTER_THREAD_TABLE`'s `Vec<Option<Thread>>` reallocating on
     spawn cannot move a context whose `saved_sp`/`on_cpu` are being used lock‑free by
     `switch_ctx`.

4. **`on_cpu` acquire/release handshake** — closes wake‑before‑save + adds the missing barrier.
   - `crates/catten/src/cpu/isa/aarch64/lp/thread_context/mod.rs`: added `pub on_cpu: u8` to
     `ThreadContext` (initialised `0`).
   - `crates/catten/src/cpu/isa/aarch64/lp/ops.rs`: `switch_ctx` now takes
     `(curr_sp_ptr, next_sp_ptr, curr_on_cpu, next_on_cpu)`. After saving the outgoing thread
     it `stlrb wzr,[curr_on_cpu]` (release‑publish "saved"); before restoring the incoming
     thread it acquire‑spins `ldarb … cbnz` until `*next_on_cpu == 0`, then claims it
     (`stlrb #1`). `cond_yield_lp` captures and passes the `on_cpu` pointers.

Unrelated, from the earlier part of this session (also in the diff): `crates/catten/src/demo.rs`
was refactored (rename `xlp_receiver_on_lp1 → xlp_receiver_on_lp`, move the self‑post `try_recv`
into the receiver, keep the LP0→LP1 send but demo self‑post only).

### Reverted debug instrumentation (not in the tree)
`[STKDBG]` (stack_allocator), `[SCHEDDBG]`/double‑run table (round_robin), `[SPDBG]` +
`saved_sp_oob()` (thread_context/ops), and the `irq_sp~…`/`spsr` print in the data‑abort
handler were all removed. Confirm with:
`rg -n "SPDBG|STKDBG|SCHEDDBG|irq_sp|saved_sp_oob" crates/catten/src` → no matches.

---

## 10. If we want a minimal tree instead

- Definitely keep #1 (per‑LP reap) and #3 (boxed context) — small, isolated, clearly correct.
- #2 (arena lock + IRQ masking) and #4 (`switch_ctx` asm handshake) are correct but larger /
  asm‑level; they can be reverted if a smaller surface is preferred while the real bug is
  hunted. `git diff` lists exactly the six changed files.

---

## 11. Remaining suspects (ranked)

1. **Shared‑guard / `find_free_region` layout under churn.** The contradiction in §6 most
   likely means a stack sometimes ends up with `0x033000` *usable* (mapped), gets an IRQ
   frame on it, and is then torn down. Re‑examine `allocate_stack`/`deallocate_stack`'s
   `KERNEL_GUARD_PAGES` reference‑counting and the "next guard after lower_guard" computation
   in `deallocate_stack` for an off‑by‑one that could unmap a page an adjacent live stack (or
   an in‑flight IRQ frame) is using. Watch the case where `unmap_page` frees an intermediate
   L2/L1/L0 table (`walker.rs`) — it covers a 2 MiB / 1 GiB range; confirm `is_table_unused`
   can never free a table that still maps a live neighbour.
2. **`push_volatile_regs` pushes 19 registers = `0x98` bytes**, leaving `SP` 8‑byte (not
   16‑byte) aligned across `bl irq_dispatcher` and the whole nested call chain
   (`ivt.asm`). It's symmetric with `pop_volatile_regs`, but it is an AAPCS/`SCTLR_EL1.SA`
   violation and a latent source of subtle misbehaviour worth eliminating (pad to `0xA0`).
3. **The lock‑free `switch_ctx` design** in `cond_yield_lp` (raw `saved_sp` pointers used with
   all scheduler locks dropped). The `on_cpu` handshake covers the save/restore ordering, but
   any *other* lock‑free access to a `Thread` between capture and use is still suspect.

---

## 12. Recommended next step: capture ground truth with a debugger

Static reasoning has repeatedly failed here; get the actual machine state at the fault.

1. Boot paused with the gdb stub (image must contain the fresh kernel):
   `scripts/run-aarch64.sh debug --gdb` (adds `-s -S`), or add `-s -S` to the manual `qemu`
   line in §2.
2. Connect a debugger that understands bare‑metal AArch64 ELF (lldb ships on macOS):
   ```
   lldb target/aarch64-unknown-none-catten/debug/catten
   (lldb) gdb-remote localhost:1234
   (lldb) breakpoint set --name sync_dispatcher
   (lldb) continue                # re-run a few times; the bug is ~40%
   ```
3. When it stops in the data‑abort branch, capture:
   - `SP_EL1`, all `x0–x30`, `ELR_EL1`, `SPSR_EL1`, `FAR_EL1`, `ESR_EL1`, `TPIDR_EL1` (LP id),
     `TTBR0_EL1`.
   - Walk the interrupted stack: is `SP` really `0x033010`? Whose stack bounds contain it?
   - Read the scheduler state: `MASTER_THREAD_TABLE` and each `RoundRobin.current_handle` to
     learn the running tid per LP at the moment of the fault.
   - Dump the page‑table entry for `0x033000` (is it a guard, or was it ever a leaf?).

Also worth building for the next session:
- A **tighter, deterministic reproducer**: a boot‑time stress thread that rapidly
  spawns/blocks/wakes/exits kernel threads across LPs, so the race fires in seconds and fixes
  can be validated quickly. Put it behind a cargo feature.
- Keep any scheduler detectors **silent except on the anomaly** (logging on the hot path
  perturbs timing enough to hide the race — observed with the `[SCHEDDBG]` per‑dispatch log).

---

## 13. Quick file index

- Panic site / exception decode: `crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs`
- IRQ vector (push/pop of volatile regs): `crates/catten/src/cpu/isa/aarch64/interrupts/ivt.asm`
- Context switch + `cond_yield_lp`: `crates/catten/src/cpu/isa/aarch64/lp/ops.rs`
- Thread context / `saved_sp` / `on_cpu`: `crates/catten/src/cpu/isa/aarch64/lp/thread_context/mod.rs`
- Scheduler (`next`, run queues): `crates/catten/src/cpu/scheduler/lp_schedulers/round_robin.rs`
- System scheduler (`abort_thread`, `block_thread`): `crates/catten/src/cpu/scheduler/system_scheduler/mod.rs`
- Threads / `DEAD_THREADS` / reaping: `crates/catten/src/cpu/scheduler/threads/mod.rs`
- Kernel stack allocator + guards: `crates/catten/src/memory/allocators/stack_allocator.rs`
- Arena address map: `crates/catten/src/memory/linear/address_map.rs`
- Page‑table walker / `unmap_page`: `crates/catten/src/cpu/isa/aarch64/memory/paging/walker.rs`
- TLB invalidation: `crates/catten/src/cpu/isa/aarch64/memory/tlb.rs`
- Demo under test: `crates/catten/src/demo.rs`, spawned from `crates/catten/src/main.rs`
</content>
