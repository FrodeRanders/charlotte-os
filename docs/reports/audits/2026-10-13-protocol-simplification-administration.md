# Deployment administration API simplification (S04b2a)

Date: 2026-10-13. Continues the audit-driven removal of internal compatibility
paths after the architecture-aware rollout prerequisite. See the
[ledger](../../reference/api-simplification.md) and
[current contract](../../reference/deployment-administration.md).

## Result and scope

Removed descriptor-less application launch syscall 64, its SDK emitter/wrapper,
`launch_artifact` runtime owner wrapper, both runtime kernel adapters and the
complete kernel implementation. There are no active callers. Scoped launch 76
remains the signed application launch contract; encrypted connector launch and
live upgrade remain separate boundaries. Both ISA entry points now check exact
`SyscallNumber` membership, rather than only its maximum: a hole such as 64
follows the existing unknown-syscall EL0 domain-abort policy before dispatch,
without reaching the trusted dispatch assertion. Number 64 is not reused.

Removed controller deploy opcode 2, DNS packed deploy/query opcodes 8/9, the
optional `DEP1` suffix parser, stored-artifact hashing adapter and console
unsigned deploy command. Negative fixtures submit an opaque payload to retired
opcodes; they preserve no old request writer or retry loop. Controller interface
version 3 requires moved full artifact names for status and rollout. Both
queries validate names and complete returned records before owning result
forwarding. Unsupported requests consume their uniquely received attachments.
Local signed-ELF upload and the key ceremony retain their contracts; the console
rejects upload names longer than eight bytes rather than truncating them.

Updated Markdown contracts/ledger, README, testing guidance and LaTeX/PDF
manual. The architecture guide no longer advertises unsigned raw deploy.
Historical reports are unchanged. The manual was rebuilt in two passes and
changed pages rendered for visual verification.

## Ownership and audit relationship

Query handlers adopt each moved request/reply token once at the raw receive ABI
boundary, use `OwnedMemory`, `ConnectionRef`, `PendingCall::wait` and consuming
`ReplyToken::reply_move`, and drop unused returned owners normally. No accepted
request is replayed; this adds no polling loop, cleanup registry or physical
retry authority. Raw kernel fixtures observe and close reply caps outside IPC.
The existing C18 [cleanup row](../../reference/cleanup-recovery.md) records this
limited scope; other raw handlers and enclosing contexts remain separate work.

Removing descriptor-less launch eliminates that development-key verification
branch historically cited under SEC-11. Live-upgrade trust remains separate,
so no numbered finding status/count changes. Required capability escrow,
rollback owners, root leases, cleanup receipts and uncertain backing retention
remain intact. This is compatibility removal, not removal of safety compensation.

## Verification

- Final `scripts/run-host-tests.sh`: **419 passed, 0 failed, 4 ignored** Rust
  tests, plus signing-policy checks, 5 boot-result and 4 secure-boot-result Python
  tests, and syscall-emitter verification. The new host ABI test rejects 64 and
  retains scoped number 76. No direct retired-syscall EL0 execution was added;
  its domain-abort route is source-reviewed and compiled on both architectures.
- Rebuilt and signed ARM services with `scripts/build-catten-services.sh --embed`
  and x86 services with `scripts/build-catten-services-x86_64.sh` (41/37 images).
- Strict `cargo clippy ... -- -D warnings` passed for services on both JSON
  targets and both kernels with features
  `acpi,virtio_net_test,disco_net_test,disco_cross_node_test,dns_net_test,deploy_net_test,clusterctl_test`.
- One linked, fresh-storage, four-LP QEMU pair passed: **ARM 22/0/0**, bitmap
  `0xcfffff`; **x86 20/0/0**, bitmap `0xcfbbff`. Both runners exited 0. Arguments
  were `--deploy-test --fresh-storage --timeout 300`, instances `s04b2a-arm`
  and `s04b2a-x86`, ARM `--net-listen 19059 --mac 52:54:00:12:34:01`, x86
  `--net-connect 127.0.0.1:19059 --mac 52:54:00:12:34:02`, with
  `CATTEN_SKIP_EMBED_BUILD=1` and the authorized socket-network fixture override.
  Native assembly permissions passed before boot.
- Each controller fixture rejected retired controller 2 with -1 and DNS 8/9
  with -3, answered short and long missing names with -1, and rejected zero,
  malformed and excessive name lengths with -3. Upload identity substitution
  rejection, valid upload and key ceremony also passed.
- Each deployment fixture queried the public controller after exact readiness,
  verifying its complete signed descriptor and aligned node/variant pin. Both
  observed generation 1 / node `0x42e278ea` / variant 0, generation 2 /
  node `0x42e27737` / variant 1, and generation 3 / node `0x42e278ea` / variant 0.
  ARM logged these at 83.666545, 87.626993 and 91.976361 seconds; x86 at
  36.564285, 40.458914 and 44.913006 seconds. This covers successful owning
  result forwarding through controller interface version 3.
- Cooperative migration and zero-grace forced retirement passed. Legacy DNS
  forwarding of the policy-published application remained denied; the earlier
  public echo phase still proved positive cross-node RPC. Authorized remote
  application invocation remains the previously recorded separate gap.
- `cargo fmt --all -- --check` and `git diff --check` passed.

No execution failure was observed in this batch. Prior architecture-prerequisite
failures remain separately recorded in their
[original evidence](2026-10-13-execution-targets.md); these passes do not erase
or identify causes for those earlier samples.

## Artifact SHA-256

These identify the executed kernels and service bytes. Results and relevant
diagnostics are inline above; temporary logs and ignored products are not
required to read this evidence.

| Artifact | SHA-256 |
| --- | --- |
| ARM kernel | `900adefc6fce51d9f1e4d54ae62e2086edfc9152882422f0e0d148a91d087d19` |
| x86 kernel | `2241bbe5228c82bcf655199678b0cf4af85d81c96f24eb3f2698d73272c780fc` |
| ARM signed clusterctl | `07ce924acdb9e5f58749e577aad7b2c99f371fb5ad39f8d05410f6d6b44051e1` |
| x86 signed clusterctl | `c5615458550d68f9935372408fe57fe82a3a39fdc512b970e5d17a31c6f786f6` |
| ARM signed DNS | `dcba1360f63f7b336e87960c0bcb23cf958539ae88218589d5504789536153ef` |
| x86 signed DNS | `5cd28b9f6e5dc34363bb13669aa7d74ea640229701fab2f17db4c143129d6d61` |
| Identical ARM/x86 six-descriptor bundles | `44fff6b37e87ec784d9b68f5333da8e4340ad4e9ce25966b507e519a7b30b1a4` |

## Next ordered work

S04b2b removes diagnostic object IDs, primary nodes and variant-zero digests
from deployment requests, follower relay, replicated commands, snapshots and
queries together. Preserve signed placement, captured capacity qualification,
exact per-replica pins and current-only restore rejection. The upload-only
short-name adapter remains a later tooling candidate. Authenticated IPC
consolidation and checked wait ABI removal follow; none may weaken capability
ownership, admission sponsorship or uncertain cleanup retention.
