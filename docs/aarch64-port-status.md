# AArch64 Port Status

> Status of the CharlotteOS / Catten kernel AArch64 (ARM64) port.
> Goal context: run the kernel on an Apple Silicon (M2) Mac under virtualization
> (QEMU `virt` machine).

## TL;DR

- The AArch64 kernel **builds and boots end to end** under QEMU `virt` with a
  GICv3. It initialises memory, brings up all secondary cores, passes the
  physical/virtual memory and allocator self-tests, enumerates PCIe via ECAM,
  and runs the scheduler with the ARM Generic Timer driving **stable,
  fault-free preemptive context switches** (hundreds of thousands of timer
  interrupts handled with zero faults, deterministic across repeated runs).
- All six previously-missing subsystems are implemented: ARM Generic Timer,
  MMU/paging (VMSAv8-64), thread context / context switching, GICv3, plus the
  boot-enablement layer (PL011 serial console, MMIO mapping, Limine request
  fixes).
- AArch64 ISA code has grown from ~843 LOC to **~2748 LOC** (x86-64 is
  ~4865 LOC). There are no remaining `todo!()`/`unimplemented!()` stubs in the
  AArch64 ISA tree.
- **Caveats**: the `display`/framebuffer (flanterm) feature now builds and
  boots on AArch64 (see "Display / framebuffer" below), but a usable framebuffer
  is not always provisioned by QEMU/edk2 on `virt`; the kernel falls back to the
  PL011 serial console when none is available, so serial is the reliable log
  sink on macOS. GIC and UART MMIO base addresses are QEMU-`virt` hardcoded
  pending device-tree support; execution reaches the kernel device-probe idle
  loop — no EL0/userspace path is exercised yet.

## How to build and run

```sh
brew install qemu mtools          # one-time
rustup component add rust-src     # one-time (build-std)
rustup component add llvm-tools   # one-time (only needed for --display)
./scripts/run-aarch64.sh          # headless: builds kernel, image, boots QEMU
./scripts/run-aarch64.sh --display  # framebuffer window + serial
# serial console is on stdio; press Ctrl-A X to quit
```

The default (headless) build uses `--no-default-features --features acpi`, so the
PL011 serial console is the log sink and the flanterm C dependency is avoided; it
creates a FAT EFI image with `mtools` (no sudo / loopback mounts) and launches
`qemu-system-aarch64 -M virt,gic-version=3 -cpu cortex-a710`.

The `--display` build additionally enables the `display,virtio_gpu` features
(the flanterm framebuffer console) and boots with a ramfb display window. See
"Display / framebuffer" below.

## What was implemented

### 1. ARM Generic Timer (`cpu/isa/aarch64/timers/`)
`LpTimerIfce` implemented on the EL1 physical timer with the system counter as
the monotonic timestamp source:
- `CNTFRQ_EL0` for the counter frequency, `CNTPCT_EL0` for `now()`.
- Deadlines via `CNTP_CVAL_EL0`; enable/mask via `CNTP_CTL_EL0`.
- Per-LP timer instances; `print_timer_info()` reports frequency/period.
- No programmable divisor and a fixed hardware-wired PPI, so `set_divisor`
  returns `DivisorNotSupported` and `set_isr_dispatch_number` only records.

### 2. MMU / paging (`cpu/isa/aarch64/memory/`)
Full `AddressSpaceInterface` for the 4 KiB granule, 48-bit VA, four-level
(L0-L3) hierarchy matching Limine's translation regime:
- `descriptor.rs`: VMSAv8-64 table/block/page descriptors, `MAIR` indices
  (0 = Normal WB per Limine, 2 = Device-nGnRnE for MMIO), AF, inner-shareable,
  `AP[2:1]` permissions, PXN/UXN.
- `walker.rs`: four-level walker with TTBR0/TTBR1 root selection by address
  half; map/unmap/translate for 4 KiB pages, 2 MiB (L2) and 1 GiB (L1) blocks;
  allocates and reclaims intermediate tables; `map_mmio_page` for device memory.
- `paging/mod.rs`: `AddressSpace` (`ttbr0_el1`/`ttbr1_el1`), `load()` with
  `DSB ISH`/`ISB`, `map_mmio_region` (maps MMIO to its HHDM alias),
  `HwAsid`, size constants.
- **TLB maintenance uses hardware broadcast** (`TLBI ...IS` inner-shareable with
  `DSB`/`ISB`) rather than the x86-style IPI shootdown — the architecturally
  correct, async-first approach on AArch64.

### 3. Thread context & context switching (`cpu/isa/aarch64/lp/`)
Cooperative, callee-saved-register context switching — cheap threads are a
cornerstone of Catten's async-first model:
- `switch_ctx`: saves x19-x30 + `TTBR0_EL1` on the outgoing kernel stack, swaps
  SP, restores with barrier sync; null-`curr` abandons the boot context.
- `cond_yield_lp`: collects switch params under the scheduler locks, releases
  all locks, then switches (so a stack-abandoning switch cannot leak guards).
- `kernel_thread_trampoline` / `user_trampoline` (EL0 via
  `ELR_EL1`/`SP_EL0`/`SPSR_EL1` + `eret`).
- `ThreadContext` with `create_kernel/user_thread_context`, synthesising an
  initial stack frame that matches `switch_ctx`'s restore order.

### 4. GICv3 interrupt controller (`cpu/isa/aarch64/interrupts/`)
- CPU interface via `ICC_*_EL1` system registers (SRE, PMR, Group 1 enable).
- Redistributor power-up (clear ProcessorSleep, wait ChildrenAsleep) + enable
  the per-core timer PPI (INTID 30).
- Distributor enable (affinity routing + Group 1).
- `send_unicast_ipi` via `ICC_SGI1R_EL1`; acknowledge/EOI via
  `ICC_IAR1_EL1`/`ICC_EOIR1_EL1` (per-LP acked-INTID so the argument-less
  `signal_eoi` completes the right interrupt).
- Real exception dispatchers: `irq_dispatcher` acks → advances the timer queue
  (waking observer threads) or drains the IPI queue → EOI → `cond_yield_lp`.
- GIC MMIO mapped as Device memory before first use.

### 5. Boot enablement
- **PL011 UART serial console** (`log/serial.rs`) as the AArch64 log backend,
  mapped as Device memory via the HHDM; `early_log`/`log`/`logln` routed to it.
- `scripts/run-aarch64.sh`: macOS-friendly image build + QEMU run.
- `Justfile`: arch-correct EFI file (`BOOTAA64.EFI`) in `create-image`;
  `gic-version=3` in the aarch64 recipe. `limine.conf`: serial enabled.

### 6. Display / framebuffer (flanterm)
The `display` feature draws log output to a linear framebuffer via the C
`flanterm` library (`log/flanterm.rs`). On AArch64 it now builds, links, and
boots:
- **Build/link fix.** `flanterm` is compiled by the `cc` crate. Its C sources
  cross-compile to ELF correctly (via the `CFLAGS_*`/`BINDGEN_EXTRA_CLANG_ARGS_*`
  target env in `.cargo/config.toml`), but on macOS the *archiver* defaulted to
  Apple's `ar`/`ranlib`, which cannot build a valid archive from ELF objects — it
  silently produced an empty Mach-O archive, so the flanterm symbols went missing
  at link time. The run script points the `cc` crate at the toolchain's ELF-aware
  `llvm-ar` via `AR_aarch64_unknown_none_catten`, resolved from the active Rust
  sysroot (needs the `llvm-tools` component).
- **Graceful fallback.** `FlantermConsole` holds an optional context; if the
  bootloader provides no usable framebuffer (missing response, zero dimensions,
  null address, or flanterm init failure) it reports unavailable instead of
  panicking, and `log!`/`logln!` fall back to the PL011 serial console so output
  is never lost.
- **Status.** When Limine hands over a usable framebuffer (e.g. `ramfb` with a
  real display surface) flanterm initialises (observed 800×600×32). GOP mode
  provisioning on QEMU aarch64 `virt` + edk2 is itself flaky between boots and is
  outside the kernel's control; the serial fallback covers the gap.

## Bugs found only by booting

These were invisible at compile time and were diagnosed via QEMU's `-d int`
exception log and `lldb` on the QEMU gdb stub:

1. **Limine requests garbage-collected.** The request statics (including the
   base-revision marker) were unreferenced and dropped by `--gc-sections`, so
   Limine fell back to base revision 0 — which it now *rejects* on AArch64 —
   and refused to boot. Fixed by placing requests in `.limine_requests*`
   sections marked `#[used]` and `KEEP`-ing them in both linker scripts.
2. **`HHDM_BASE` resolved to 0.** The shared `VAddr::from` applies x86 canonical
   sign-extension (bit 47 as sign), which zeroes AArch64's HHDM base of
   `0xffff_0000_0000_0000` (bit 47 clear). Fixed by storing the bootloader
   offset verbatim via `from_raw_unchecked`. This was the "x86 canonical rules
   should work on aarch64" assumption in the code being wrong.
3. **FP/SIMD trap.** The `+neon`-compiled kernel used FP/SIMD before
   `CPACR_EL1.FPEN` was set, faulting as "undefined instruction". Fixed by
   enabling FP/SIMD early on every core.
4. **EL1t vs EL1h.** Running in EL1t meant interrupts pushed state onto an
   invalid `SP_EL0`, producing a runaway fault storm. Fixed by forcing EL1h
   (SP_ELx) early (copying the active SP into SP_EL1 before selecting it) and by
   making the IVT dispatch the SP_EL0 exception group as well.
5. **GICv2 vs GICv3, and unmapped MMIO.** QEMU `virt` does not default to GICv3;
   the driver requires `-M virt,gic-version=3`. GIC (and UART) MMIO also had to
   be explicitly mapped as Device memory because Limine only HHDM-maps real RAM.

## Remaining work / known limitations

- **Display/framebuffer.** The `display` feature (flanterm framebuffer console)
  now builds and boots on AArch64; see the "Display / framebuffer" section below.
  The remaining rough edge is that a usable framebuffer is not reliably
  provisioned by QEMU/edk2 on `virt` (GOP mode setup is flaky and requires a real
  display surface), so the kernel falls back to the serial console when none is
  present. On macOS the `display` build additionally requires the toolchain
  `llvm-ar` (ELF-aware archiver) rather than Apple's `ar`; this is wired up
  automatically by `scripts/run-aarch64.sh --display`.
- **Device-tree discovery.** GIC distributor/redistributor and PL011 base
  addresses are hardcoded to the QEMU `virt` defaults. They should be read from
  the `/intc` and `/pl011` device-tree nodes; the `devicetree` feature and
  `flat_device_tree` dependency exist but the consumer (`get_pcie_segment_groups`
  for DT) is still `todo!()`. ACPI parsing works on `virt` (which does expose
  ACPI), so this is not blocking today.
- **No EL0 / userspace exercised.** `user_trampoline` and
  `create_user_thread_context` are implemented but only kernel threads have been
  run; the EL0 drop and syscall path are untested.
- **SPI / external interrupt routing.** Only PPIs (timer) and SGIs (IPIs) are
  wired; device SPIs will need the `ExternalInterruptControllerIfce` path
  (GICD `IROUTER`/config) once drivers attach.
- **Self-tests.** x86-specific hardcoded HHDM debug probes were gated out for
  AArch64; the portable map/write/read/unmap VMM test does run and pass.
- **Not runtime-hardened.** The port boots reliably in QEMU but has not been
  tested on real hardware, under stress, or with KASLR.

## Verified boot output (abridged)

```
Catten Kernel Version 0.8.1
LP 0..3 designated and initialised (4 cores online)
All physical memory subsystem tests passed.
All virtual memory tests passed!            (maps/reads/unmaps a higher-half page)
Kernel allocator self-test: PASSED
Testing Complete. All Tests Passed!
CPU Vendor: ARM    PA bits: 40    VA bits: 48
The ARM Generic Timer frequency is 62500000 Hz.
[ACPI] Finished parsing XSDT. Found 5 tables.  (FADT, MCFG, SPCR, GTDT, MADT)
LP 0: Probing device topology...
  Segment Group 0 (ECAM @ 0xffff820000000000, buses 0x00-0xff)
    00:00.0  PCI Express Host Bridge
    00:01.0  Red Hat VirtIO
    00:02.0  Red Hat VirtIO
```

Steady state: hundreds of thousands of timer IRQs serviced with **0 faults**.

## Toolchain / environment notes

- Toolchain: `nightly` (per `rust-toolchain.toml`); `rust-src` required for
  `build-std`.
- Host tested: `aarch64-apple-darwin` (Apple M2), QEMU 11.x, edk2 aarch64
  firmware shipped with QEMU (`edk2-aarch64-code.fd`).
- x86-64 Rust build is unaffected by the port; on macOS its default (display)
  build still fails at the flanterm C link step for the same Apple-`ar` reason
  described above — the `AR=llvm-ar` fix is currently wired only for the aarch64
  display build in `scripts/run-aarch64.sh` and could be applied to x86-64 too.

## Key references in-tree

- ISA traits: `crates/catten/src/cpu/isa/interface/`
- x86-64 reference impl: `crates/catten/src/cpu/isa/x86_64/`
- aarch64 implementation: `crates/catten/src/cpu/isa/aarch64/`
- Serial console: `crates/catten/src/log/serial.rs`
- Generic timer system: `crates/catten/src/timers/mod.rs`
- Scheduler / threads: `crates/catten/src/cpu/scheduler/`
- Firmware abstraction: `crates/catten/src/environment/`
- Run script: `scripts/run-aarch64.sh`; target spec:
  `target_specs/aarch64-unknown-none-catten.json`
