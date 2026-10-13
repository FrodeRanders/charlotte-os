# Canonical authenticated IPC receive (S05)

Date: 2026-10-13. Continues the ordered [API simplification](../../reference/api-simplification.md)
after S04b2b. This report retains its results and hashes inline; temporary logs
and ignored artifacts are not required to interpret the evidence.

## Contract and implementation

Scalar, blocked and vector receives now have one twelve-register envelope.
Current syscall numbers 71/72/73 are named `IpcRecv`, `IpcRecvBlock` and
`IpcRecvVec`; nine-register numbers 22/27/53 are unassigned. SDK methods
`ipc_recv`, `ipc_recv_block` and `ipc_recv_vec` and owning runtime methods
`Endpoint::try_receive` / `receive` use that envelope. The alternate
`*_authenticated` names, old wrappers, dispatch routes and conditional
register packing are removed. The unused kernel `VectorMessage` type (which
incorrectly suggested a padded result header) is removed; actual vector results
keep their packed u16 count and u64 IDs. Every error clears payload and identity
registers 1 through 11. Success returns the enqueue-time sender generation, signed
principal and supervisor roles. The sender ASID is not re-resolved.

All service callers and the six raw assembly fixture families on both
architectures are migrated together. ARM result pointers move from x9 to x27;
x86 preserves live r12--r14 across receive on its mapped stack. Scalar fixtures
check nonzero generation from actual EL0 receipt. Vector fixtures exercise the
real dispatcher: invalid result storage clears the envelope while preserving
the queued mixed-mode transaction, then valid storage returns exact captured
client generation and all four attachments. Existing reply, queued/delivered
cancellation, endpoint close and budget reconciliation assertions remain.

The register contract changes, so `RUNTIME_ABI` advances from 1 to 2. Signed
CDEPLOY6 target grammar and storage tags stay unchanged. Placement and launch
require exact ABI equality; no ABI 1 decoder, remapping or compatibility alias
is added. All descriptor fixture producers, including additional x86 variants,
now declare ABI 2; all native service images are rebuilt/signed. Markdown,
LaTeX/manual PDF and AGENTS guidance describe the current contract.

See [register/ownership contract](../../reference/ipc-receive.md) and
[execution-target matching](../../reference/execution-targets.md).

## Verified results

- Host Rust suite: **427 passed, 0 failed, 4 ignored**, plus signing-policy,
  boot-result/secure-boot-result Python and syscall-emitter checks. New checks
  reject retired receive numbers and runtime ABI 1 on both architectures.
  Existing descriptor selection/image and catalog placement mismatch tests now
  use explicit ABI 1 rejection. The owning-message test checks identity retention
  alongside exactly-once attachment cleanup.
- ARM/x86 service bundles and ARM catten-user rebuilt and signed. Strict service
  and kernel Clippy passes for both targets (linked-deployment and final
  S3/HTTP profiles); formatting,
  runner syntax and diff checks pass.
- Fresh four-LP linked pair, instances `s05a-arm` / `s05a-x86`, socket port
  19065, MACs `52:54:00:12:34:01/02`, `--deploy-test --fresh-storage --timeout 300`,
  staged bundles and authorized socket networking: **ARM 22/0/0**, bitmap
  `0xcfffff`, at 87.552922 s; **x86 20/0/0**, bitmap `0xcfbbff`, at 40.910603 s.
  Both runners exit 0. Peers report architecture-specific runtime ABI 2 profiles;
  three pinned generations select ARM variant 0, x86 variant 1, then ARM 0.
  Readiness, cooperative migration, forced retirement and denied legacy
  forwarding pass.
- Native vector staging passes at ARM 5.110056 s / x86 4.476793 s. Actual EL0
  scalar/block/cross-root and move/read-borrow/write-borrow/copy/cancellation
  receipts pass on both architectures. Cancellation's two empty receives return
  status 2/2, with no delivery of cancelled attachments. The scalar receipt's
  nonzero generation assertion proves the new register output is executed,
  alongside preserved call state and returned-resource cleanup.
- Fresh ARM TLS RustFS ingress repeat, instance `s05a-s3-repeat-arm`, four LPs,
  port 19082, `--deployment-ingress-test --fresh-storage --timeout 240`:
  **20/0/0**, bitmap `0x303ffff`, at 14.616858 s; runner exit 0. The host's
  signed upload/atomic release and actual S3 GET select ARM variant 0, fetch
  96,864 verified bytes, launch at 20.803784 s and publish readiness at
  21.509798 s. Initial unknown execution-profile requests reject HTTP 409/-23
  before later capacity-qualified admission; they do not replay an accepted RPC.
- Final fresh x86 TLS RustFS/HTTP ingress, instance `s05a-s3-x86`, four LPs,
  deployment/HTTP ports 19083/19080, `--deployment-ingress-test --http-test
  --fresh-storage --timeout 240`: **19/0/0**, bitmap `0x323bbff`, at
  19.879779 s; runner exit 0. The selected x86 variant 1 fetches 32,544 verified
  bytes, launches at 23.847461 s and publishes readiness at 24.513883 s.
  Host signed-release readiness and HTTP controls pass. During host negative
  ingress/liveness probes, the transport logs `ConnectionResetError; retry 1/2`
  and sometimes `retry 2/2`; framing, invalid-body, EOF, abortive-reply and
  idle-peer recovery controls subsequently pass. These retries are retained as
  observations, not evidence of an error-free transport or proof of their cause.
  These bounded retries are also documented in the earlier
  [HTTP progress evidence](2026-10-10-security-http-progress.md); this batch
  does not resolve their exact guest phase.
  In particular, `request` also retries healthy status controls after deliberately
  interrupted peers; subsequent success does not qualify immediate availability.
  The trickled-body check rejects at 30.21 s without restarting its total deadline.
  The returned `charlotte.cluster.v2` view is fresh, leader-served and shows
  greet generation/service generation 1 ready on node `0000000042e21edb`, with
  no object ID.
- ARM security fixture, instance `s05a-security-arm`, four LPs, fresh storage,
  `--security-test --timeout 240`: **19/0/0**, bitmap `0x2003ffff`, at
  56.653202 s; runner exit 0. Independent signing roots and wrong-role root
  rejection pass; two generations deny all five legacy lookups of a
  policy-published endpoint, retain role/principal authorization and fence its
  retired publication. Controller RPC and owned multi-source reply waits pass;
  concurrent cancellation traffic retires after 14,944 requests.
- Manual compiles to 210 pages. Changed pages 129, 170--171 and 196 were rendered
  and inspected; two long identifiers on affected pages were given break points.

## Initial fixture startup failure

The first ARM object-store invocation (`s05a-s3-arm`) failed before guest
execution. The invocation overlapped the security guest and used its same default host
deployment port 8081; QEMU could not install the SLIRP forwarding rule:

```text
-netdev user,id=charlotte-net,hostfwd=tcp::8081-:7444:
Could not set up host forwarding rule 'tcp::8081-:7444'
error: QEMU exited before the 240s test window elapsed
```

Runner exit 1, no guest self-test counts or conclusions. This is a host fixture
port conflict, separate from the prior DMA admission issue. No port-owner capture
was taken; attribution to the overlapping security fixture is an inference from
its shared forwarding port and timing. The fresh repeat
uses `CATTEN_DEPLOY_HOST_PORT=19082` and a distinct instance; the failure remains
recorded here rather than being replaced by a passing repeat.

## Limits and unresolved findings

This changes no numbered audit finding count or cleanup custody. Enqueue owners,
capability escrow, pending delivery, loan pins, cancellation claims, root leases,
readiness registration and terminal retention are unchanged. No polling loop or
new recovery owner is introduced. Trusted raw fixtures can carry zero principal
and roles; those values do not authorize a service operation.

The prior S04b2b initial x86 DMA map admission failure and downstream discovery
expiry remain unresolved, with their original diagnostics and qualification
limits in [that report](2026-10-13-protocol-simplification-records.md). Passing
fresh boots here do not establish the contending owner, physical quiescence or
closure of public DMA-map readiness. Security policy persistence, distributed
policy/hard revocation and authenticated remote application invocation remain
separate work. The next ordered simplification is S06, checked wait APIs and
explicit deadline/admission outcomes.

## SHA-256 artifacts

The linked kernels were hashed before later fixture builds overwrite common
target paths. The local signed fixture bundles are identical on both peers.

| Artifact | SHA-256 |
| --- | --- |
| Linked ARM kernel | `118543b79ca45e480a4593398df0ab40651be1c39136f50fa893acbbcb521153` |
| Linked x86 kernel | `6a561bdb18d2ed287414ac16f2f5c129a063c773fc87fe9d28c917df905c3492` |
| Signed ARM greet ELF | `556d98b8402a10d6a4d0f21469a717950f2a65a9e161b2f56b04afb2b0ba51d3` |
| Signed x86 greet ELF | `3d8be98678d7f6a69328507e0b5dbb743cdec25454c2b8f9e20cabd1fd3579d8` |
| Linked fixture descriptor bundle | `cf3f9ad4dfd53252e330598319416b8410781bc099dc13533b835135b41b1c4d` |
| Updated manual PDF | `21c9129d57540da3fb444a7a3149f562468f4815e76abf6f4883d7b3d9d6bfbe` |
| ARM security boot image | `10d220a5192a4ce2eb01bdef8c1d79d4a2ecf130dd28fcb468beea5d78a594c8` |
| Signed security probe ELF | `caaf5e7b2c2ccb13eaf0a34d848aa4a98ee50a4468797d5b92ba173bd87ec47b` |
| Security admitted descriptor | `8f4321dca35e2f5ef826cbcddb9749c62c4883819bfe2c16e83d8341e919e4f7` |
| Initial unbooted ARM S3 image | `b7ccd59ef40305caec84c83efd717a24cc2e0c72850cc0262924dac58c92df0b` |
| Initial ARM S3 descriptor | `57a4c907522c88b041a4ad590c9efc6d24e300c5f9d890c95f8d502559bf0f7d` |
| Final ARM S3 repeat boot image | `da26cb8f94c2f617b6786c7bc582cc090326cc75bf34dc912d83c9d7d3e4bd71` |
| ARM S3 repeat descriptor | `9d26a40f8e3408ba9b1b8ae07b00b5f22bc397b7834e9113a3a7c3544f1dd447` |
| ARM S3 repeat release | `957623ae3c8ba4356569a7a15f29380f22faf79aea104e5b0e9af016a6906129` |
| ARM security kernel | `45d7ddbae860fb4323063847294e479cee0f798af007994b5eeafb01f73910c5` |
| Final ARM S3 repeat kernel | `9092c1122d24d53ca18e81f7607303fb1962b54fa84df7ad56469a19c9c0a39d` |
| Final x86 S3/HTTP kernel | `aecd379aa8824452f49f9409a3a4946cf4aef3b725c0f8ab21477ac42b2e7464` |
| Final x86 S3/HTTP boot image | `0bd1268742ebd78aa850668a362dfbce870457ae793d1510eed30483c7340fcd` |
| x86 S3 descriptor | `2d300880f89155bdf7d19d56814a0d20a3b0889eef37b01c34c162dddcf28882` |
| x86 S3 release | `70b126a7f85251ec5b5365f22541f1cbd28235614d55abc7842c7c70d765c91a` |
