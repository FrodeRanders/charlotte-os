# Object-store operator scope policy (SEC-12, 2026-10-10)

This is the third SEC-12 increment, after the
[reserved-namespace authority](2026-10-10-security-object-store-reserved-namespace.md)
and [per-object ownership](2026-10-10-security-object-store-ownership.md).
Those steps made the store enforce fixed policy; this step adds an
authenticated, persisted operator policy that can delegate read or write
rights over object-id ranges. With it, SEC-12's implemented scope covers the
store's object set, ownership and delegation, and the finding moves to
**corrected in the documented scope**.

## Gap

Ownership alone cannot express legitimate sharing: a deployment or
administration workflow may need another principal to read (or write) a
specific object range, and the only sharing mechanism was the object-level
`ATTR_SHARED_READ` bit fixed at creation. There was no authenticated,
persisted, revocable policy at the store boundary.

## Correction

- Wire protocol: `OP_APPLY_SCOPE` (11) attaches a read-only memory object
  holding an encoded table: 16-byte header (`OSCP` magic, version, entry
  count, reserved) followed by up to 64 entries of
  `principal:u64 | first:u64 | last:u64 | rights:u32 | reserved:u32`.
  Rights are read/size (`1`) and resize/overwrite/delete (`2`).
- Policy authority: only the kernel principal or the signed `clusterctl`
  principal may apply a table (`reserved_mutator`); every other caller gets
  `ERR_DENIED`. A malformed table is rejected and leaves the previous table
  in force.
- Persistence: the accepted canonical table is written to the reserved
  `SCOPE_OBJECT_ID` (`0xffff_0000_0000_0002`) through the store's own write
  path and re-applied at mount. A missing table means no grants; a malformed
  persisted table fails closed with a diagnostic log.
- Enforcement: `may_mutate_scoped`/`may_read_scoped` (`catten-services`
  [objstore module](../../../crates/catten-services/src/lib.rs#L1408)) combine
  ownership, sharing and the table for `OP_READ`, `OP_INFO`, `OP_DELETE`,
  `OP_SET_SIZE` and `OP_WRITE`. Scope grants never extend the reserved
  artifact/executable allowlist.
- Fixture: the kernel stages a second kernel-owned probe and applies a
  read-only grant for it to the `objstore_client` principal. The client reads
  the exact granted bytes, is denied delete on the granted object, and still
  sees the ungranted probe denied for both read and delete.

## Evidence

- Host tests: table round-trip/canonical encoding, inclusive range and right
  matching, malformed-table rejection (magic, version, padding, truncation,
  zero principal, inverted range, unknown rights, entry padding), and the rule
  that scope grants cannot extend reserved mutation.
- x86_64 Intel VT-d and AMD-Vi guests: **15 passed, 0 failed, 0 pending**
  (`0x1bbff`) each, including `[nvme] SUCCESS: storage stack verified`. Those
  runs require the granted read, the granted-delete denial, the ungranted
  read/delete denials, kernel staging and Raft restart recovery.
- AArch64 live-upgrade guest: **2 passed, 0 failed, 0 pending** (`0x10800`).
- AArch64 two-guest `--relmsg-test`: **both guests 18 passed, 0 failed,
  0 pending** (`0x3ffff`).
- AArch64 security guest: **19 passed, 0 failed, 0 pending** (`0x2003ffff`).
- Workspace host suites: **349 tests, zero failures**.
- Clippy `-D warnings`: AArch64/x86_64 kernels (default and
  `live_upgrade_test`) plus bundled AArch64 services.

Kernel SHA-256:

- x86_64 (both IOMMU backends):
  `0cbb6f7d74539fc3b3d2292e28418d173c00135ce3a26c999e47457faaf0e853`
- AArch64 live-upgrade:
  `b980dd8781ab96fee66693f7f6370bc7b7304f014ad0bafaaef5af1e58924b5a`
- AArch64 two-guest relmsg build:
  `a8e9bcc653aeee95ec7a8eba6f9ca53b3d544406f44403bb8451de1988041b44`
- AArch64 security:
  `c3f4b036f8a21ee8bb8276e1a0faf074e443967dc1f227de27c1c50f6c6c9fa1`

An initial x86 run failed all scope checks because the kernel embedded a
stale `x86_64` service bundle (only the AArch64 bundle had been rebuilt);
after `scripts/build-catten-services-x86_64.sh` the same revised kernel
passed. That was a build-invocation mistake, not a store regression. Embedded
kernel bundles are per-architecture and must both be rebuilt for cross-target
validation.

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu intel \
  --instance sec12c-intel --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance sec12c-amd --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18859 CATTEN_DEPLOY_HOST_PORT=18159 \
  scripts/run-aarch64.sh --live-upgrade-test \
  --instance sec12c-upgrade --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-aarch64.sh --relmsg-test --net-listen 12058 \
  --instance sec12c-relmsg-a --mac 52:54:00:12:34:01 --fresh-storage --timeout 120
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-aarch64.sh --relmsg-test --net-connect 127.0.0.1:12058 \
  --instance sec12c-relmsg-b --mac 52:54:00:12:34:02 --fresh-storage --timeout 120
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18860 CATTEN_DEPLOY_HOST_PORT=18160 \
  scripts/run-aarch64.sh --security-test \
  --instance sec12c-sec --fresh-storage --timeout 180
```

## Limits and residual risk

- The policy authority is the signed `clusterctl` artifact identity; operator
  enrollment and management-channel authentication remain SEC-08/10. A
  compromised administration service can already mutate artifacts and policy
  by design.
- The table is bounded to 64 entries; broader policy sets need a paged or
  object-backed representation.
- Delegation is principal-scoped data enforced at the store, not an IPC
  capability mint; a delegated principal can exercise the right from any
  connection.
- Pre-ownership images retain unowned objects that are readable but
  kernel-mutable only; recreate them with `--fresh-storage`.
- There is no listing opcode to restrict; a future one must consult the same
  policy.
