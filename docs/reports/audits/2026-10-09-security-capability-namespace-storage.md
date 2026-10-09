# Prepared unified capability namespace storage

This follows the [device registry storage correction](2026-10-09-security-device-registry-storage.md)
within C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-07 and SEC-18 remain partial: **20 corrected, six partial and four open**
findings, eighteen owner families and seven acceptance gates. This changes the
existing unified namespace registry; it adds no registry or recovery interface.

## Defect and correction

User registration prepared a fallible capability budget account, but namespace
publication still allocated an outer `BTreeMap` node after ASID publication under
lifecycle/table/capability serialization. Missing permanent-kernel/raw-fixture
namespaces also allocated account and map storage inside `CAPABILITIES`.
Namespace removal could destroy its map node, remaining authority entries,
charges and account while that guard remained held. Empty preparation's implicit
field destruction did not distinguish ordinary cancellation from abandonment.

The device registry's ordered owning-node adapter is now shared as
[`retirement_list::AdmittedMap`](../../../crates/catten/src/klib/collections/retirement_list.rs).
Device storage uses the same implementation. The outer unified namespace map
uses it too; individual authority records retain their existing inner `BTreeMap`.
Lookup and sorted insertion are linear scans. Relinking and detaching prepared
nodes perform no allocator work.

[`PreparingNamespace`](../../../crates/catten/src/capability.rs) owns both a
fallibly prepared registry node and its original budget account in retaining
`ManuallyDrop` storage. User registration prepares them before local lifecycle/
table guards and ASID publication. Publication installs the exact handle by
relinking its node. Ordinary unused preparation consumes
`cancel_unpublished` after local guards leave, including slot rejection and Arm
hardware-tag rejection. Drop retains both allocations without allocator, registry,
physical cleanup or logging. There is no unprepared namespace insertion branch.

For permanent-kernel/raw-fixture admission, a short namespace preflight leaves
`CAPABILITIES` before fallible preparation. Admission rechecks under the guard;
if another publisher supplied the namespace, unused private storage remains
owned and is disposed after unlock. Generation-bearing user namespaces are never
lazily recreated. Their exact account/generation checks remain unchanged.
These callers can still have outer lifecycle/subsystem guards; that context is
separate G1 work.

Final namespace teardown retires admission and detaches its complete owning node
under `CAPABILITIES`, then leaves the guard before explicit destruction.
Remaining entries and the original account travel in that node. Charges survive
detachment until their entries are destroyed; late tokens still retain their
original account identity and cannot revoke a successor's reused serial.
The containing teardown's outer lifecycle/subsystem context remains separate.
This is metadata completion after payload drain, not a new physical retry proof.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Node/account preparation rejects | No namespace/ASID publication; ordinary first-node cleanup leaves local publication guards. No capability serial or charge is consumed. |
| Prepared user namespace | Complete node/account remain owned before lifecycle/table publication. |
| Slot or hardware-tag admission rejects | Unused namespace storage cancels explicitly after local guards; the rejected root follows its existing explicit rejection adapter. |
| Namespace publication | Exact generation and original account publish by relinking admitted storage; no namespace-node allocation/destruction. |
| Another raw/kernel namespace publisher wins | Existing namespace remains; redundant private preparation is explicitly disposed after capability unlock. |
| Final namespace detach | Node owns all remaining records and original charges; no allocation/destruction occurs during extraction. |
| Explicit detached-node release | Entry charges and account release after capability unlock. No reconstruction or replay by numeric ASID. |
| Abandonment/interruption before explicit disposal | Prepared or detached node fallback retains its allocations and owned dependencies. There is no retry ticket, forced refund or destructor cleanup. |

## Execution evidence

Serialized boot fixtures inject rejection before node preparation and before
account preparation after an actual node allocation. Both return
`CapabilityNamespaceAllocationFailed` from user registration without changing
root/namespace occupancy, physical frames or capability charges. Boundary probes
require local lifecycle/root-table/kernel-table/physical/heap/capability guard
availability during these registration preparations and ordinary cancellation.
These hooks simulate allocation rejection; they do not exhaust the actual heap.
Existing repeated slot-rejection registration tests still execute.

A real, nonrunning user root provides an exact handle for publication with the
**actual heap held**. A charged namespace is detached with the heap held too;
its complete node retains the original charge until explicit release outside
the capability guard. A replacement namespace on the same root receives the
same capability serial with a different account. Publishing the old staged token
returns `Retired`, refunds no replacement charge and leaves replacement authority
usable. Final exact-root close releases the replacement account.

A deterministic private-map fixture exercises the helper state after another
namespace publisher wins; redundant preparation remains owned for post-guard
cancellation. This is not an actual concurrent-publisher race. Guarded
preparation abandonment holds lifecycle, capability and actual heap guards;
it retains one empty namespace node/account, and the retained account's strong
count proves no implicit account destructor ran. No new record/table/data/root
charge is retained. The fixture adds **two unbudgeted metadata allocations**;
record count admission does not charge these bytes.

Seven node-preparation and six disposal-boundary observations require capability/
heap availability and preserved entry IRQ state. Registration preparation also
checks the other guards listed above. These are serialized boot observations,
not proof that final teardown or every raw captured caller releases its outer
lifecycle/subsystem guards. Selected diagnostic:

```text
[capability namespace storage] first-node/account preparation rejection before ASID publication; heap-held exact namespace publication/detachment; original-charge release outside capability guard; late token cannot alter replacement; guarded abandonment retains one empty node/account (7 allocation, 6 disposal boundaries; entry IRQ preserved)
```

Two new host tests use the production `AdmittedMap` with allocation/deallocation
tracing: prepared insertion, ordering, lookup/mutable lookup, failed/successful
extraction and owning-map abandonment perform zero allocator work in the traced
intervals. Detached payloads survive until explicit post-trace release; abandoned
payloads are not destroyed. Existing retirement-list tests remain unchanged.

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,880 cancellation requests retired |

All three emitted the selected seven/six-boundary diagnostic. Both x86 runs
used SHA-256
`44c58d1e383275c9759cc83eabaf27201eb36308d7a9830bdb7672ab58745662`.
Arm used
`e3f600a8ac768f136b0c29f9069b540522cc5bb88aae9a6b1949ffede51fc195`.
Guests ran sequentially after the last Rust change and each passed on its first
attempt. No guest validation failed for this change; historical intermittent
root-lease timeouts remain separately documented and causally unresolved.

`scripts/run-host-tests.sh` passed, including 29 `IdTable`, eight shared
retirement-list/`AdmittedMap`, six scratch and 33 runtime ownership tests, plus
its existing signing/policy/protocol suites. Strict default-feature kernel Clippy
passed for both custom targets with `--locked -- -D warnings`. Rustfmt, diff
checks, changed documentation links/anchors and table shapes passed; the
eighteen-owner/seven-gate map is unchanged. Runners reused validated service
bundles, rebuilt kernels and enforced assembly permissions (249 x86 native
entries, one Arm). Arm used the authorized runner because its local forwarding
ports are sandbox-blocked.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance capability-namespace-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance capability-namespace-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18729 CATTEN_DEPLOY_HOST_PORT=18029 scripts/run-aarch64.sh --security-test --instance capability-namespace-arm-20261009 --fresh-storage --timeout 180
```

## Limits and remaining work

This removes infallible **namespace** publication allocation and destruction
inside `CAPABILITIES`. Individual authority-record allocation, per-record
removal, reservation/source Drop and source revocation in `publish_batch` still
use `BTreeMap` metadata and retain their existing serialized destruction. Their
conversion must preserve atomic mixed-batch publication, source escrow and the
containing payload owner through retired-record disposal. No unsafe older-caller
allocation path is retained for namespace nodes.

Other root-registration metadata, backend domain/pin registries and containing
outer guards remain separate G1/G4 work. Namespace final disposal may still run
beneath the containing lifecycle/teardown guards; the probes qualify only its
local capability guard boundary. Empty namespace bytes/counts are not covered
by the 4,096/65,536/49,152 authority-record ceilings. Linear namespace lookup,
aggregate admission, principal fairness and essential progress need SEC-07/G7
qualification; fresh QEMU success is not a pressure benchmark.

No new intentional table/data/root retention is added. Table-only fixture totals
remain VT-d **38/29**, AMD-Vi **34/25**, SMMUv3 **40/31** charges/frames;
previously retained roots/data frames and the new empty metadata allocations
remain distinct. There is no new custody, physical retry or supervisor
reconciliation. Historical intermittent user-stack root-lease timeouts remain
causally unresolved; passing runs do not close them or SEC-18.
