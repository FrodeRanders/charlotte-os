# Verification of 2026-10-10 security progress

Reviewed revision: `ec8924fe7d319720d54fb1cb0a68ee785ea6540c` (`main`).
The working tree was clean at the start. This review covers the twelve commits
dated 2026-10-10 in Europe/Oslo, from `e0a92701` through `ec8924fe`, against
the audit reports and current contracts. It changes documentation and adds a
standalone historical reproduction; it does not change production code.

## Assessment

The day's changes make substantial, source-supported progress. Seven of the
eight renewed-audit findings have corrections in their documented scope;
SEC-32 correctly remains partial. The five initial kernel commits advance
specific SEC-07/18 ownership and admission boundaries without claiming to
complete either finding. The live-upgrade boot regression has a direct fix;
the stack change mitigates one possible failure mechanism without establishing
the cause of the historical intermittent timeout.

**SEC-12's latest closure claim is premature.** Two object-store paths still
violate the original acceptance criterion: metadata-read failure permits a
foreign read, and automatic ID allocation can alias an existing foreign-owned
`CREATE_AT` object. Both are confirmed below using production method bodies
with a host storage double. Neither was attacked through a guest in this review.
The living ledger returns SEC-12 to partial: **27 corrected, eight partial,
three open**, among the existing 38 numbered findings. Historical reports retain
their original dispositions.

## Commit-to-finding verification

| Commits | Findings | Verified progress and boundary |
| --- | --- | --- |
| `e0a92701` | SEC-07/18 | `PendingPin` retains prepared/detached mapping nodes; quarantine relinks original storage. SMMU walker-cache storage admits before branch publication and follows domain lifetime. This does not establish general heap/principal budgets or device completion under outstanding I/O. |
| `e01543e7` | SEC-07/18 | `PreparingMailbox` composes captured root, fallible payload storage, family charge and authority preparation before local publication guards. Explicit close owns payload and authority through post-guard disposal, root last. Wider caller/IRQ contexts remain separate. |
| `b7065157` | SEC-18 | Closing/final-root owners retain detached mailbox and authority namespaces. Failed final invalidation returns the complete owner; confirmed completion releases metadata before root backing/slot. Physical failure remains terminal. |
| `26e55f5d`, `2221b5bf` | SEC-07/18 | Word operations retain exact roots and admitted queue nodes. Fixed rings allocate fallibly; queue-set/byte charges follow actual backing through losing preparation, final invalidation failure and abandonment. These are requested-byte limits, excluding allocator/namespace overhead and aggregate principal consumption. |
| `f66f793c` | SEC-31 | Advertised totals gate state creation and assembly; assembler reserves fallibly. Host tests reject `0xffffffff` and over-ceiling totals. Existing recorded two-node runs validate legitimate fragmentation, not the hostile wire probe. |
| `f66f793c` | SEC-32 | Tokens require persistent TX scratch; no empty-buffer closure on admission failure. Driver-transfer allocation failure drops a frame. Receiver-retained objects still consume TCP/IP's creator budget and can starve transmission: partial remains appropriate. |
| `f66f793c` | SEC-33/34/35 | Runtime artifact launch uses the fallible transaction; `svc #77` exists and emitter coverage checks wrappers; platform classification follows explicit launch provenance and upgrades inherit the old service's classification. Direct legacy-launch rejection/connector deployment/classification probes are still absent. |
| `98b3dc36` | SEC-36 | Discovery and learned routes have 64-entry bounds; advertised MAC must match Ethernet source. The L2 source remains spoofable; state bounds do not authenticate membership. |
| `98b3dc36` | SEC-37 | Immediate, keyed/named and deferred legacy lookup paths deny policy-published registrations. Existing guests exercise the name service, but do not assert this particular denial or its audit record. |
| `98b3dc36` | SEC-38 | Persistent upgrade ELF size is checked before snapshot; snapshot reservation is fallible. The later boot fix restores normal upgrade execution. No direct zero/oversize rejection guest probe is recorded. |
| `7e3dee9b` | SEC-18 validation support | IOMMU early initialization now includes `live_upgrade_test`, matching the removal of lazy device initialization. The follow-up report supersedes the earlier unresolved boot panic with root-cause and passing AArch64 evidence. |
| `f43fed8b` | SEC-18 | At most three fresh invalidation attempts precede physical stack release; physical/identity/detach errors remain one-shot. The historical lease-timeout cause remains unproven and already-retained pairs are not recovered. |
| `674d4d4c`, `c3eb996c`, `ec8924fe` | SEC-12 | Kernel-authenticated principal, persisted owner/private reads, reserved mutation policy and bounded persisted scope grants are real. Closure is withheld for the two defects below. Operator/management authentication remains SEC-08/10. |

The cleanup/recovery owner-family rows C02/C03/C15/C16/C18 and their evidence
were updated alongside the kernel changes. No additional custody registry or
generic retry authority was introduced. The Markdown storage contract describes
the new policy; the LaTeX storage chapter's object-store section still primarily
describes the layout/operations and should gain the current isolation contract.

## SEC-12 follow-up A: ownership metadata failure permits foreign reads

**Medium, conditional on a transient metadata-read failure.** Introduced by
`c3eb996c`; still present at the reviewed head.

[`ObjStore::metadata`](../../../crates/catten-services/src/bin/objstore.rs#L640)
returns `None` for both a missing object and header-read/validation failure.
The dispatcher at
[`objstore.rs:1401`](../../../crates/catten-services/src/bin/objstore.rs#L1401)
converts either into `(owner=0, attributes=0)`. The
[`may_read`](../../../crates/catten-services/src/lib.rs#L1398) policy makes
owner-zero objects publicly readable for legacy compatibility. `OP_READ` then
calls `read_and_reply`, which reads the header again without checking the
recovered owner. If the second read succeeds, the foreign private payload is
returned. `OP_INFO` has the corresponding retry and exposes its size.

This is a fail-open decision, not just failure to report an I/O error. It does
not require a forged sender, owner-zero object, or scope grant. The review does
not establish that a tenant can deliberately cause the required I/O sequence.

Correction: distinguish NotFound from metadata failure, reject the latter, and
retain the validated header/ownership decision through read/size processing.
Acceptance: inject first-read failure followed by success for a private foreign
object; neither read nor info may reveal data. Test true legacy metadata and
authorized reads separately.

## SEC-12 follow-up B: automatic creation aliases foreign-owned explicit IDs

**Medium, for an ordinary object created at an ID the automatic counter reaches.**
The allocator collision predates today's changes, but ownership enforcement
does not close it.

[`ObjStore::create`](../../../crates/catten-services/src/bin/objstore.rs#L428)
checks existing IDs only in its `Some(id)` branch. `None` consumes `next_id`
without checking the directory, while `CREATE_AT` does not advance that counter.
Both ordinary creation operations are public. On a fresh store:

1. Principal A calls `CREATE_AT(1)`: object 1 records A as owner; `next_id` is 1.
2. Principal B calls `CREATE`: it also returns 1, publishes a second live
   directory record, and replaces the ID-to-directory cache entry.
3. Later lookups resolve B's newly created object. A's original object remains
   allocated but its public ID has been taken over; A can no longer access it
   through the normal ownership check.

This does not show that the current high-range Raft/identity objects can be
reached by a practical number of automatic allocations. It does directly break
the stated isolation of arbitrary ordinary objects created through `CREATE_AT`.

Correction: ensure automatic allocation chooses an unused, non-reserved ID
before publication, with bounded rejection/search; preserve one live directory
record per ID. Acceptance: mix two principals' explicit and automatic creation,
including restart, deletion and collision, and verify the existing object,
owner, content and original allocation remain intact.

## Reproduction evidence

The version-controlled
[`2026-10-10-object-store-review.rs`](evidence/2026-10-10-object-store-review.rs)
copies the reviewed `create`, `metadata` and read-policy bodies, plus the exact
dispatcher fallback. `std::Mutex` substitutes for `spin::Mutex`; directory,
header, allocation and disk operations are host doubles. Successful persistence
is assumed, and one metadata read returns `None`. This establishes the control
flow and duplicate-ID publication, not a real NVMe failure or guest exploit.
The assertions demonstrate the historical defects and are intentionally not
installed as passing regression tests for corrected behavior.

```sh
rustc --edition=2024 -A dead_code \
  docs/reports/audits/evidence/2026-10-10-object-store-review.rs \
  -o /tmp/charlotte-object-store-review
/tmp/charlotte-object-store-review
```

Observed output:

```text
CONFIRMED: failed metadata read authorizes a foreign private object; second header read succeeds
CONFIRMED: automatic creation aliases an existing foreign-owned ID; cache owner changes and two live directory records share the ID
```

Reviewed-source SHA-256:

- `crates/catten-services/src/bin/objstore.rs`:
  `7627d6093d138801949b873a42e4d9d2120a5bc336ed242381118ab3874d06de`
- `crates/catten-services/src/lib.rs`:
  `184fa0e7dbfad8c92a2326f77703411ae0d53d3d32bd1ae23194e3f3d4328dd1`
- Historical reproduction:
  `38c5a67d42a9dd64e62da2041ff5da7a0ebf9524ae261edc07849130dbcb1115`

## Validation performed in this review

- `scripts/run-host-tests.sh`: exit 0; **349 Rust tests passed, zero failed,
  three ignored**, plus five boot-result tests, four secure-boot-result tests,
  signing-policy checks, signing-tool self-tests and **68 syscall wrapper call
  sites across four arities**.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Standalone object-store reproduction: both assertions demonstrated above.
- Fresh-storage AArch64 security guest: **19 passed, zero failed/pending**,
  `passed_bitmap=0x2003ffff`; scoped probe `checks=0xffff` twice and normal
  operator scope installation/foreign-object storage checks passed.
- Fresh-storage Intel VT-d x86 guest: **15 passed, zero failed/pending**,
  `passed_bitmap=0x1bbff`, including the storage stack and Raft process restart.
- Fresh-storage AArch64 live-upgrade guest: **two passed, zero failed/pending**,
  `passed_bitmap=0x10800`; `[smmu] early initialization complete` and
  `[service] persistent upgrade completed` both observed.
- Fresh-storage AMD-Vi x86 repeat: **15 passed, zero failed/pending**,
  `passed_bitmap=0x1bbff`, using the unchanged Intel/AMD kernel and x86 bundle.
  The first AMD run failed as recorded separately below.

Successful guest commands (after rebuilding/signing both architecture bundles):

```sh
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18961 CATTEN_DEPLOY_HOST_PORT=18261 \
  scripts/run-aarch64.sh --security-test \
  --instance audit-review-20261010-unsandboxed --fresh-storage --timeout 180
scripts/run-x86_64.sh --no-network --iommu intel \
  --instance audit-review-intel-20261010 --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18962 CATTEN_DEPLOY_HOST_PORT=18262 \
  scripts/run-aarch64.sh --live-upgrade-test \
  --instance audit-review-upgrade-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance audit-review-amd-repeat-20261010 --fresh-storage --timeout 150
```

Artifact SHA-256 for these fresh executions:

- AArch64 security kernel:
  `6d806577faf59c62c624d8e82ed5b2e1c294c6b1149637258a9091fa073ad8ed`
- Intel and both failed/passing AMD x86 executions' kernel:
  `01481194c1ddaa5fb4489cf8221f9533742e0b78011564564770d34ba81903fe`
- AArch64 live-upgrade kernel:
  `d13fa1b5d525b51d7d4ff6b7f28724bb8c5f85e46d08cea3ad2dfc9d492f14bf`
- AArch64 `objstore.elf`:
  `ff50803afbad034012a07636a66f19c7feac3f76aa5959f4116276e50ecdfc8a`
- x86 `objstore.elf`:
  `6a351393153c2af0cd9e0b2a166e29ea81f44e47a648cff58635374214b3d0c7`
- `Cargo.lock` (unchanged):
  `6237dbf75d0e29bb7fe1c0dba4301d12d5a298d626fdc89bd05e5d766c50aa76`

Two initial AArch64 attempts failed before kernel entry with QEMU's
`Could not set up host forwarding rule` (ports 8081 and 18261 respectively).
The second used the freshly rebuilt AArch64 bundle. These are separate harness
failures, not kernel/test failures; the next attempt ran outside the sandbox.

### Failed AMD-Vi abort-handoff fixture

The first AMD-Vi run used the same x86 kernel hash as the passing Intel run,
with `--no-network --iommu amd --instance audit-review-amd-20261010
--fresh-storage --timeout 150` and `CATTEN_SKIP_EMBED_BUILD=1`.
At guest time approximately 11.65 seconds it panicked before an authoritative
final result:

```text
panicked at crates/catten/src/cpu/scheduler/system_scheduler/domain_abort/tests.rs:72:5:
assertion failed: memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some()
error: authoritative self-test result was not produced within 150s
```

This is an unresolved execution failure in the existing abort-handoff probe.
An immediate global `try_lock` failure does not identify the holder and can
also arise from another LP's legitimate lifecycle activity. The evidence does
not distinguish that contention from an enclosing caller hold, prove an AMD
IOMMU defect, or establish a relationship to the historical stack lease timeout.
It must remain separate from passing runs and is relevant to SEC-18's concurrent
context qualification. No assertion was disabled and no timeout was increased.

After the failed runner exited with status 1, a sequential repeat with unchanged
code, kernel hash and bundle completed with **15 passed, zero failed/pending**
(`0x1bbff`), exit 0. This demonstrates an intermittent fixture failure; it does
not identify its cause or close the concurrency qualification item. The passing
Intel and AArch64 results are also distinct executions, not replacements for
the failed AMD evidence.

## Remaining priorities and limits

Close the two SEC-12 paths before restoring that disposition, then add direct
negative probes for scope application/revocation/remount, legacy lookup of a
policy-published name, over-ceiling relmsg frames, oversized upgrades and TCP/IP
receive retention. The latest object-store reports verify normal granted read
and denied write, but do not assert policy revocation or store-remount persistence.

SEC-04's protected boot/enrollment/custody boundary, SEC-08/10 authenticated
node/operator/management channels and SEC-09 trusted security time remain
production blockers. SEC-07 still requires allocation inventory, aggregate
heap/principal limits and progress under real concurrent pressure. SEC-18 still
requires full caller/destructor qualification, broader recovery/custody,
outstanding-device-I/O/concurrent recovery and physical-platform evidence.
Passing serialized fixtures do not establish these properties or explain
historical intermittent stack-retirement failures.

This is a review of today's deltas and their documented evidence, not a fresh
verification of every older closure, full cryptographic audit, advisory scan,
hostile multi-LP campaign or physical-device qualification. Prior guest results
in the linked historical reports remain prior evidence; they are not silently
counted as fresh executions here. No historical reports were rewritten.
