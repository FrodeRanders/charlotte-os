# Raft persistence failure diagnostics

Date: 2026-10-15. Source baseline: `1109d05a`, with the preceding audit-process
review's documentation edits retained. This addresses the lost-error gap in
that [review](2026-10-15-audit-process-review.md). It changes no numbered SEC
disposition, stored grammar, runtime ABI, deadline or cleanup/replay policy.

## Finding and correction

The object-store adapter returned `Result<_, ()>` for allocation, mapping,
submission, accepted-call waiting, reply and copying failures. Opening a Raft
pair reduced these to `StoreError::Io`. Mutations combined write and flush with
`and_then(...).is_err()` and emitted one generic fail-stop message. Therefore
the historical `FATAL: failed to persist Raft log state` captures did not identify
whether size change, payload preparation/write, transport waiting or flush failed.

The existing owned adapter now returns `IoError` containing:

- Operation: lookup, read, create, size change, payload write or flush.
- Durable object ID when that protocol operation targets one.
- Stage: lookup, allocation, mapping, unmapping, submission, accepted-call wait,
  reply validation or copy allocation.
- Cause: the original runtime memory/IPC error, exact signed scalar service
  rejection, unusable read result/capacity, allocation rejection or unavailable
  lookup.

`StoreError::Io(IoError)` retains this context through opening, so existing DNS
and Raft startup rejection logs report it. Mutation failure additionally retains
record kind, hexadecimal record object ID, optional encoded byte count and
`Encoding`, `Write(IoError)` or `Flush(IoError)`. Global flush has no object in
its wire request; its enclosing mutation still identifies the affected record.
Logging formats scalar diagnostics only, without payloads or credentials.

For example, the host-injected flush rejection exercises this diagnostic form
(not a captured native failure):

```text
[raft-store] FATAL: persistence rejected: record=Log object=0x800000000000000b bytes=Some(8) cause=Flush(IoError { operation: Flush, object: None, stage: Reply, cause: RemoteStatus(4) })
```

Two limitations remain explicit. The shared naming helper exposes only an
optional connection, so lookup can report only `LookupUnavailable`. Encoding
still returns one rejection for invalid state and allocation failure. Read
results share their register between lengths and statuses: only exact
`ERR_NOT_FOUND` without memory means absence; other unusable replies preserve
the signed result and optional memory capacity without inventing a device cause.

## Ownership and continuation

`PreparedWrite` still prepares both owned buffers before submitting size change.
The same `Connection`, `OwnedMemory`, `MappedMemory`, `PendingCall` and returned
`CallResult` owners handle all ordinary rejection and cancellation. Failed map,
unmap and moved submission retain their returned owners until existing runtime
disposal; no raw capability adoption or manual cleanup ladder is introduced.
The diagnostic error contains no capability, pointer, cleanup receipt or owner.

Production mutations call one fallible persistence helper, then log and exit on
rejection as before. Write rejection prevents flush; flush rejection does not
repeat the accepted write. Success writes and flushes exactly once. Record
mutexes leave before IPC. No retry, new polling, recovery registry, fence
clearing, charge refund or physical cleanup authority is added. C08's existing
Raft transport entry in [cleanup/recovery](../../reference/cleanup-recovery.md)
records the diagnostic extension; broader C18/G1/G2/G7 remain open.

The original IPC exact-exit and subsequent Raft persistence failures remain
unattributed. These new fields cannot reconstruct their missing causes.

## Host and build validation

The final `scripts/run-host-tests.sh` passes **488 Rust tests, zero failed,
four ignored**, plus existing signing-policy, boot-result, secure-boot-result,
CI-input, syscall-emitter, tooling, store-format and signer self-tests.

Five new tests cover signed reply/status preservation, exact read absence versus
length/attachment rejection, propagation of size/payload allocation/map/unmap/
submit/wait/reply errors through both record families, flush rejection after a
completed write, tested diagnostic formatting, encoding rejection before I/O,
and one successful write/flush. The existing seven-position open-failure matrix
now checks exact operation/object/stage/cause rather than a generic Io label.
Failure tests assert the operation sequence and retained contents, preventing a
flush after rejected write or replay after rejected flush.

Transport statuses in these tests are injected at the ObjectStore boundary;
they do not execute native memory allocation/map/cancellation failures in this
client. Existing runtime owner tests run in the same suite. The native runs
below exercise the real successful adapter and process/disk recovery paths.

Both complete service bundles rebuilt and signed through
`scripts/build-catten-services.sh --embed` and
`scripts/build-catten-services-x86_64.sh`. Strict service Clippy (`--bins --lib`,
`--release`, `-Z build-std=core,alloc`, `-- -D warnings`) passes on both custom
targets. Workspace rustfmt and diff whitespace pass. Native runners verify
executable/read-only kernel assembly sections.

An initial host compile rejected passing the never-returning fail-stop function
directly to `unwrap_or_else`; explicit coercing closures corrected it before
the passing suite and all native builds. An additional host `--lib --tests`
Clippy check fails on four unchanged test-code sites: `items_after_test_module`
in socket_state and operations_admission, `byte_char_slices` in the earlier
disk_raft revision test, and `bool_comparison` in lib.rs. Those are source-
confirmed pre-existing expressions, not a successful host-test lint result.
The first attempt at that extra check used a relative manifest from /tmp and
failed before compilation; the corrected absolute-manifest check produced
the four diagnostics above. No lint allowance or unrelated code change is added.

## Native persistence validation

All guests use four LPs, isolated instances, no network, the rebuilt signed
bundles and 180-second runner budgets. The environment uses
`CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
Each guest's storage fixture starts a durable voter, waits for election,
tears down that process and restarts the same identity. All runners exit **0**.

| Guest | Disk | Durable term progression | Authoritative result |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | Fresh | 1 at 9.736262 s → 2 at 10.162464 s | **17/0/0** at 11.890996 s; bitmap `0x1ffff` |
| Same AArch64 instance/kernel | Explicit disk reuse | 3 at 16.422437 s → 4 at 16.886682 s | **17/0/0** at 18.707619 s; bitmap `0x1ffff` |
| x86 / Intel VT-d | Fresh | 1 at 20.816154 s → 2 at 21.255408 s | **15/0/0** at 22.552883 s; bitmap `0x1bbff` |
| Same x86 kernel / AMD-Vi | Separate fresh disk | 1 at 13.832079 s → 2 at 14.253820 s | **15/0/0** at 15.643839 s; bitmap `0x1bbff` |

Every guest also completes all 64 original IPC close/exit episodes and the
four-LP first-entry delivery control. Across four guests that is **256 close/
exit episodes and 16 first-entry controls**, with no kernel panic or persistence
fail-stop. Expected contained user-fault fixture diagnostics are not kernel
panics. No failed native guest run occurred in this batch. Passing these
bounded contexts does not close the historical IPC, lease or persistence items.

Commands, with the environment above:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance raft-diagnostics-arm
./scripts/run-aarch64.sh debug --smp 4 --no-network --reuse-storage --timeout 180 --instance raft-diagnostics-arm
./scripts/run-x86_64.sh debug --smp 4 --no-network --iommu intel --fresh-storage --timeout 180 --instance raft-diagnostics-intel
CATTEN_SKIP_KERNEL_BUILD=1 ./scripts/run-x86_64.sh debug --smp 4 --no-network --iommu amd --fresh-storage --timeout 180 --instance raft-diagnostics-amd
```

No network/DNS replication, HTTP recovery, security-profile rollout, actual
device I/O failure, power-loss injection or production trust qualification ran.
The changed signed DNS binary is rebuilt/linted but is not DNS guest execution
evidence. No native rejection was deliberately induced in the new adapter.

## Artifact identity

These are the executed kernel identities and the staged signed service bytes
used for the fixtures. Concise results and relevant failed checks are inline;
no temporary logs or ignored artifacts are required as published evidence.

| Artifact | SHA-256 |
| --- | --- |
| Executed ARM kernel, fresh/reuse | `d34502ddcf9905af677a75433a512d102730d2cbe4b85e02c308863ad9ec4752` |
| Executed x86 kernel, Intel/AMD | `685e01571e00be39f36e154ca27b60084673517ab550fecda2f3cb21eb85b893` |
| Signed ARM Raft, 384,504 bytes | `17a5d27f523accbd7a8fdb46bb615917a910f9d1a0f01882bec2c0b55b1eec9d` |
| Signed x86 Raft, 348,056 bytes | `09f1c93c1524f2aacaea5cbbaa494ce0b3e38af7aea341e71af890ffda61742a` |
| Rebuilt signed ARM DNS, 924,664 bytes | `298ed0e38457ccd9128aa97df220d2dbe14e03b573dcf3a7a91b6377b85927c5` |
| Rebuilt signed x86 DNS, 949,504 bytes | `85d8a9ae62d6cecd999e8d869934ccf02701c7e8cc60999cfb270923c5493870` |
| objstore_client.rs | `42bc124ad298ce0bb6c17dcdada276cc3e15da2fff1f210f8bae91da5dec1560` |
| disk_raft.rs | `82936a9fba9f5b98dc5c178350692b2760870dab9576778fa150f88483664263` |

The [living persistence contract](../../reference/raft-persistence.md) describes
the new diagnostics and remaining ambiguity. Next failure capture should keep
this complete per-operation line with the matching kernel/service identities
and independent IPC/exit/LP observations before drawing a causal conclusion.
