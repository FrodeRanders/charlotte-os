# Object-store per-object ownership (SEC-12, 2026-10-10)

This is the second enforcement increment for
[SEC-12](../audits/2026-10-03-security-audit.md), following the
[reserved-namespace authority](2026-10-10-security-object-store-reserved-namespace.md).
The previous step protected artifact and executable objects; ordinary
application objects were still store-wide, so any domain holding the public
store connection could read, overwrite or delete another domain's persistent
state. Persisted per-object ownership and read isolation now enforce the
owner at the store boundary.

## Defect

Every ordinary object was created, read, resized, overwritten and deleted
through the same public connection with no owner recorded and no sender check
beyond the reserved namespaces. A hostile signed application could read the
DNS/Raft persistent state, corrupt another service's persistent objects, or
delete them, defeating the persistence the node's recovery depends on.

## Correction

The extensible object header (offsets 64 and 88 of the 384-byte header, zero
in images written by earlier builds) now stores the creating principal and an
attribute word; CRC coverage and the shared encode/decode path keep the format
compatible with image-staged objects.

- Policy ([objstore module](../../../crates/catten-services/src/lib.rs#L1350)):
  - `may_create`: ordinary IDs are open; reserved IDs require a reserved
    mutator, so creation cannot squat the artifact/executable namespaces.
  - `may_mutate`: reserved IDs follow the kernel/`clusterctl` rule; ordinary
    owned objects are mutable only by their owner or the kernel; unowned
    legacy objects are kernel-only.
  - `may_read`: reserved objects are public (signature-verified artifacts),
    `ATTR_SHARED_READ` objects admit every authenticated domain, unowned
    legacy objects stay readable for compatibility, and owned ordinary
    objects are private to their owner and the kernel.
- Store ([objstore.rs:1273](../../../crates/catten-services/src/bin/objstore.rs#L1290)):
  creation records the kernel-authenticated sender as owner; `OP_READ`,
  `OP_INFO`, `OP_DELETE`, `OP_SET_SIZE` and `OP_WRITE` evaluate the policy
  before touching data and reply `ERR_DENIED` otherwise. Header rewrites
  preserve owner and attributes.
- Shared identity: `OP_CREATE_AT_SHARED` (with `ATTR_SHARED_READ`) is used by
  `NodeIdentity::load_or_create`, because discovery, DNS and the Raft fixture
  intentionally read the same node-identity object from different principals.
  Mutations still follow ownership.
- Namespace hygiene: Raft persistent IDs are now confined to the `0x8xxx`
  partition (`disk_raft.rs`), keeping them disjoint from the identity
  (`0x9xxx`), artifact (`0xfffe…`) and executable (`0xffff…`) namespaces.
- Fixtures: the kernel stages a kernel-owned ordinary probe object; the EL0
  object client asserts that reading and deleting a foreign principal's
  object returns `ERR_DENIED`, while its own object still round-trips, and
  keeps the reserved-executable delete denial from the previous increment.

## Evidence

- Host tests cover creation/mutation/read policy: owner-private ordinary
  objects, shared-read, reserved namespaces, unowned compatibility and the
  kernel principal.
- x86_64 Intel VT-d and AMD-Vi guests: **15 passed, 0 failed, 0 pending**
  (`0x1bbff`) each, including `[nvme] SUCCESS: storage stack verified`. Those
  runs require the foreign-object denial probes, the kernel staging, the
  restart-based Raft recovery and the ordinary object round trip.
- AArch64 live-upgrade guest: **2 passed, 0 failed, 0 pending** (`0x10800`)
  with the DNS-owned persistent Raft store surviving replacement under the
  ownership checks.
- AArch64 two-guest `--relmsg-test`: **both guests 18 passed, 0 failed,
  0 pending** (`0x3ffff`) with the fragmented reliable-message exchange;
  discovery and DNS share the node identity through `ATTR_SHARED_READ`.
- AArch64 security guest: **19 passed, 0 failed, 0 pending** (`0x2003ffff`).
- Workspace host suites: **346 tests, zero failures**.
- Clippy `-D warnings`: AArch64 and x86_64 kernels (default and
  `live_upgrade_test`) plus bundled AArch64 services.

Kernel SHA-256:

- x86_64 (both IOMMU backends):
  `358c5add290e5b5cdab4d3a95da8364bfb3b28f848df5997adaf1818176d8d69`
- AArch64 live-upgrade:
  `6afdb61e95e354bcc5726992d3fbdba70f795f208b8270da6f9bb3f30632f961`
- AArch64 security:
  `2250fe8e8e0a7b95e474c380d13e60ff969bf01ed857d1b8a48ad1542ea63ec5`

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu intel \
  --instance sec12b-intel --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance sec12b-amd --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18855 CATTEN_DEPLOY_HOST_PORT=18155 \
  scripts/run-aarch64.sh --live-upgrade-test \
  --instance sec12b-upgrade --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-aarch64.sh --relmsg-test --net-listen 12056 \
  --instance sec12b-relmsg-a --mac 52:54:00:12:34:01 --fresh-storage --timeout 120
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-aarch64.sh --relmsg-test --net-connect 127.0.0.1:12056 \
  --instance sec12b-relmsg-b --mac 52:54:00:12:34:02 --fresh-storage --timeout 120
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18857 CATTEN_DEPLOY_HOST_PORT=18157 \
  scripts/run-aarch64.sh --security-test \
  --instance sec12b-sec --fresh-storage --timeout 180
```

## Limits and remaining SEC-12 work

- Unowned legacy objects (written before ownership enforcement) remain
  readable for compatibility and immutable except to the kernel. An image
  last written by an older build must be recreated (`--fresh-storage`) before
  its runtime services can mutate their objects.
- Read isolation has one explicit sharing mechanism (`ATTR_SHARED_READ`); an
  operator-provisioned read delegation for application-to-application sharing
  does not exist.
- The allowlist for reserved mutation is still fixed to the kernel and the
  signed `clusterctl` principal.
- Any future listing opcode must apply the same policy.
- The owner is a principal, not a capability: a replacement domain of the same
  principal can access its predecessor's objects by design (needed by
  live-upgrade and restart recovery).

SEC-12 remains **partly corrected**: the object set is now enforced against
the owning principal, but an authenticated operator policy and scoped
connection delegation are still required for the full acceptance criteria.
The [remediation ledger](../../reference/security-remediation.md) records the
state.
