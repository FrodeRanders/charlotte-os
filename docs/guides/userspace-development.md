# CharlotteOS userspace development

CharlotteOS Rust programs use `catten-rt` as their small userspace runtime.
The source-level entry contract is:

```rust
#![no_std]
#![no_main]

use catten_rt::Context;

fn main(ctx: Context) -> ! {
    let bootstrap = ctx.bootstrap_cap();
    let mode = ctx.manifest_value(catten_rt::manifest_key(b"mode"));

    // Program or service loop.
    unsafe { catten_syscall::thread_exit() }
}

catten_rt::entry!(main);
```

The program does not define `_start`, a panic handler, or a global allocator.
`entry!` supplies those pieces. `_start` is the ELF entry point; `main` is the
Rust developer contract and is not exported as a C ABI function.

`Context` is the supported interface to launch-time state. It provides launch
manifest values, the bootstrap capability, device grants, per-shard completion-queue
layout, live-upgrade handoff state, and explicit bounded startup reads. Programs
should not depend on config-page virtual addresses or field offsets.

Initial authority is encoded as a bounded vector of typed capability records.
`Context::capabilities()` enumerates their kind, rights metadata, flags, and
handle; role-oriented helpers such as `bootstrap_cap()` and `mmio_cap()` search
the same vector. Presence is represented by a record, so handle zero is not
mistaken for an absent capability.

The former `fn(Args, Input<N>) -> !` entry form has been removed. Startup input
is no longer hidden in a function signature; a program explicitly calls
`Context::read_startup_input` when it intends to block for input.

## Launch ABI v2

Before calling `main`, crt0 validates a fixed-width header in the mapped launch
page. Version 2.0 contains an eight-byte magic value, ABI major and minor
versions, header size, config-page size, feature flags, bounded manifest and
capability-vector locations, and the declared heap, input-buffer, default
completion-queue, and mutable status layouts. An invalid or out-of-bounds layout
aborts the entire domain rather than interpreting unchecked offsets or leaving
sibling threads running with an invalid launch contract.

The kernel and runtime import this representation from the shared no-std
`charlotte-launch` crate. Compile-time size assertions keep the header and
capability record layouts stable across both sides of the boundary.

The manifest is a bounded vector of named, typed records. Keys are stable
packed names of up to eight bytes; values may be unsigned integers, signed
integers, or bounded byte strings. Keys may repeat to represent lists. Variable
data resides in a separately bounded region of the read-only launch page.
Applications consume it through `Context::manifest()` or
`Context::manifest_value()` and should not parse the backing page directly.

The launch page is mapped read-only in EL0. Mutable program status and test
progress use a separate zeroed status page; `config::read`, `config::write`,
and `config::output_ptr` address that page for low-level programs. Applications
should still prefer their service protocol or completion queues for normal
results rather than treating the status page as general IPC.

## Ownership-aware resources

Application code should prefer `catten_rt::owned` over raw capability syscalls.
`OwnedMemory` is a linear capability, `MappedMemory<ReadOnly>` and
`MappedMemory<Writable>` tie slices to the mapping lifetime, and `DmaTransfer`
can only consume an unmapped object. `Completion` and `ReadOperation<'a>` keep
asynchronous resources alive through terminal completion, including the
cancel-and-wait path. `Endpoint`, `Connection`, `PendingCall`, and
`CapabilityVector` make close, move, copy, and borrow behavior explicit;
vector loans remain borrowed until their pending call terminates. `MmioRegion`,
`MappedMmio`, and `Interrupt` provide single-close device ownership while
leaving register access unsafe. `ThreadHandle` retains the spawn-time thread
generation so a delayed join cannot observe a recycled TID.

The raw `catten-syscall` functions remain available for runtime and driver
protocols. Coherent `dma_map` is explicitly unsafe because the driver must
synchronize CPU references and device access itself. Once a raw capability is
adopted with `OwnedMemory::from_raw`, it must not be used independently or
adopted a second time.

The host test suite injects failures at unmap, DMA release, MMIO release,
completion cancellation, and IPC submission boundaries. The shared
`charlotte-lifecycle` crate exhaustively checks the generation decisions used
by both thread joins and timed completion waits.

## Building bundled examples

```sh
scripts/build-catten-services.sh --embed
scripts/build-catten-user.sh --embed
scripts/run-aarch64.sh debug --hvf --timeout 10
```

The build scripts use Charlotte's AArch64 target specification and linker
script and validate the generated ELF layout. Generated programs are staged
under `target/embedded-services/aarch64-unknown-none/`; they are not source
files and are not version-controlled. `scripts/run-aarch64.sh` exports that
architecture-qualified bundle path while compiling the kernel. A direct
AArch64 kernel build must do the same:

```sh
export CATTEN_AARCH64_SERVICE_BUNDLE="$PWD/target/embedded-services/aarch64-unknown-none"
cargo build --package catten \
  --target target_specs/aarch64-unknown-none-catten.json \
  --no-default-features --features acpi
```

Non-AArch64 kernel builds neither require nor embed this bundle. Each future
EL0-capable architecture must provide its own native service build and
qualified bundle directory.

`catten-user` currently integrates the sibling `sitas` checkout. CI obtains
`FrodeRanders/sitas` at the pinned revision recorded in the workflow and places
it at the path expected by `crates/catten-user/Cargo.toml`; developer checkouts
may provide the same sibling repository locally.
