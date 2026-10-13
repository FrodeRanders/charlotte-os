# Lean deployment records and current-only storage (S04b2b)

Date: 2026-10-13. Continues [API simplification](../../reference/api-simplification.md)
after architecture-aware rollout and legacy deployment entry-point removal.

## Result

`DeploymentEntry` now contains only generation, signed descriptor and aligned
replica node/variant arrays. Local object ID, diagnostic primary node and
variant-zero digest are removed from application state, snapshots, queries,
observability and its JSON representation. The descriptor remains the source
of identity, requested placement and per-variant keys/digests. Rollout status
computes its representative node from the first assignment; it is not stored
or used to choose a binary. Local upload/cache object names remain separate.

Single-component DNS admission (33) moves the exact descriptor. Redundant
name/object/node/digest request fields and the old envelope opcode 15 are gone.
Follower relay 0x22/0x23 retains session, request correlation and source-route
caller binding with the descriptor; 0x17/0x18 are unsupported. Raft command
0x0e stores only resolved node, selected variant and descriptor; 0x05 is retired.
Apply retains signature, signed pin, singleton policy, capacity/profile,
sequence and exact-state idempotence checks. Release/reassign retain their
committed pins, with no descriptor-less existing-record promotion branch.

Deployment queries use `CDEPQR01` followed by generation, length-prefixed
descriptor and exact replica pins. Catalog snapshots use `CATALOGH` v17,
rejecting v1–v16. Snapshot artifact map keys must match the signed descriptor
name; rejection precedes every live section publication. Raft pair tags advance
to `CRFTSTA2` / `CRFTLOG2`: an old log-only store without an application snapshot
also rejects before replay. Existing rejected bytes are not rewritten or reset.
Only both absent objects permit initialization. There are no production stores
to convert; future deployed-store changes still require explicit,
interruption-tested migration.

Observability uses `CLOBSV2` / version 2, with 28-byte fixed deployment records,
and `charlotte.cluster.v2` JSON. Both boot runners validate the current schema
and absence of object IDs. Markdown and LaTeX/PDF contracts, observers, console
and native fixtures are updated. Historical reports remain unchanged.

See [catalog contract](../../reference/catalog-snapshots.md),
[administration](../../reference/deployment-administration.md) and
[Raft storage](../../reference/raft-persistence.md).

## Initial failed pair and qualification limits

The first linked run used instances `s04b2b-arm/x86`, port 19061, otherwise the
same four-LP fixture settings. x86 NIC startup logged at 14.191463 s:

```text
DMA map rejected pages=3 direction=Bidirectional
DmaMapError { status: 14, phase: 3, cause: 12 }
startup rejected stage=4 error=DmaDomainMap detail=0xc030e
```

Phase 3 is BACKEND; cause 12 is OPERATION_IN_FLIGHT. Further rejection messages
covered another three-page bidirectional map and 128-page receive/transmit
maps. The frouter stayed at stage 1. At `self_test/el0_net.rs:105:22`, its
30-second startup wait expired; x86 ended **15 passed / 1 failed / 4 pending**,
bitmaps `0x1bbff / 0x20000 / 0xcc0000`, runner exit 1. ARM's network fixture
passed, but peer discovery later expired at `self_test/el0_disco.rs:116:26`;
ARM ended **19 passed / 1 failed / 2 pending**, bitmaps
`0x43ffff / 0x40000 / 0x880000`, runner exit 1. Deployment checks did not run.

These messages identify backend admission rejection and downstream readiness
failure, not the contending owner or why it persisted. No DMA, IOMMU, NIC or
physical cleanup code changed in this batch. The corrected repeat changes only
the serial console iterator to satisfy Clippy (`as_chunks`); it does not fix
DMA readiness. Existing retaining map-failure policy and public DMA-map
notification gaps remain. No physical pin/backing measurements were added to
this failed run. A passing fresh boot neither proves quiescence of those
failed owners nor closes that audit boundary. C18
[cleanup evidence](../../reference/cleanup-recovery.md) preserves this scope.

Pre-execution checks also caught one leftover `request.artifact` access after
field removal, a test helper typo, and a console `chunks_exact_to_as_chunks`
Clippy warning; those were corrected before the final checks. No older fields
or unsafe compatibility branch was restored to pass validation.

This simplification changes no numbered audit finding count or cleanup custody.
Capability escrow, root leases, signed admission, exact pins, pending RPC owners,
rollback receipts and uncertain backing retention remain. Infallible snapshot
metadata allocation, per-section reader isolation, power-loss atomicity, public
DMA-map readiness and authenticated remote application invocation remain
separate work. The next ordered simplification is canonical authenticated IPC.

## Passing validation

- Final host suite: **425 passed, 0 failed, 4 ignored** Rust tests, plus signing
  policy, 5 boot-result and 4 secure-boot-result Python tests, and syscall emitters.
  New tests cover retired command/relay/query tags, every truncated current
  command/relay/result prefix, invalid pins, tampered signatures, idempotence,
  snapshot name substitution without publication, retired complete disk pairs
  without an application snapshot, and retired observation versions.
- Rebuilt/signed ARM and x86 bundles (41/37 images); strict service Clippy passes
  on both targets. Strict kernel Clippy passes on both linked-deployment profiles
  and both S3/HTTP profiles. Formatting, runner syntax and diff checks pass.
- Corrected linked four-LP fresh pair: **ARM 22/0/0** (`0xcfffff`), **x86 20/0/0**
  (`0xcfbbff`), both runners exit 0. Instances `s04b2b-repeat-arm/x86`, socket
  port 19063, MACs `52:54:00:12:34:01/02`, `--deploy-test --fresh-storage
  --timeout 300`, staged bundles and the authorized socket-network override.
  Initial port-19061 pair failures are retained separately below.
- Both replicas observed controller query generation 1 / node `0x42e278ea` /
  variant 0, generation 2 / node `0x42e27737` / variant 1, generation 3 /
  node `0x42e278ea` / variant 0. ARM observations: 77.642695, 82.470441 and
  86.887769 s; x86: 31.842811, 36.677563 and 40.955453 s. At ARM 78.230419 s,
  the follower's descriptor-only request relayed through the current tags and
  preserved generation 1. Readiness, cooperative migration and forced retirement
  passed; legacy proxy access stayed denied. Public echo retained positive RPC.
- Real TLS RustFS atomic-release integration: **ARM 20/0/0** (`0x303ffff`),
  **x86 19/0/0** (`0x323bbff`, includes HTTP fixture), both exit 0. ARM first /
  x86 second descriptors selected variant 0/1 respectively and completed actual
  S3 GET, scoped launch and generation-1 ready state. Unknown initial execution
  profile returned HTTP 409 / -23 before later capacity-qualified admission;
  this is expected warmup rejection, not authority to replay an accepted RPC.
- x86 host keyhole returned a fresh `charlotte.cluster.v2` view with `greet`
  ready, generation/service generation 1, demand 12 frames, desired and ready
  node arrays both `["0000000042e21edb"]`, and **no object_id**. Its runner
  asserted the schema and removed-field absence. ARM S3 preceded the final
  JSON label update; x86 HTTP validates that presentation change. Both use the
  already-current lean record codec. No new deployment authority comes from JSON.
- Manual rebuilt in two passes; changed consensus/storage, observability,
  deployment and naming pages rendered and inspected.

S3 commands used `--deployment-ingress-test --fresh-storage --timeout 240`,
instances `s04b2b-s3-arm/x86`, with staged ARM-first/x86-second ELFs and keys
`deployments/greet-arm-s04b2b.elf` / `deployments/greet-x86-s04b2b.elf`.
The x86 run additionally used `--http-test` and host keyhole port 19080.
Native kernel assembly permission checks passed before every boot.

## Artifact SHA-256

These identify executed kernels and the shared signed control-plane bytes;
results and diagnostics are recorded above, with no temporary-log dependency.

| Artifact | SHA-256 |
| --- | --- |
| Initial failed ARM kernel | `55406483f1d9a2eae23c4cce485b543cdb8242f2d7250f07f24ef74a25a51274` |
| Initial failed x86 kernel | `fedb7f55d60fd93e2c067599bc46e584f38f79c81c6b1b2de255ea55e473fa26` |
| Passing linked ARM kernel | `fe3c979385cfee0aa13ed84817dcaaa20655f8abad2a24d7d45f7a502dfe6f8f` |
| Passing linked x86 kernel | `d46a7a7469bba459be1e4006a2a034e06c833f15e590ba624d2130923682befe` |
| Passing S3 ARM kernel | `59b88d8c7f64fdad9e1233864e43669f5160e1b0efc2adab31c28de9b521e363` |
| Passing S3/HTTP x86 kernel | `114c0e5680e2ec9c08e8db7b21819cf32313931683e3f7cdd978f9a70e95d6bf` |
| aarch64-unknown-none signed dns | `85f9c6417f572a7b053210c606843779b4993354d861b717ba8d17d27bfe20b3` |
| aarch64-unknown-none signed clusterctl | `5de42b8e7fc55d2cff41166a3a3987b3544a425722d3dd7b3e7692d8ea81a4f7` |
| aarch64-unknown-none signed agent | `29375d9b9a9d26a6731773fa36a903fa84f74539a24bd884767d17c84cfc17ea` |
| x86_64-unknown-none signed dns | `1ac64716a1f843ac1492d21b2f7e3f608b5463c6e06d71e6d6be5c055ddafa90` |
| x86_64-unknown-none signed clusterctl | `91545a1cfc4207adf62849e3f07c962bd679e67f061a9b37178c6c53fd433b74` |
| x86_64-unknown-none signed agent | `062a96fcc1bcc07e06b0c14b48d78a5123cabd1b26a6d0e51cac017326ea6001` |
| S3 arm greet.cdep | `5217067a5dc98b8080239d69ad1fa3ddec2fa0cf0317c5f7770a41c3a75bf232` |
| S3 arm greet.crelease | `2065b2e09954a603d837e94abde19a8a03d024749ed78e19d25ebcc0f84dc2c6` |
| S3 x86 greet.cdep | `49dba570b7b8a9ed365a4ecfece2ae36d6bbc6795069cb77ef4a3d838d0f2402` |
| S3 x86 greet.crelease | `063c127fd4ba5084e388d1f92acff434fa388f2d0e1ee3e25322d8c72ecf2fd2` |
| Shared six-descriptor linked fixture | `036383d2f8ce0506876ef5e80f01284aa865a12d4adb25d06c79e5358abf1c7a` |
