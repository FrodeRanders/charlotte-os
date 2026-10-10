# Renewed-audit remediation — SEC-36–38 (2026-10-10)

This records corrections to the
[2026-10-10 renewed security audit](2026-10-10-security-audit.md) for the three
findings left open by the
[SEC-31–35 remediation](2026-10-10-security-audit-remediation.md). It builds on
that commit (`f66f793c`) and the reviewed revision
`2221b5bfa0ce6f303cfe4e07f6c8602060adfbfa`.

## Disposition

| Finding | Disposition | Result |
| --- | --- | --- |
| SEC-36 | Corrected | Discovery bounds live peers and rejects advertised MACs that differ from the frame source; the raft transport bounds learned routes |
| SEC-37 | Corrected | Legacy lookup opcodes deny policy-published registrations and audit the denial; normal legacy/DNS paths are unaffected |
| SEC-38 | Corrected | `SPAWN_UPGRADE` enforces the artifact ceiling and the kernel snapshot reserves fallibly |

## SEC-36: bounded discovery state

The discovery service now refuses new peer MACs past
[`disco::MAX_DISCOVERED_PEERS`](../../../crates/catten-services/src/lib.rs#L1563)
(64, with known MACs still refreshing in place) through
[`peer_learning_admitted`](../../../crates/catten-services/src/lib.rs#L1567)
and its host test. `handle_frame` additionally requires the frame's advertised
identity (the disco header source at bytes 18..24) to equal its real Ethernet
source (bytes 6..12), so one host cannot populate the table with invented MACs
under a single frame source
([disco.rs:497](../../../crates/catten-services/src/bin/disco.rs#L503)).

The Raft transport applies the same bound to routes learned from discovery:
[`MAX_PEER_ROUTES`](../../../crates/catten-services/src/relmsg_transport.rs#L71)
(64) is enforced in
[`add_peer`](../../../crates/catten-services/src/relmsg_transport.rs#L210),
refusing new identities at capacity while refreshing known ones.
`learned_routes_are_bounded_and_refresh_in_place` covers the refusal, the
unchanged count and the in-place refresh.

**Limits:** the cap bounds live state; it does not authenticate discovery
peers (SEC-08). An attacker can still fill the 64 slots with fabricated
identities, evicting legitimate peers until the 30-second TTL expires. The
MAC consistency check raises the cost from a payload field to an Ethernet
source, which a local L2 attacker can still spoof. Discovery remains a
best-effort, unauthenticated bootstrap function; membership authorization is
SEC-08/10 work.

## SEC-37: legacy lookup cannot resolve policy-published registrations

The legacy lookup helpers now deny any live registration whose `authorized`
flag is set:

- immediate [`lookup_or_defer`](../../../crates/catten-services/src/bin/ns.rs#L653)
  and [`try_lookup`](../../../crates/catten-services/src/bin/ns.rs#L696)
  entry points;
- the deferred fulfillment arm in
  [`register`](../../../crates/catten-services/src/bin/ns.rs#L583), with the
  caller's kernel-authenticated identity retained in
  `PendingLookup::Legacy`.

Denials are recorded through the existing bounded audit stream
(`AuthorizationError::RightsDenied`, requested `SEND|CALL`) when the envelope
supplies a valid identity, then answered with `ERR_ACCESS_DENIED`. Legacy
lookups remain outside the authorization boundary: only the
`OP_LOOKUP_AUTHORIZED` path evaluates policy and issues attenuated rights.

The asymmetry is deliberate and matches the existing tests: authorized
lookups may still resolve legacy registrations when a policy rule exists (the
security probe provisions a rule for legacy `echo`), while legacy opcodes can
never reach a policy-published service. The distributed name service registers
committed remote names locally through the legacy opcode and the cluster
services self-register through it, so ordinary boot, discovery, DNS and
upgrade paths are unchanged. `grantctl` is the only authorized registration;
scoped applications receive its connection from the kernel rather than through
a name lookup.

## SEC-38: bounded upgrade snapshot

`SPAWN_UPGRADE` now rejects a persistent ELF whose length is zero or above
`charlotte_launch::MAX_ARTIFACT_ELF_SIZE` before snapshotting, closing the
capability on rejection
([syscall/mod.rs:2069](../../../crates/catten/src/syscall/mod.rs#L2069)). The
shared snapshot helper reserves its caller-sized buffer fallibly
([object.rs:547](../../../crates/catten/src/memory/object.rs#L547)), so a
rejected allocation returns `FrameAllocFailed` instead of aborting the kernel;
the artifact and deployment gates inherit the same safety.

## Validation

- `scripts/run-host-tests.sh`: passed, **342 tests, zero failures** (was 340;
  the discovery-admission and route-cap tests are included; the syscall
  emitter check still passes).
- Workspace Clippy with `-D warnings`: AArch64 and x86_64 kernels passed.
  Bundled AArch64 services Clippy (`--release -Z build-std=core,alloc`)
  passed, covering `ns`, `disco` and the transport.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Fresh-storage AArch64 security guest: **19 passed, 0 failed, 0 pending**,
  `passed_bitmap=0x2003ffff`; scoped probe `checks=0xffff` at publication
  generations 1 and 2; concurrent cancellation retired after 4848 requests.
  The guest exercises the local name service's authorized and legacy paths.
- Two-guest AArch64 `--relmsg-test`: **both guests 18 passed, 0 failed,
  0 pending** (`0x3ffff`) with
  `[relmsg] SUCCESS: two guests exchanged sequenced, acknowledged Ethernet
  messages`. This exercises discovery learning (including the new cap and MAC
  consistency check) and the Raft transport route table across two nodes.

Guest commands:

```sh
CATTEN_HTTP_HOST_PORT=18841 CATTEN_DEPLOY_HOST_PORT=18141 \
  scripts/run-aarch64.sh --security-test \
  --instance sec3638-20261010 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18843 CATTEN_DEPLOY_HOST_PORT=18143 \
  scripts/run-aarch64.sh --relmsg-test --net-listen 12054 \
  --instance sec3638-relmsg-a --mac 52:54:00:12:34:01 --fresh-storage --timeout 120
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18844 CATTEN_DEPLOY_HOST_PORT=18144 \
  scripts/run-aarch64.sh --relmsg-test --net-connect 127.0.0.1:12054 \
  --instance sec3638-relmsg-b --mac 52:54:00:12:34:02 --fresh-storage --timeout 120
```

Validated artifact SHA-256 values:

- AArch64 security-guest kernel:
  `775b18b90d683b5469f8054b80e5d4e7a5094722d17488d9999e8e92017c08c4`
- Two-guest relmsg build kernel:
  `d58fb2e596dfba09d2db7a8ca17c373147bf56cdb0fd69e527498f92e97f8ba5`
- `ns.elf`:
  `d35fe690a55dd4a28fda63df7aa4e60d917dbcc658e8f8c090380c2e62ecf7e5`
- `disco.elf`:
  `ef33d2e7b8b3e8ca18ea079f2381712bbb32e3043bf0773a875c56a63956f566`
- `relmsg.elf`:
  `85b8ee99abc5a65c8b2fbb7c6a724a61546174fe488e7d312980bb6b3f225667`
- `tcpip.elf`:
  `a3ec0f30d7033af16bebe15ed5a2cf608448664121a2314f3fbf7292f6411b27`
- `agent.elf`:
  `97b2b2e79c306f849b1e5cc4fa74cf9059c100fcedff9b4526b6f9ba4b031925`
- `Cargo.lock` (unchanged):
  `6237dbf75d0e29bb7fe1c0dba4301d12d5a298d626fdc89bd05e5d766c50aa76`

The live-upgrade guest build (which would exercise the `SPAWN_UPGRADE` size
bound directly) did not produce a clean run in this environment: its
`live_upgrade_test` kernel (`a9ede00d`, unchanged between two runs) panicked
deterministically during boot at `device/smmu.rs:1113`
(`test_reject_sparse_map`), before any upgrade request, while the PCIe
topology probe thread was still enumerating. The panic is in device-recovery
code untouched by these edits, and the same recovery tests pass in the
security build; it is recorded as an unresolved pre-existing failure, not
attributed to this remediation. SEC-38's normal path is therefore not
guest-validated in this pass; its rejection logic is source-reviewed and the
shared snapshot allocation is host-compiled.

## Limits and remaining work

SEC-36 remains unauthenticated by design; the bound is availability
hardening, not peer identity. SEC-37 does not remove the legacy opcodes'
ability to resolve legacy services or the broader SEC-08/10 exposure of the
cluster segment; the new denial branch itself is source-reviewed and its
audit records are not asserted by a dedicated guest probe (no existing test
performs a legacy lookup of a policy-published service). SEC-38 is
manager-only and does not change the live-upgrade policy. The SEC-07/18 and
production trust/authentication criteria in the
[remediation ledger](../../reference/security-remediation.md) are unchanged.
