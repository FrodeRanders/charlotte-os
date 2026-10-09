# Prepared unified capability authority records

This continues the [namespace-storage correction](2026-10-09-security-capability-namespace-storage.md)
within C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-07 and SEC-18 remain partial: **20 corrected, six partial and four open**
findings, eighteen owner families and seven acceptance gates. No additional
registry, owner-family row or recovery interface is introduced.

## Defect and correction

The outer unified namespace map already used admitted nodes, but each namespace's
authority records still used an infallible `BTreeMap`. Insertion could allocate
under `CAPABILITIES` after charge and serial mutation. Removal, cancellation and
batch source revocation could destroy map storage and original charges there.

Both levels now use the existing shared
[`retirement_list::AdmittedMap`](../../../crates/catten/src/klib/collections/retirement_list.rs).
[`PreparingRecord`](../../../crates/catten/src/capability/record.rs) owns a fallibly
prepared node before capability serialization and charge/serial mutation. Admission
stores the charge in the same owner until insertion; identity exhaustion retains
it until ordinary explicit completion after capability unlock. Publication only
relinks prepared storage. Unused preparation finishes explicitly; inert Drop
retains unfinished node/charge without locks, allocator work or logging.

Removal detaches a `RetiredRecord` containing the exact node, entry and original
account/class charge. Explicit release refunds only that charge after capability
unlock. Abandonment retains the entire node; namespace teardown, later platform
promotion and successor ASID reuse cannot refund or reclassify it. Final namespace
completion drains its existing admitted storage one record at a time without a
teardown snapshot, then releases its namespace node/account.

Batch publication validates all sources and destinations before mutation. A moved
source's detached authority node stays in its exact `SourceEscrow`, inside the
containing [`PreparedTransfer`](../../../crates/catten/src/memory/object.rs), through
payload completion and pin release. Explicit disposal follows memory-registry
unlock. Loans restore their existing authority in place; copies have no retiring
source. Interrupted ownership is retained, not reconstructed by scalar ASID/cap.
This does not establish interruption safety for every surrounding payload path.

Captured admission may still own lifecycle/subsystem guards. Ordinary removal
and active reservation/escrow Drop leave their local capability guard before
disposal, but can still have outer IPC/device/lifecycle holds. Those existing
token destructors enter the capability registry and are not inert fallbacks.
The new preparation/retirement owners do not erase these G1/G4 obligations.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Record storage rejects | `AllocationFailed` before serial/charge or authority mutation. |
| Prepared storage; policy/identity rejects | Owner retains unused node and any original charge; ordinary explicit completion occurs after capability unlock. |
| Captured admission/publication | Relinks admitted storage with exact namespace account/generation. No record-node allocation/destruction under the local guard. |
| Source escrow/loan restoration | Existing entry changes state; no node allocation or retirement. |
| Mixed publication rejects | Full validation precedes mutation; source escrow and hidden destination remain owned for ordinary rollback. |
| Successful batch move | Detached source node/charge remain in the exact containing transfer until payload completion and memory unlock. |
| Authority/namespace detach | Existing node ownership moves without allocation/destruction; original charges survive. |
| Explicit release | Destroys admitted node once and refunds its original charge; no numeric re-adoption or physical retry. |
| Preparation/retirement abandonment | Terminal retention of node/charge/account. No destructor cleanup, retry ticket or forced refund. |

## Execution evidence

Serialized kernel fixtures reject record storage for all six kinds and verify
unchanged next serial, domain charge and node counts. They preprepare nodes, then
hold the **actual heap allocator** while exercising captured admission,
publication, escrow/restoration and typed detach. All six detached records retain
their charges until post-guard explicit release. Identity exhaustion separately
verifies ordinary unused-node/charge completion.

A mixed move/loan/copy batch publishes while the heap is held. The moved source's
original charge survives detach until its explicit completion; the loan remains
usable. A second batch rejects a retired destination before any source/output
mutation, then rolls back the exact source and cancels only the staged destination.
Cancellation probes cover staged Drop, active escrow Drop, trusted teardown of
escrowed authority, and late tokens after root close and same-ASID reuse.

Guarded abandonment holds lifecycle, capability and actual heap guards while
dropping charged preparation and retirement owners. Both original ordinary
charges survive platform promotion, exact-root teardown and successor reuse:
**two capability-record charges, two record-node allocations and one shared
original account control block** remain retained. This adds no physical
table/data/root retention. The preceding empty namespace node/account retention
is independent and unbudgeted. Existing IOMMU table-only retention remains
VT-d **38 charges/29 frames**, AMD-Vi **34/25**, SMMUv3 **40/31**; separately
charged data/root fixtures are not included in those table-only totals.

Selected focused diagnostics on each target:

```text
[capability record phases] 20 prepared-node and 18 disposal boundaries outside local capability/heap guards; entry IRQ state preserved
[capability record ownership] all six kinds reject storage before serial/charge mutation; heap-held captured admission/publication/escrow/detach; mixed batch rejection is atomic, move retains its exact retired source charge until explicit completion; cancellation and late tokens preserve exact accounts; guarded preparation/retirement abandonment retains two original ordinary charges after root reuse
```

The same capability/heap boundary probes also run through real NVMe recovery:

| Target | Registered tests | Real recovery record preparation/disposal boundaries |
| --- | ---: | ---: |
| Intel VT-d | 15 passed, zero failed/pending | 34 / 35 |
| AMD-Vi | 15 passed, zero failed/pending | 31 / 32 |
| Arm SMMUv3 | 19 passed, zero failed/pending | 32 / 33 |

These counts cover a selected real sequence, including disposal of previously
prepared authority; they are not a whole-boot balance or leak count. Local probes
check capability/heap availability and preserve each operation's entry IRQ state,
including already masked callers. Bounded availability checks tolerate another
LP's transient ownership; they do not prove full outer-context freedom or stress
cross-LP races. Injection simulates admission rejection rather than real OOM.

Final fresh-storage runs execute sequentially with reused, previously validated
service bundles because this change affects the kernel only:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance capability-record-intel-final-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance capability-record-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18739 CATTEN_DEPLOY_HOST_PORT=18039 scripts/run-aarch64.sh --security-test --instance capability-record-arm-20261009 --fresh-storage --timeout 180
```

Final x86 passed bitmap is `0x1bbff`; Arm is `0x2003ffff`, with scoped
security checks `0xffff` at publication generations 1 and 2; concurrent
cancellation retires after 4,952 requests.
Kernel SHA-256:

- x86, shared by final Intel and AMD runs:
  `5a0e9d0dd7371024afe836709b9afe069f1d8cc32ebf4a69e81d464417b2acf5`.
- Arm: `e0b67ccc66e50bc8b635048dfba80df30d86d4f4e0fdee1547f41f2ebdc86cf1`.

Strict locked default-feature Clippy passes on x86 and Arm (`-D warnings`). Formatting and diff checks pass. Boot runners also validate
native assembler section/load permissions: 249 x86 entries and one Arm entry. Shared retirement-list/`AdmittedMap`
source and its eight host tests are unchanged; the preceding
[namespace report](2026-10-09-security-capability-namespace-storage.md) records their
passing full-host-harness execution. That harness is not rerun in this kernel-only
pass; the new kernel ownership fixtures execute in all three guests.

## Failed initial run

The initial Intel artifact
`9170d5e8087ce85373bf92372fb5abcd91cf8a45c1c1aca43d460c26594175a2`
panicked in the new fixture before reporting completed self-tests. A server
restart interrupted collection of the runner's final status, but the matching
serial capture and symbol sidecar preserved this failure:

```text
capability/record_tests.rs:26: assertion left == right failed
left: false; right: true
0xffffffff80056f85  capability::record_tests::boundary+0xe5
0xffffffff8008dd68  PreparingRecord::try_new+0xc8
```

The probe compared every operation with the suite's starting IRQ state. Existing
captured callers can legitimately retain an outer mask, so this assertion did not
measure preservation of their entry state. The corrected probe captures each
operation's entry state and verifies it after allocation or before/after explicit
disposal. It also tolerates transient remote capability/heap contention with a
bounded availability check. The final Intel repeat above passes after this
fixture correction; the failed artifact remains a separate result. It supplies
no causal evidence for the historical intermittent user-stack lease timeouts.

## Remaining work

Outer lifecycle/IPC/device allocation and destruction contexts, active token
fallback, backend registry metadata and other payload/control-block storage
still require qualification. Ordered map lookup/insertion scans are linear;
this is not a pressure/performance benchmark. Capability record ceilings do not
bound all metadata bytes, empty namespaces or service-principal reuse, and do not
guarantee physical OOM recovery or progress. Hardware timeout, cross-LP stress,
complete custody/reconciliation and the historical intermittent lease timeouts
remain open. Passing repeats do not close SEC-07/18.
