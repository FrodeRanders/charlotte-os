# CharlotteOS test paths

CharlotteOS has two intentionally different test environments. Pure logic
should run on the development host. Kernel behavior and EL0 entry-point wiring
must run on the CharlotteOS target, normally under QEMU.

## Host-testable logic

Run every host-side Rust suite and tool self-test with:

```sh
scripts/run-host-tests.sh
```

The runner currently covers:

| Component | What is tested |
|---|---|
| `charlotte-authorization` | principal binding, role separation, default deny, attenuation, policy and service fencing, one-shot redemption, and bounded state |
| `charlotte-protocol-disco` | discovery encoding, decoding, and malformed input |
| `charlotte-protocol-msg` | framing, checked parsing, fragmentation offsets, and session fencing |
| `charlotte-protocol-net` | NIC status decoding |
| `catten-graft` | Raft election, membership, joining, snapshots, persistence projections, and wire format |
| `charlotte-smoltcp` | receive-queue bounds and clock progression |
| `cluster-sign` | digest, signed metadata, and placement-policy self-tests |

The script invokes Cargo from a temporary directory. This is necessary because
the repository's `.cargo/config.toml` asks Cargo to rebuild `core`, `alloc`, and
`compiler_builtins` for freestanding targets. If an ordinary host test is
started from the repository tree, Cargo discovers that target configuration
and may link a second copy of `core`. The wrapper keeps the pinned toolchain and
absolute manifests while preventing the bare-metal configuration from leaking
into host tests. It discovers crates containing `#[test]` and fails if such a
crate disables its harness, so a new suite cannot silently fall outside the
inventory. CI calls the same wrapper.

## Target-only tests

The `catten` kernel, `catten-user`, and the binaries in `catten-services` are
`no_std`/`no_main` target programs. Cargo's host test harness is not their
execution environment. Their target declarations therefore retain
`test = false`; kernel and service integration behavior is exercised by
`scripts/run-aarch64.sh` and the boot-time self-test registry.

Both architecture runners source `scripts/lib/boot-common.sh` for dependency
validation, Limine configuration resolution, payload hashing, atomic FAT image
construction, and authoritative self-test verdict validation. To create only a
mount-free UEFI boot image from an already-built kernel, use
`scripts/create-boot-image.sh`; the Justfile's `create-image` recipe delegates
to the same implementation.

`catten-rt`, `catten-syscall`, and `charlotte-launch` also retain disabled
standalone harnesses. They contain target runtime/ABI support and currently
contain no dormant `#[test]` functions. Their host-compatible portions are
compiled as dependencies of the `catten-graft` and `charlotte-smoltcp` suites.

At the 12 August 2026 audit, `charlotte-protocol-net` was the only component
that had an actual Rust unit test hidden by `test = false`. Its harness is now
enabled and included in the shared runner. `charlotte-protocol-disco` already
had an enabled harness, but was missing from CI; it is now included as well.

## Rule for new service logic

Keep the thin syscall loop and process entry point in an EL0 binary. Put policy
evaluation, codecs, state machines, bounds, and other deterministic behavior in
a dependency-free or narrowly dependent `no_std` library with an ordinary host
test harness. The authorization implementation follows this rule: the policy
engine is host-tested independently, while connection minting remains target
code and must not be enabled until the kernel supplies an authenticated,
generation-aware caller identity.
