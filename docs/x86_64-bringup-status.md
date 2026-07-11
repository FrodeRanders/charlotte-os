# x86_64 Bring-Up Status

> Status of the x86_64 port of Catten, and a precise map of the current
> blockers. The AArch64 port is the mature, fully-working platform (boots,
> passes all self-tests, runs userspace at EL0 with a working syscall). x86_64
> is substantially behind and is brought up opportunistically.
>
> Boot with: `scripts/boot-x86.sh` (QEMU q35, TCG, `-cpu max`, serial to
> `/tmp/catten-x86-serial.log`).

## What works now

1. **Boot via Limine** — the kernel image loads and `bsp_main` runs.
2. **Serial console** (`log/serial_x86.rs`) — a 16550 UART driver on COM1
   (port `0x3F8`). Wired into `early_log!`/`early_logln!`/`_write_args`. Before
   this, x86_64 headless boot was completely silent (output was discarded).
3. **CPU feature requirement** — the kernel uses `RDTSCP` (for `get_lp_id` via
   `TSC_AUX`) and `WRFSBASE`/`WRGSBASE` (`init_lp_state`). These require a CPU
   model that supports them. `-cpu qemu64` does **not**; `-cpu max` does. The
   boot script uses `-cpu max`. (A real fix would gate on CPUID and fall back,
   or require these features explicitly.)
4. **BSP progresses through**: `assign_id` → ISA init (GDT/IDT/segment reload)
   → physical frame allocator init → into kernel-heap allocator init.

## Current blocker: kernel heap allocator init

`init_primary_allocator` (`memory/allocators/global_allocator.rs`) hangs. The
sequence, confirmed by instrumentation:

1. `try_allocate_and_map_range(base, Large, 1)` allocates a 2 MiB frame
   (`allocate_large_frame -> 0x200000`) and calls the x86_64 `map_large_page`.
2. `map_large_page` completes every step: walk, `ensure_pml4`, allocate + zero
   the PDPT (parent index 0), allocate + zero the PD (parent index 0), and set
   the PD large-page entry. It returns `Ok`.
3. Back in `init_primary_allocator`, the global-allocator lock is acquired
   cleanly (not contended).
4. **The first write to the freshly-mapped heap base VA hangs** — and a full
   CR3 reload (TLB flush) before the write does **not** help.

### Two distinct bugs implied

- **(A) The large-page mapping is not effective.** Writing to the mapped VA
  faults even though `map_large_page` reported success and the TLB was flushed.
  Likely causes to investigate: the 2 MiB PD entry format (PS bit / reserved
  bits / PAT bit 12 / address field alignment in `pte.rs::set_frame` +
  `set_page_size`), or the HHDM write to the page-table frame not reaching the
  live table. Note the arena VA has `pml4_index == 0` and `pdpt_index == 0`
  (a low-canonical address) — worth confirming that is intended and not
  colliding with an existing mapping.

- **(B) The page-fault handler does not fire.** A bad write should raise `#PF`
  and hit `ih_page_fault` (which panics with a message). Instead the machine
  goes silent — a triple fault (QEMU `-no-reboot` halts). This means x86_64
  exception delivery is itself broken: the IDT entry, the ISR asm stub, the
  TSS/IST kernel stack, or the GDT selectors used by the IDT gate are wrong.
  **Fix (B) first** — once faults are visible, (A) becomes debuggable instead
  of silently triple-faulting.

## Known issues beyond the boot blocker (from earlier study)

- **SYSCALL GS base** — `interrupts/syscall.rs`'s trampoline uses `swapgs` and
  `gs:[0x0]`/`gs:[0x8]` for the kernel stack, but `IA32_KERNEL_GS_BASE` is never
  initialized and GS base is 0. A per-CPU data structure must back it.
- **SYSRET GDT ordering** — the GDT is `[null, kcode, kdata, ucode, udata, tss]`.
  `SYSRET` requires `STAR[63:48]+16 = user code` and `+8 = user data`, i.e. user
  code must follow user data. The current order can't satisfy `SYSRET`; either
  reorder the GDT or return to ring 3 via `IRETQ` (recommended — works with any
  GDT layout, matching the existing `user_trampoline`).
- **Ring-3 execution** has never been exercised on x86_64.

## Suggested order for a future x86_64 session

1. Fix exception delivery (B) so faults are visible (verify IDT load, ISR
   stubs, TSS `rsp0`/IST). Add a deliberate fault test.
2. Fix the 2 MiB large-page PDE (A); confirm heap init completes and self-tests
   run (they are architecture-agnostic and already pass on AArch64).
3. Fix SYSCALL GS base + choose `IRETQ` return; exercise a ring-3 test mirroring
   the AArch64 `self_test/el0.rs`.
</content>
