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
| `charlotte-protocol-msg` | v3 framing, checked parsing, 32-bit message lengths and fragmentation offsets, typed IPC envelopes, and session fencing |
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

The runners' ordinary runtime configuration is intentionally separate from
their verifier selection. They attach a NIC by default, so DHCP, discovery,
cluster, TCP/IP, HTTP, and time services run even when no network-test feature
is compiled. `--net-test`, `--dhcp-test`, `--disco-test`, and related options
register additional target verifiers; `--no-network` is the explicit runtime
opt-out. Tests should never be the mechanism that enables a production
capability.

![Ordinary boot and optional test validators](../manual-v2/figures/boot-and-testing.svg)

The two-guest `--relmsg-test` verifier sends and compares a 70,000-byte
payload. This intentionally crosses v2's 65,535-byte limit and exercises the
v3 IPC envelope, fragmentation, adaptive retry, reassembly, and cumulative
delivery acknowledgement end to end.

`--s3-test --timeout 240` is an explicitly test-only integration fixture. It
adds a local TLS RustFS Docker container, a provisioned S3 service profile, and
an in-guest PUT/HEAD/GET/DELETE verifier. Network, DHCP, time synchronization,
and the S3 client itself are ordinary services; the switch supplies the
ephemeral external server, test credentials/CA, and pass/fail observer. The
VirtIO RNG device and entropy service are part of ordinary QEMU operation, not
test-only support. See
[S3 client service](../reference/s3-client.md#rustfs-integration-test).

`--deployment-ingress-test --timeout 240` extends the same fixture with the
release path. The host uploads the signed `greet` ELF to RustFS, signs and
wraps its `CDEPLOY4` descriptor in a signed `CRELEASE` envelope, atomically
admits the release, and waits on the management API until the exact desired
generation owns the active service name on its assigned node.

The two-guest `--deploy-test` exercises the runtime retirement paths. It moves
the lifecycle-aware `greet` domain between nodes and requires an acknowledged
cooperative exit, then returns the assignment with a test-injected zero grace
period and requires forced termination, generation-safe reaping, and continued
reachability through the replacement generation.

For a deterministic test that does not require networking or cluster
formation, use:

```sh
./scripts/run-aarch64.sh release --shutdown-test --no-network \
    --fresh-storage --timeout 120
```

The isolated verifier launches one probe that drops an owned endpoint and
acknowledges the read-only lifecycle request, then a deliberately unresponsive
probe that must be forcibly terminated and reclaimed after its deadline. The
same switch is available through `scripts/run-x86_64.sh`.

`--kafka-test --timeout 300` similarly adds a disposable three-broker Apache
Kafka KRaft cluster with ephemeral verified TLS listeners and an in-guest verifier.
The verifier covers the TLS handshake, idempotent production, bounded
read-committed consumption, aborted-record filtering, and an atomic
consume-transform-produce transaction with the consumer offset included. The
runner creates a fresh single-partition `charlotte-events` topic and removes
the fixture and its volumes on exit. See
[Kafka client service](../reference/kafka-client.md#docker-integration-test).
Use `--kafka-coordinator-test` to hard-stop the transaction coordinator chosen
by Kafka, or `--kafka-fencing-test` to start a second connector with the same
transactional identity and require the stale producer to fail closed.

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
