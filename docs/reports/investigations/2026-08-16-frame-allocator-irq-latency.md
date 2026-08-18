# Analysis: Long interrupt-masked windows under the frame-allocator lock

> **Historical analysis:** records a code-review finding and the options
> considered for addressing it. See the
> [documentation index](../../README.md) for current status.

Status: **IMPLEMENTED** — Options 1 and 2 below are implemented on branch
`analysis/allocator-irq-latency`; an AArch64 QEMU boot (`--smp 4`) passed 18/18
self-tests. Originates from the August 2026 code review, finding #1: the
interrupt-masking spin `Mutex` protecting `PHYSICAL_FRAME_ALLOCATOR` is held
across page zeroing and copying that can reach 64 MiB in a single call.

---

## 1. Summary

`PHYSICAL_FRAME_ALLOCATOR` is the IRQ-masking spin `Mutex`
(`crates/catten/src/memory/mod.rs:260`, `spin/mutex.rs`). The frame allocator
itself is a bitmap that tracks ownership only — it deliberately does not scrub
pages. The zero/copy loops in `crates/catten/src/memory/object.rs` therefore
hold the lock far longer than the allocator needs: the bitmap
`allocate_frame`/`deallocate_frame` operations are amortized O(1) byte reads,
but the call sites perform 4 KiB × `pages` of `memset`/`memcpy` inside the same
critical section. Because the `Mutex` masks maskable IRQs for the entire
ownership interval, one legal `memory_alloc(64 MiB)` or IPC copy defers every
device, timer, and IPI interrupt on that LP for on the order of tens of
milliseconds under QEMU TCG.

The locking rationale and primitives are documented in
[`docs/reference/locking.md`](../../reference/locking.md); this report records
the specific latency finding and the repair options.

---

## 2. Affected sites

| Site | Lock held | Bulk work under lock | On which path |
|---|---|---|---|
| `allocate()` `object.rs:222-242` | frame allocator | `write_bytes` up to `MAX_MEMORY_OBJECT_PAGES` (16 384 pages, 64 MiB) | `memory_alloc` syscall |
| `copy_to()` `object.rs:697-719` | **both** frame allocator **and** `MEMORY_OBJECTS` registry | `copy_nonoverlapping` up to 64 MiB | IPC copy / move-reply (`ipc/mod.rs:568,730,871,1959`) |
| `snapshot_bytes()` `object.rs:322-346` | registry (read) | copy up to `len` | EL0 ELF load / persistent ELF snapshot |
| `write_bytes()` `object.rs:355-377` | registry (read) | copy up to object size | syscall + IPC result-page write-back |
| `allocate_with_bytes()` `object.rs:281-291` | registry | copy | supervisor load path |
| `create_user_thread_context()` `thread_context/mod.rs:248-268` | frame allocator | zero 4 pages | thread spawn (minor) |

`copy_to` is the worst case: `registry` is taken at `object.rs:675` and never
dropped before the copy loop, and the allocator lock is taken *inside* it — a
64 MiB IPC copy holds two interrupt-masking locks.

A secondary detail: both `allocate` and `copy_to` build their frame list with
`Vec::new()` and `push` while holding the allocator lock. For a large request
the `Vec` reallocates repeatedly inside the critical section, and each growth
takes the talc heap lock — a nested frame-allocator→talc acquisition that also
prolongs the masked window. `Vec::with_capacity(pages)` avoids this entirely.

---

## 3. Correctness: why "just drop the lock" is wrong for `copy_to`

For `allocate`, the fix is trivially sound: a freshly allocated frame has been
removed from the free list and not yet published, so it is exclusively owned by
the allocating thread and may be zeroed with interrupts enabled.

For `copy_to` it is not. The source frames are `PAddr` values cloned out of the
source object's `frames` Vec. Releasing the registry lock before copying lets
the source object's owner `close_cap` it on another LP: the frames are
`deallocate_frame`'d and reallocated, and the copy then reads freed, possibly
reused physical memory. The current code holds the registry lock across the
copy precisely to prevent this. Any fix must preserve a source-frame lifetime
guarantee, not merely shorten the critical section.

The kernel already solves the analogous problem for DMA with a pin count:
`dma_pins`/`exclusive_dma_pins` plus `destroy_when_unpinned` (`object.rs:91-93,
1169-1213`). The copy path needs the same shape.

---

## 4. Options

### Option 1 — Split allocate/zero in `allocate()` (recommended, low risk)

Three phases:

1. `Vec::with_capacity(pages)`; allocate *all* frames under the lock with no
   zeroing and no reallocation (roll back already-allocated frames on failure).
2. Release the lock; zero every frame.
3. Publish under the `MEMORY_OBJECTS` registry lock (unchanged).

Correctness is free (frames are exclusively owned), and the masked window drops
from ~zero-time to ~allocation-time. Also pre-reserving capacity removes the
`Vec`-growth talc acquisitions from inside the critical section.

### Option 2 — `copy_to`: batch-allocate + a source "copy pin"

1. Allocate all target frames under the allocator lock only.
2. Under the registry lock, increment a new `copy_pins` counter on the source
   object and clone `frames`.
3. Release both locks; copy source → target.
4. Re-take the registry lock, decrement `copy_pins`, publish the target object.

`close_cap`/`close_address_space` must treat `copy_pins != 0` exactly like
`dma_pins != 0` (refuse to free; set `destroy_when_unpinned`). This mirrors the
existing DMA discipline and removes *both* locks from the copy loop. It is the
only correct way to make IPC copy lock-free.

The copy pin is a shared-read pin, not only a lifetime reference: while it is
held, new writable CPU mappings, in-kernel writes, writable DMA pins, ownership
changes, and write lends are rejected. Read-only mappings and DMA pins may
coexist. Copy and DMA release share one deferred-destruction check, which frees
the object only after both pin counts reach zero.

### Option 3 — Background zeroed-frame pool

A kernel thread (or per-LP worker) pre-zeroes free frames into a clean list;
`allocate` prefers clean frames and only falls back to inline zeroing when the
pool is empty. Eliminates zeroing latency entirely in steady state, at the cost
of a background thread and memory-bandwidth contention. The right long-term
answer, but a much larger change; Option 1 should remain the fallback.

### Option 4 — Preemption-disable instead of IRQ-mask

Change `PHYSICAL_FRAME_ALLOCATOR` to a lock that disables preemption but not
interrupts, so device IRQs still run and only the owner is non-preemptible.
Structurally removes the IRQ-latency problem, but the preemption-disable
machinery does not exist and interacts with the scheduler. Out of scope here;
worth recording for a future scheduler work item.

### Option 5 — Faster zeroing (DC ZVA / explicit memset tuning)

Shrinks the constant but does not remove the masked window. Worth doing in
addition, not instead.

---

## 5. Recommendation

Do **Option 1 now** (small, provably correct, removes ~99 % of the masked
window), then **Option 2** for `copy_to`, since IPC is the actual hot path.
Treat `snapshot_bytes`/`write_bytes`/`allocate_with_bytes` separately: they are
kernel-internal trust-boundary copies, mostly on the loader path, so either
accept the registry-lock window there for now or extend the same copy-pin idea
later. Fold in `Vec::with_capacity` as part of Option 1.

---

## 6. Validation plan

- Existing host self-test `crates/catten/src/self_test/memory/object.rs`
  (exercises `allocate`, the `MAX_MEMORY_OBJECT_PAGES + 1` rejection, and
  ownership/move paths) must still pass. Its AArch64 coverage also asserts
  copy-pin writer exclusion and copy/DMA deferred destruction after owner exit.
- A QEMU boot (`scripts/run-aarch64.sh`) must show the full self-test suite
  passing, since `copy_to` changes touch the IPC move/reply path.
- A targeted latency probe: while one LP performs a 64 MiB `allocate`/`copy_to`,
  another LP asserts that a timer PPI / device IRQ is still delivered within a
  bound. Currently the only proxy is the frozen-LP signature described in
  [`live-upgrade-stall.md`](live-upgrade-stall.md) §2.4 (DAIF state and the
  per-LP PPI counter); the probe should check that the masked window no longer
  spans the zero/copy.
- `cargo fmt` + `cargo clippy` clean for the touched files.

---

## 7. Key files

- `crates/catten/src/memory/object.rs` — `allocate`, `copy_to`,
  `snapshot_bytes`, `write_bytes`, `allocate_with_bytes`; the `dma_pins`
  pattern to mirror.
- `crates/catten/src/memory/mod.rs` — `PHYSICAL_FRAME_ALLOCATOR` and the
  interrupt-masking rationale.
- `crates/catten/src/cpu/multiprocessor/spin/mutex.rs` — the `MutexCore` that
  masks IRQs.
- `crates/catten/src/cpu/isa/aarch64/lp/thread_context/mod.rs` — the minor
  4-page zeroing case.
- `crates/catten/src/ipc/mod.rs` — `copy_to` call sites (the hot path).
- `docs/reference/locking.md` — the locking-primitive inventory this finding
  belongs to.
