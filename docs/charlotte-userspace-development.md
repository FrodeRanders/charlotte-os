# CharlotteOS userspace development

CharlotteOS Rust programs use `catten-rt` as their small userspace runtime.
The source-level entry contract is:

```rust
#![no_std]
#![no_main]

use catten_rt::Context;

fn main(ctx: Context) -> ! {
    let bootstrap = ctx.bootstrap_cap();
    let first_argument = ctx.arg(0);

    // Program or service loop.
    unsafe { catten_syscall::thread_exit() }
}

catten_rt::entry!(main);
```

The program does not define `_start`, a panic handler, or a global allocator.
`entry!` supplies those pieces. `_start` is the ELF entry point; `main` is the
Rust developer contract and is not exported as a C ABI function.

`Context` is the supported interface to launch-time state. It provides launch
arguments, the bootstrap capability, device grants, per-shard completion-queue
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

## Launch ABI v1

Before calling `main`, crt0 validates a fixed-width header in the mapped launch
page. Version 1.1 contains an eight-byte magic value, ABI major and minor
versions, header size, config-page size, feature flags, bounded argument and
capability-vector locations, and the declared heap, input-buffer, and default
completion-queue layouts. An invalid or out-of-bounds layout terminates the
initial thread rather than interpreting unchecked offsets.

The kernel and runtime import this representation from the shared no-std
`charlotte-launch` crate. Compile-time size assertions keep the header and
capability record layouts stable across both sides of the boundary.

The argument count is a `u32`, independent of kernel or userspace pointer
width. The current argument payload remains an array of `u32` values. Future
minor versions may add typed records behind `Context`; applications should not
parse the backing page directly.

Some bundled services still use `config::write` for test progress and status
reporting. That output mechanism is transitional and is separate from the
developer-facing launch-data API.

## Building bundled examples

```sh
scripts/build-catten-services.sh --embed
scripts/build-catten-user.sh --embed
scripts/run-aarch64.sh debug --hvf --timeout 10
```

The build scripts use Charlotte's AArch64 target specification and linker
script, validate the generated ELF layout, and refresh the images embedded in
the kernel self-tests.
