# CharlotteOS userspace development

CharlotteOS Rust programs use `catten-rt` as their small userspace runtime.
Run `cargo fmt --all` from the repository root to format every package,
including the separately targeted service and application crates and host
tools. Use `cargo fmt --all -- --check` for a non-mutating CI-style check.
Because `catten-user` is part of that workspace, its sibling
`../../gautelis/sitas` path dependency must be checked out; CI provisions the
pinned revision before running workspace tooling.
The source-level entry contract is:

```rust
#![no_std]
#![no_main]

use catten_rt::Context;

fn main(ctx: Context) -> ! {
    let bootstrap = ctx.bootstrap_connection();
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

## Declaring execution resources

Stack and thread count are protected-domain execution limits, not
application-controlled manifest knobs. For centrally deployed applications,
generation or development records the required number of 4 KiB pages per
thread and maximum active threads in the component deployment plan. Release
tooling places the reviewed values in the signed `CDEPLOY4` descriptor. Every
thread subsequently created in that domain inherits the stack allocation, and
the scheduler counts the bootstrap thread against the signed thread quota.

Choose the value from actual worst-case call depth and stack-resident data,
including generated adapter and language/runtime frames. Prefer heap-backed
buffers for large or input-sized data. The current valid range is 1 through 64
pages (4 KiB through 256 KiB), and the current thread-count range is 1 through
64. The kernel enforces both values exactly and rejects invalid requests rather
than clamping them. A thread publication beyond the quota fails closed by
aborting that protection domain under the current spawn ABI. Built-in launches
and legacy `CDEPLOY1` descriptors use four stack pages and 16 threads;
`CDEPLOY2` preserves its signed stack value and receives the 16-thread
compatibility default. Developers own the estimates, while the deployment
signer and cluster admission retain the right to reject them.

The same developer-owned plan carries `shutdownGraceMillis`, because the
component author knows how long in-flight work and remote teardown may need.
The signed value is bounded to zero through 300,000 milliseconds. Structure
lifecycle-aware code so its owning serving scope returns before calling the
divergent exit primitive; see [Cooperative shutdown](shutdown.md).

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

Application and service code must use `catten_rt::owned` rather than raw
resource-owning capability syscalls. See the dedicated
[userspace resource ownership guide](resource-ownership.md) for the complete
rules and examples.
`OwnedMemory` is a linear capability, `MappedMemory<ReadOnly>` and
`MappedMemory<Writable>` tie slices to the mapping lifetime, and `DmaTransfer`
can only consume an unmapped object. `Completion` and `ReadOperation<'a>` keep
asynchronous resources alive through terminal completion, including the
cancel-and-wait path. `Endpoint`, `Connection`, `ConnectionRef`, `PendingCall`,
`IncomingMessage`, `ReplyToken`, and `CapabilityVector` make close, receive,
reply, move, copy, and borrow behavior explicit;
vector loans remain borrowed until their pending call terminates. `MmioRegion`,
`MappedMmio`, and `Interrupt` provide single-close device ownership while
leaving register access unsafe. `ThreadHandle` retains the spawn-time thread
generation so a delayed join cannot observe a recycled TID.

Protocol-specific scalar resources also need owners;
`catten_services::socket::OwnedSocket` supplies explicit fallible close plus a
best-effort `Drop` fallback for TCP/IP socket IDs.

The raw `catten-syscall` functions remain available at runtime and narrowly
documented driver boundaries, not as a service-development convenience.
Coherent `dma_map` is explicitly unsafe because the driver must
synchronize CPU references and device access itself. Once a raw capability is
adopted with `OwnedMemory::from_raw`, it must not be used independently or
adopted a second time.

The host test suite injects failures at unmap, DMA release, MMIO release,
completion cancellation, and IPC submission boundaries. The shared
`charlotte-lifecycle` crate exhaustively checks the generation decisions used
by both thread joins and timed completion waits.

## Authenticated service lookup

Security-sensitive service discovery uses the name service's authorized
protocol, not `OP_LOOKUP`, `OP_LOOKUP_KEYED`, or a caller-provided identity.
The explicit authenticated receive syscalls supply the sender's exact address-
space generation, stable signed-artifact principal, and deployment roles while
leaving the legacy nine-register receive ABI unchanged.
Callers encode only the service name and explicit requested rights with
`charlotte_authorization::wire::encode_lookup`, copy that request into
`OP_LOOKUP_AUTHORIZED`, and receive a connection attenuated to the policy
decision. Default is deny.

Administration artifacts may set an exact principal/service rule through
`OP_SET_POLICY`; service-manager artifacts may publish through
`OP_REGISTER_AUTHORIZED`. Both operations are authorized from the kernel IPC
envelope. `get_domain_identity()` exposes a workload's own assigned identity
for administration tooling, but placing another identity in request memory
cannot impersonate it. Authorized lookup outcomes are retained in the bounded,
administrator-readable `OP_AUTH_AUDIT` stream.

The older public and bearer-key opcodes remain compatibility paths and do not
provide principal-based authorization. Policy and audit storage are currently
node-local and volatile, and policy updates provide prospective—not selective
retroactive—revocation of connections already issued.

## Building bundled examples

For AArch64, build and stage the service and sitas bundles before invoking the
runner:

```sh
scripts/build-catten-services.sh --embed
scripts/build-catten-user.sh --embed
scripts/run-aarch64.sh debug --hvf --no-network --timeout 10
```

These build scripts use Charlotte's AArch64 target specification and linker
script and validate the generated ELF layout. Generated AArch64 programs are
staged under `target/embedded-services/aarch64-unknown-none/`; they are not
source files and are not version-controlled. `scripts/run-aarch64.sh` exports
that architecture-qualified bundle path while compiling the kernel. A direct
AArch64 kernel build must do the same:

```sh
export CATTEN_AARCH64_SERVICE_BUNDLE="$PWD/target/embedded-services/aarch64-unknown-none"
cargo build --package catten \
  --target target_specs/aarch64-unknown-none-catten.json \
  --no-default-features --features acpi
```

x86-64 uses native ring-3 service ELFs from its own target and bundle directory.
The x86 runner builds, signs, and stages those artifacts automatically before
compiling the kernel:

```sh
scripts/run-x86_64.sh debug --smp 4 --timeout 20
```

The generated services are staged under
`target/embedded-services/x86_64-unknown-none/`, and the runner exports
`CATTEN_X86_64_SERVICE_BUNDLE`. A direct kernel build that enables x86 service
tests must point that variable at an already built and signed native bundle.
Service bundles are architecture-qualified and must never be shared between
AArch64 and x86-64 builds.

`catten-user` currently integrates the sibling `sitas` checkout. CI obtains
`FrodeRanders/sitas` at the pinned revision recorded in the workflow and places
it at the path expected by `crates/catten-user/Cargo.toml`; developer checkouts
may provide the same sibling repository locally.
