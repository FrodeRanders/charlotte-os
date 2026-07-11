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
4. **Full boot + all self-tests pass.** As of the LA57 fix (below), x86_64
   boots all the way through ISA init, physical memory, kernel heap allocator,
   and **every self-test** (completion-capability, syscall dispatch, IPI,
   ShardLocal, ShardMailbox, CQ ring — the same suite that passes on AArch64),
   then proceeds to PCIe device enumeration with zero panics. Only the real-EL0
   test is AArch64-gated (`#[cfg(target_arch = "aarch64")]`).

## RESOLVED: heap #GP (was "bug A") — LA57 / paging-mode mismatch

**Symptom:** a #GP with `RAX = 0xff90000000000000` (non-canonical under 4-level
paging) during kernel-heap large-page mapping.

**Root cause:** `CpuInfo::get_vaddr_sig_bits()` read the CPU's *maximum* linear
address width from CPUID `0x80000008` EAX[15:8]. With `-cpu max` that is **57**
(the CPU supports 5-level paging / LA57). The kernel then selected
`LA_MAP_57BIT`, whose `kernel_allocator_arena` base is `0xff90000000000000` —
canonical only under 5-level paging. But Limine boots with **4-level** paging
(48-bit VAs), under which that address is non-canonical → #GP on first
dereference.

**Fix:** `get_vaddr_sig_bits()` now reports the width of the *active* paging
mode, derived from `CR4.LA57` (bit 12): set ⇒ 57, clear ⇒ 48. This selects
`LA_MAP_48BIT` under Limine's 4-level paging, so all kernel VAs are canonical.

## RESOLVED: silent triple-fault on any fault (was "bug B")

**Root cause:** the panic handler used `logln!`, which routes through
`INT_STATE` — a `LazyLock` whose first-use initialization **allocates from the
kernel heap** (`alloc::vec!`). When a fault occurred before the heap was ready
(or *because* the memory subsystem faulted), the panic handler's `logln!`
triggered a heap access → another fault → re-entered the panic handler →
infinite recursion → stack overflow → triple fault (silent hang).

This masked *every* early fault as a silent hang and was why the heap fault
looked like a mysterious lockup.

**Fix:** the panic handler now uses `early_logln!` (direct serial writes, no
heap, no `INT_STATE`). Verified: a deliberate page fault now prints
`Page fault at RIP=… faulting address=…` and exception delivery + the #PF/#GP
handlers all work correctly. Exception delivery was **never** broken.

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

## Remaining work (B and A both fixed; x86_64 boots + all self-tests pass)

1. **Ring-3 / SYSCALL**: fix the SYSCALL GS base (`IA32_KERNEL_GS_BASE` + a
   per-CPU data area) and return to ring 3 via `IRETQ` (the GDT ordering can't
   satisfy `SYSRET`); then exercise a ring-3 test mirroring `self_test/el0.rs`.
2. **Multi-LP bring-up** (`-smp > 1`): AP startup + per-LP GDT/TSS/IDT (the
   x86_64 async IPI handler is now unstubbed).
3. **Robust CPU-feature handling**: gate `RDTSCP`/`FSGSBASE` on CPUID instead of
   requiring `-cpu max`.
</content>
