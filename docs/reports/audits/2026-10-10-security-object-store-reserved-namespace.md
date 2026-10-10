# Object-store reserved-namespace authority (SEC-12, 2026-10-10)

This is the first enforcement increment for
[SEC-12](../audits/2026-10-03-security-audit.md): the local object store's
public connection was store-wide, so any domain holding it could read,
overwrite or delete the signed artifact and executable objects the node
depends on. The store now enforces a reserved-object set at its boundary.
SEC-12 remains **partly corrected**: per-object ownership and read isolation
for ordinary objects are not implemented here.

## Defect

`objstore` accepted every request from any connection without inspecting the
kernel-authenticated sender, and `service ELF`/deployment consumers load ELFs
from the same object IDs. A signed but hostile application could delete or
overwrite `EXECUTABLE_ECHO_ID` and the `0xfffe…` artifact objects, denying
persistent service recovery and the service-manager replacement path. The
loader re-verifies signatures when it maps an ELF, so this is an integrity and
availability attack on the deployment chain, not code execution.

## Correction

- Protocol: `charlotte_protocol_objstore::ERR_DENIED` (7).
- Scope rules:
  [`objstore::is_reserved_object`](../../../crates/catten-services/src/lib.rs#L1342),
  [`reserved_mutator`](../../../crates/catten-services/src/lib.rs#L1349) and
  [`mutation_allowed`](../../../crates/catten-services/src/lib.rs#L1357). The
  reserved namespaces are the artifact tag `0xfffe…` and the executable tag
  `0xffff…`; mutation is allowed only to the kernel principal (`1`) or the
  signed deployment administration service (`clusterctl`).
- Store: requests arrive through
  `Endpoint::try_receive_authenticated`; `OP_CREATE_AT`, `OP_DELETE`,
  `OP_SET_SIZE` and `OP_WRITE` reject a reserved object from any other
  principal with `ERR_DENIED`
  ([objstore.rs:1267](../../../crates/catten-services/src/bin/objstore.rs#L1269)).
  The sender principal is filled by the kernel envelope and is never a
  caller-supplied field.
- Kernel: the persistent echo executable is staged by the kernel with its own
  principal ([store.rs:365](../../../crates/catten/src/service/store.rs#L365),
  called from [el0_nvme.rs:299](../../../crates/catten/src/self_test/el0_nvme.rs#L301)).
  The EL0 object client no longer writes reserved IDs; it now asserts that a
  reserved delete from an ordinary domain returns `ERR_DENIED`
  ([objstore_client.rs:102](../../../crates/catten-services/src/bin/objstore_client.rs#L101)).
- Architecture: [persistent storage](../../architecture/persistent-storage.md)
  documents the boundary.

## Evidence

- Host tests in the `objstore` module cover the namespace partition and the
  allowlist: kernel/`clusterctl` may mutate reserved IDs; `greet` and zero may
  not; ordinary IDs are unchanged for every caller.
- x86_64 Intel VT-d guest: **15 passed, 0 failed, 0 pending** (`0x1bbff`),
  including `[nvme] SUCCESS: storage stack verified`. That path requires the
  kernel staging to succeed and the ordinary client's reserved delete to be
  denied.
- x86_64 AMD-Vi guest: **15 passed, 0 failed, 0 pending** (`0x1bbff`).
- AArch64 live-upgrade guest: **2 passed, 0 failed, 0 pending** (`0x10800`),
  including `[smmu] early initialization complete`, the object-store spawn,
  `[service] persistent upgrade completed` and `[nvme] SUCCESS: storage stack
  verified` — the service manager reads the kernel-staged executable.
- AArch64 security guest: **19 passed, 0 failed, 0 pending** (`0x2003ffff`).
- Workspace host suites: **344 tests, zero failures**.
- Clippy `-D warnings`: AArch64 and x86_64 kernels (default and
  `live_upgrade_test`) plus the bundled AArch64 services.

Kernel SHA-256:

- x86_64 (both IOMMU backends):
  `87f72eeb766001d864e5fa247eabc3ea5ddea13b346846a88f86a90657c4214d`
- AArch64 live-upgrade:
  `077650e30fb246714c5f650257addf55215a0d14acc489a38079ff3510f21d15`
- AArch64 security:
  `eacdcba57c81794fcce3bb6bc01ba7ad99b2681aac8eb050b61e437c3deaa242`

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu intel \
  --instance sec12-intel --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance sec12-amd --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18853 CATTEN_DEPLOY_HOST_PORT=18153 \
  scripts/run-aarch64.sh --live-upgrade-test \
  --instance sec12-upgrade --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18854 CATTEN_DEPLOY_HOST_PORT=18154 \
  scripts/run-aarch64.sh --security-test \
  --instance sec12-sec --fresh-storage --timeout 180
```

## Limits and remaining SEC-12 work

- Ordinary application objects remain store-wide: any caller with a store
  connection can read, overwrite or delete another domain's runtime objects.
  Closing that requires persisted per-object ownership plus a sharing
  model; the header is extensible but the on-disk format and staged images
  would need a migration.
- Reads of reserved objects remain open; artifacts are public and
  independently signature-verified. Confidentiality is not claimed.
- The allowlist is fixed to the kernel and the signed `clusterctl` principal.
  An authenticated operator policy for additional writers, and delegation of
  scoped store connections, are not implemented.
- The protocol has no listing opcode to deny; a future one must enforce the
  same scope.
- The remote S3 connector path is unchanged and remains outside this local
  boundary.

SEC-12 moves from Open to Partly corrected in the
[remediation ledger](../../reference/security-remediation.md).
