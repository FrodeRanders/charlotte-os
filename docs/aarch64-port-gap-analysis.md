# AArch64 Port Gap Analysis

> Status snapshot of the CharlotteOS / Catten kernel AArch64 (ARM64) port.
> Goal context: assess whether the kernel can run on an Apple Silicon (M2) Mac
> under virtualization (QEMU `virt` machine).

## TL;DR

- The kernel **currently only builds and runs on x86-64**.
- AArch64 scaffolding exists (target spec, linker script, Justfile recipe,
  Limine `BOOTAA64.EFI`, ISA module tree) but the **AArch64 build does not
  compile**: 50 errors.
- AArch64 has roughly **843 lines** of ISA code versus **~4865 lines** for
  x86-64. The core runtime subsystems (interrupt controller, timer, MMU/paging,
  thread context / context switching) are empty stubs or `todo!()`.
- Reaching a bootable AArch64 kernel is a **substantial porting effort**, not a
  configuration change.

## How this was assessed

- Read the ISA abstraction traits under
  `crates/catten/src/cpu/isa/interface/` and compared the x86-64 and aarch64
  implementations under `crates/catten/src/cpu/isa/{x86_64,aarch64}/`.
- Attempted builds with the project's custom target specs:
  - `cargo build -p catten --target target_specs/x86_64-unknown-none-catten.json`
  - `cargo build -p catten --target target_specs/aarch64-unknown-none-catten.json --no-default-features --features acpi`
- Toolchain: `nightly` (per `rust-toolchain.toml`), host `aarch64-apple-darwin`.
  `rust-src` component is required (`rustup component add rust-src`) because the
  build uses `build-std`.

## Build status matrix

| Target | Rust compiles? | Links? | Notes |
|--------|----------------|--------|-------|
| x86-64 | Yes (all crates) | No (on macOS) | Fails only at the final link of the C `flanterm` static lib. `ranlib` warns `not a mach-o file`; `rust-lld` then reports `undefined symbol: flanterm_write`, `flanterm_fb_init`, etc. This is a macOS cross-compilation toolchain issue, not a kernel bug. |
| aarch64 | No | n/a | 50 compile errors (see below). With default features it fails even earlier because `flanterm_bindings`' `bindgen` rejects the custom triple: `version 'catten' in target triple 'aarch64-unknown-none-catten' is invalid`. |

### macOS-specific build caveat

Even x86-64 does not fully link on macOS out of the box because the C toolchain
(`cc`/`ar`/`ranlib` from Apple's toolchain) produces Mach-O objects that
`rust-lld` cannot combine into an ELF kernel. Building the kernel reliably will
likely require a GNU/LLVM cross toolchain (e.g. `llvm` from Homebrew with
`llvm-ar`/`llvm-ranlib`, or a Linux build environment / container). This affects
both architectures and is independent of the AArch64 code gaps.

## The ISA abstraction contract

Generic (arch-independent) kernel code depends on each architecture providing an
implementation of the traits in `crates/catten/src/cpu/isa/interface/`:

| Interface (trait) | Purpose | x86-64 | aarch64 |
|-------------------|---------|--------|---------|
| `init::InitInterface` | BSP/AP bring-up, deinit | Done (`init/{bsp,ap,gdt}.rs`) | Partial (`init/mod.rs` only loads IVT) |
| `system_info::CpuInfoIfce` | Vendor/model, VA/PA bits, ISA extensions | Done | **Done** (`system_info/` reads MIDR_EL1, ID_AA64* regs) |
| `io::*RegNIfce` | MMIO / port IO register wrappers | Done (8/16/32/64, port + MMIO) | Partial (only `IoReg8` MMIO) |
| `memory::MemoryInterface` + `AddressSpaceInterface` | Page tables, map/unmap, translate | Done (4K/2M/1G, PTE walker) | **Stub** (only `get_current`/`load`/`translate_address`; all map/unmap/find/is_mapped are `todo!()`; large/huge page fns missing entirely) |
| `interrupts::LocalIntCtlrIfce` | Local interrupt controller, IPIs, EOI | Done (x2APIC) | **Empty** (`GicRedist` is a unit struct with no impl) |
| `interrupts::DynInterruptDispatcherIfce` | Dynamic vector dispatch | Done | Missing |
| `interrupts::ExternalInterruptControllerIfce` | External IRQ routing (IOAPIC/GIC distributor) | In progress | Missing |
| `timers::LpTimerIfce` | Per-LP timer + timestamp source | Done (APIC timer + TSC) | **Empty** (`ArmGenericTimer` is a unit struct with no impl) |
| `lp::LpIsaDataIfce` + LP ops | Per-LP data, context switch, trampolines, int masking | Done | Partial (masking, ID regs present; **context switch + trampolines missing**) |

## Per-module gap detail (aarch64)

### 1. Interrupt controller — GIC (blocking)
`crates/catten/src/cpu/isa/aarch64/interrupts/gic/mod.rs`
- `GicRedist` is a bare `pub struct GicRedist;` with **no `LocalIntCtlrIfce`
  impl**. Missing `init_lp`, `send_unicast_ipi`, `signal_eoi`.
- The exception vector table (`ivt.asm`) is wired up (`vbar_el1`) and dispatchers
  are declared, but `sync_dispatcher`/`irq_dispatcher`/`fiq_dispatcher`/
  `serr_dispatcher` in `interrupts/mod.rs` are **empty bodies** — no ESR_EL1
  decoding, no GIC acknowledge/EOI, no register save/restore of the trap frame
  beyond the volatile-reg macros.
- Needed: GICv3 (or GICv2 for QEMU `virt`) distributor + redistributor + CPU
  interface driver, SGI-based IPIs, EOI/priority handling, IRQ→handler routing.

### 2. Per-LP timer — ARM Generic Timer (blocking)
`crates/catten/src/cpu/isa/aarch64/timers/mod.rs`
- `ArmGenericTimer` is a bare unit struct; **`LpTimerIfce` not implemented**.
- This single gap cascades into ~14 build errors because generic
  `crates/catten/src/timers/mod.rs` uses `LpTimer::now()`,
  `LpTimer::get_ts_cycle_period()`, `TimerEvent`, and `TimerQueue` which all
  require the trait.
- Needed: `CNTPCT_EL0`/`CNTVCT_EL0` timestamp source, `CNTP_*`/`CNTV_*` compare
  registers for deadlines, frequency from `CNTFRQ_EL0`, interrupt masking, and a
  `print_timer_info()` equivalent (imported by `main.rs`).

### 3. MMU / paging (blocking)
`crates/catten/src/cpu/isa/aarch64/memory/mod.rs`, `memory/paging/mod.rs`,
`memory/tlb.rs`
- `AddressSpaceInterface` only implements `get_current`, `load`, and
  `translate_address` (via `AT S1E1A` + `PAR_EL1`). All of `find_free_region`,
  `map_page`, `unmap_page`, `is_mapped` are `todo!()`.
- The whole large-page / huge-page half of the trait is **not implemented**:
  `PAGE_SIZE`, `LARGE_PAGE_SIZE`, `HUGE_PAGE_SIZE`,
  `find_free_region_large_aligned`, `find_free_region_huge_aligned`,
  `map_large_page`, `unmap_large_page`, `map_huge_page`, `unmap_huge_page`,
  `is_mapped_large_page`, `is_mapped_huge_page` (error E0046).
- `paging/mod.rs` declares `PAGE_SIZE = 64 KiB` (via `crate::common::size`) but
  `MemoryInterfaceImpl::PAGE_SIZE = 4096` — an inconsistency to resolve (choose a
  granule: 4 KiB is simplest for QEMU `virt`).
- `tlb.rs`: `inval_range_kernel`, `inval_range_user`, `inval_asid` are `todo!()`
  (need `TLBI` instruction sequences + `DSB`/`ISB`).
- There is no AArch64 page-table entry (PTE) abstraction or table walker
  equivalent to x86-64's `paging/pte.rs` and `paging/pth_walker.rs`.

### 4. Thread context & context switching (blocking)
`crates/catten/src/cpu/isa/aarch64/lp/thread_context/mod.rs`, `lp/ops.rs`
- `ThreadContext` is a unit struct whose `new` is `todo!()`; it does not derive
  `Debug` (required by generic thread code).
- The generic scheduler (`cpu/scheduler/threads/mod.rs`) calls
  `ThreadContext::create_user_thread_context` and
  `create_kernel_thread_context` — **neither exists** for aarch64.
- No AArch64 equivalent of x86-64's `switch_ctx` / `enter_init_thread_ctx` /
  `user_trampoline` / `kernel_thread_trampoline` naked-asm routines. This is the
  heart of the scheduler; without it there is no multitasking. Needs callee-saved
  register + `SP`/`ELR_EL1`/`SPSR_EL1`/`TTBR0_EL1` save-restore and an
  `eret`-based user trampoline.
- `lp/ops.rs` is missing `get_int_state` and `await_interrupt` (a fn/macro),
  both imported by generic code (`panic.rs`, interrupt tracking, spin mutex).
  A `halt!` macro exists but the naming/signature differs from what generic code
  imports (`await_interrupt`).

### 5. Missing type aliases and modules (mechanical, but blocking)
- `cpu/isa/aarch64/lp/mod.rs` does not define `EicId`, `EicPinNum`,
  `InterruptVectorNum` (x86-64 does). Imported by
  `cpu/interrupt_routing/mod.rs` and the timer code.
- No `cpu/isa/aarch64/constants` module (x86-64 has
  `constants::{interrupt_vectors, msrs, rflags}`). Generic code imports
  `crate::cpu::isa::constants::interrupt_vectors::{ASYNC_IPI_VECTOR,
  LAPIC_TIMER_VECTOR}` unconditionally.
- `crate::common::{size,bitwise}` is referenced by aarch64 code but **does not
  exist** — the real modules are `crate::klib::{size,bitwise}`. (These are import
  bugs in the aarch64 tree.)
- `cpu/isa/aarch64/memory/paging` does not export an `AddressSpace` type that
  `memory/allocators/stack_allocator.rs` imports.
- `MemoryInterface` is imported privately (`E0603`) — a `pub use` visibility fix
  in the aarch64 `memory/mod.rs`.

### 6. Device / platform enumeration (secondary)
- `HwDeviceIfce` has aarch64 variants gated behind `#[cfg(target_arch =
  "aarch64")]` (`ArmPl011Uart`, `ArmGic`, `ArmSmmu`) but the PCIe class matcher
  references `HwDeviceIfce::ArmGpu`, which is **not a declared variant** (only
  `AmdGpu`/`IntelGpu`/`NvidiaGpu` exist). Similarly `SmBusController` is gated
  x86-only yet matched unconditionally in the aarch64 build.
- No PL011 UART driver (the natural early console on QEMU `virt`), and no
  device-tree consumer even though a `devicetree` feature and `flat_device_tree`
  dependency exist. On the `virt` machine, hardware discovery is via **FDT**,
  not ACPI — see firmware section below.

### 7. Cross-cutting issues surfaced by the aarch64 build
- `cpu/multiprocessor/startup.rs` imports `limine::mp::MP_FLAG_X2APIC`
  unconditionally; the pulled-in `limine` crate version does not expose it, and
  it is an x86 concept. SMP bring-up needs an arch-gated path (Limine MP + PSCI
  on ARM).
- `panic.rs` returns `()` instead of `!` once `await_interrupt!` is unresolved —
  will resolve once the LP ops are provided.
- `display`/`flanterm` feature cannot be built for the custom
  `-catten` triple because `bindgen`/`clang` reject the environment component.
  The `.cargo/config.toml` sets `CFLAGS_*`/`BINDGEN_EXTRA_CLANG_ARGS_*` only for
  `x86_64-unknown-none-catten`; an equivalent entry for
  `aarch64-unknown-none-catten` (`--target=aarch64-unknown-none-elf`) is missing.

## Firmware / boot model concern for Apple Silicon + QEMU

- The kernel's firmware abstraction assumes UEFI + ACPI on servers/PCs, and
  ACPI **or** a Flattened Device Tree (FDT) on embedded. `environment/mod.rs`
  gates ACPI on `x86_64` or the `acpi` feature and provides an `arm_smc` module
  and a `devicetree` feature, but `get_pcie_segment_groups()` for devicetree is
  `todo!()`.
- QEMU `-M virt` (what the `qemu-run-aarch64` recipe uses) presents an **FDT**,
  not ACPI, unless `acpi=on` is requested. Booting there realistically requires
  the device-tree path, which is unimplemented.
- Practical route on an M2 Mac: QEMU (TCG or HVF) running the `virt` machine with
  edk2 `QEMU_EFI.fd` + Limine `BOOTAA64.EFI`. Note QEMU HVF acceleration on
  Apple Silicon runs AArch64 guests natively; TCG works but is slower.

## Build / tooling infrastructure gaps

- **`Justfile` bug**: `create-image` always copies `./limine-binary/BOOTX64.EFI`
  to `EFI/BOOT/BOOTX64.EFI`, even for the aarch64 image. AArch64 UEFI needs
  `BOOTAA64.EFI` (present in `limine-binary/`) at `EFI/BOOT/BOOTAA64.EFI`.
- **Linux-only image tooling**: `create-image` uses `losetup`, `parted`,
  `mkfs.fat`, and `mount`, none of which exist on macOS. On an M2 this recipe
  cannot run as-is; it needs a macOS-compatible path (e.g. `hdiutil` + `mtools`
  `mformat`/`mcopy`, or building the image inside a Linux container/VM).
- `just` itself is not installed in this environment (`brew install just`).
- `rust-src` must be added to the nightly toolchain for `build-std`.
- A GNU/LLVM cross toolchain is needed for the `flanterm` C dependency (see macOS
  caveat above), or build headless with `--no-default-features` (drops the
  `display` feature) until the framebuffer console is ported.

## Suggested porting order (dependency-first)

1. **Mechanical unblock**: fix imports (`crate::klib::*` not `crate::common::*`),
   add `constants` module + interrupt-vector constants, add `EicId`/`EicPinNum`/
   `InterruptVectorNum` aliases, fix `HwDeviceIfce` variant mismatches, make
   `MemoryInterface` re-export public, arch-gate `MP_FLAG_X2APIC`, add
   `get_int_state`/`await_interrupt` to `lp/ops.rs`, add the aarch64
   `BINDGEN_EXTRA_CLANG_ARGS`/`CFLAGS` cargo env (or build headless).
2. **ARM Generic Timer** (`LpTimerIfce`) — unblocks ~14 errors and the scheduler
   tick.
3. **GIC driver** (`LocalIntCtlrIfce` + dispatchers) — interrupts, IPIs, EOI.
4. **MMU/paging** — full `AddressSpaceInterface` incl. large/huge pages + TLB
   invalidation, choosing a granule (4 KiB recommended for `virt`).
5. **Thread context + context switch asm** — `create_{kernel,user}_thread_context`,
   `switch_ctx`, trampolines; the core of preemptive multitasking.
6. **Platform enumeration**: PL011 UART early console, device-tree parsing for
   PCIe/SMMU, SMP bring-up (PSCI).
7. **Build/boot glue**: fix `Justfile` for `BOOTAA64.EFI` + macOS image creation,
   QEMU `virt` run recipe validation.

## Key references in-tree

- ISA traits: `crates/catten/src/cpu/isa/interface/`
- x86-64 reference impl (use as the template): `crates/catten/src/cpu/isa/x86_64/`
- aarch64 work-in-progress: `crates/catten/src/cpu/isa/aarch64/`
- Generic timer system: `crates/catten/src/timers/mod.rs`
- Scheduler / threads: `crates/catten/src/cpu/scheduler/`
- SMP startup: `crates/catten/src/cpu/multiprocessor/startup.rs`
- Firmware abstraction: `crates/catten/src/environment/`
- Build recipes: `Justfile`; target spec: `target_specs/aarch64-unknown-none-catten.json`
