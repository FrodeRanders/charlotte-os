# Prepared device registry storage and post-guard disposal

This follows the [DMA close claim correction](2026-10-09-security-dma-close-claim.md)
within C12/C13/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-07 and SEC-18 remain partial: **20 corrected, six partial and four open**
findings, eighteen owner families and seven acceptance gates. No additional
registry, recovery interface or owner-family row is introduced.

## Defect and correction

Device grants published unified capability authority before infallibly allocating
namespace or payload `BTreeMap` storage beneath lifecycle/device serialization.
Successful explicit close could deallocate payload nodes beneath the device
guard. Whole-domain cleanup detached a map, but did not own the namespace node
itself through post-guard disposal. A logical capability reservation did not make
these heap allocations fallible or move their destruction outside serialization.

All MMIO, interrupt and DMA grants now use shared
[`GrantAdmission`](../../../crates/catten/src/device/publication.rs). It captures
the destination once, leases its exact live root and prepares two empty owning
nodes before local lifecycle/device/backend guards or hardware work. Kernel
DMA grants borrow the permanent kernel root. Each user grant requires a real
live root; there is no fabricated-ID compatibility path. Reservation and
publication use the same captured destination, with generation/closing
revalidation under lifecycle serialization. DMA keeps its typed complete-unit,
reset and private/registered rollback obligations beside the common owner.

[`device::registry`](../../../crates/catten/src/device/registry.rs) reuses the
existing [`retirement_list`](../../../crates/catten/src/klib/collections/retirement_list.rs)
owning nodes. Namespace and capability publication only relink prepared nodes.
Ascending capability order preserves the previous cleanup order. An existing
namespace reuses its node; the unused preparation node is explicitly disposed
outside local guards. Reservation publication borrows its owner so rejection
cannot implicitly destroy reservation metadata below those guards.

Explicit close detaches a `RetiredEntry` only at the category's existing
confirmed-completion boundary. Disposal happens after device/lifecycle unlock.
MMIO still retains its reset-visible descriptor through uncertain cleanup; DMA
still retains its original payload and authority during backend destruction.
Whole-domain `PreparedNamespaceDevices` carries the original namespace node and
its capability nodes, consumes completed records one at a time, and releases the
empty namespace node only after confirmed completion. No teardown snapshot or
allocation is needed for these device payload nodes.

Common preparation uses `ManuallyDrop` for inert abandonment. Unused nodes,
reservation and exact root remain retained; the DMA containing owner also keeps
its typed hardware obligations. Detached node/list fallback retains its backing
without allocator, registry, logging or physical cleanup. Ordinary explicit
completion disposes unused storage and reservation, then completes the exact
root. No scalar restoration or failure reinsertion branch is added.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Exact-root admission rejects | No device node preparation, authority publication or hardware work. |
| First or second node preparation rejects | No authority/hardware mutation; only empty preparation storage is destroyed, outside local guards, then the admitted root completes explicitly. |
| Reservation or pre-publication validation rejects | Complete common preparation remains owned through ordinary explicit disposal. DMA must first complete its typed rollback if hardware work started. |
| Authority and payload publication | Same captured destination; admitted authority and prepared payload nodes publish under serialization without device-node allocation/destruction. |
| Explicit close or namespace cleanup rejects | Existing category claim/authority/root and unfinished owning nodes remain according to their physical contract. Retained backing is not a retry receipt. |
| Confirmed cleanup | Exact authority/payload consumption precedes post-guard node disposal and root completion. Namespace completion releases only confirmed records, then its empty node. |
| Abandonment/interruption | Retain the containing preparation or detached unfinished nodes and their existing dependencies; never invoke cleanup from Drop or replay physical release from IDs. |

## Execution evidence

Six serialized injected preparation failures cover first and second nodes for
MMIO, IRQ and DMA. Each requires `ResourceLimit`, zero capability usage, no
device namespace and a still-live root that closes afterward. DMA callbacks
panic if either hardware creation or destruction is reached. Second-stage
injection occurs after an actual first-node allocation and tests explicit empty
storage disposal; this is not an actual exhausted-heap experiment.

Three synthetic publication fixtures hold the **actual heap allocator** while
publishing preadmitted authority and already prepared payload nodes. They cover
fresh namespace creation, existing namespace reuse, all three payload categories
and ordinary explicit close, followed by exact-root close. The synthetic DMA
identifier has no hardware backing. The fixture emits nine allocated-node and
ten disposal-boundary observations. Local lifecycle/device/backend/root-table/
kernel-table/physical/heap guard availability and entry IRQ state are checked at
these allocation/disposal boundaries. These serialized boundary checks do not
prove every outer caller context or cross-LP interruption.

Real QEMU NVMe recovery applies the same boundary checks to actual grant, close,
reassignment and namespace cleanup. Existing reset, hardware maintenance,
physical release, public DMA claim and charged-pin probes remain active. The
real sequence emits 38 allocated-node/25 disposal-boundary observations on
Intel VT-d, 32/22 on AMD-Vi and 40/26 on Arm SMMUv3. These count probe
boundaries, not a byte budget or a census of all live/retained nodes. Selected
diagnostics:

```text
[device registry phases] 9 allocated-node and 10 disposal boundaries outside local lifecycle/device/backend/table/physical/heap guards; entry IRQ state preserved
[device registry ownership] first/second node rejection precedes all MMIO/IRQ/DMA authority/hardware; heap-held namespace creation/reuse and capability publication; explicit post-guard node disposal and exact-root completion passed
[device registry phases] 38 allocated-node and 25 disposal boundaries outside local lifecycle/device/backend/table/physical/heap guards; entry IRQ state preserved
```

Host allocation/deallocation tracing tests ordered insertion, mutable traversal,
failed/middle/tail extraction and relinking, with zero allocations or
deallocations during those operations. Explicit release occurs after tracing.
Detached-node and list abandonment performs no allocation/deallocation or
payload destruction. Existing head-pop/retirement tests also remain green.

Boot device fixtures now retain exact primary/peer roots across asynchronous
IRQ delivery rounds; both roots close after the waiter completes all root
access. x86 IOAPIC/MSI fixtures have separate live roots and close them after
capability cleanup. Their scheduled test threads still execute in the kernel
domain. This replaces the old fabricated namespace IDs rather than weakening
production admission.

Two deterministic validation failures exposed fixture assumptions before final
qualification. The staged-close fixture expected `NamespaceRetired`, but exact
root admission correctly returned `AddressSpaceClosing` before preparation. Its
expectation was corrected. The following attempt reached the old fabricated
namespace fixture and stopped with:

```text
crates/catten/src/self_test/device.rs:106:58
[device] grant_mmio failed: NamespaceRetired
```

Those fixtures were migrated to exact live roots. Neither failure is an
intermittent stack teardown timeout or evidence of physical-release failure.
Historical user-stack root-lease timeouts remain causally unresolved.

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result | Real metadata allocation/disposal boundaries |
| --- | --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` | 38/25 |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` | 32/22 |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,888 cancellation requests retired | 40/26 |

Both x86 runs used SHA-256
`77b9e8da5d1e0adc204a0660335b21d1371ff55446bf784c1e334d888109f9d8`.
Arm used
`4bbf3065721496eb19334cabecd867d75fc5dde35f3eec80f9a331644d373bf8`.
Final guests ran sequentially after the last Rust change and each passed on its
first post-migration attempt. The two earlier deterministic fixture failures
are recorded above; passing final runs do not erase them or resolve historical
intermittent failures.

`scripts/run-host-tests.sh` passed, including 29 `IdTable`, six shared
retirement-list, six scratch and 33 runtime ownership tests, plus its existing
signing/policy/protocol suites. The reusable list source and its tests did not
change after that run; subsequent fixture migration was kernel-only. Strict
default-feature kernel Clippy passed on both custom targets with
`--locked -- -D warnings`. Rustfmt, diff checks, changed documentation links/
anchors and table shapes passed. The eighteen-owner/seven-gate map is unchanged.
Runners reused validated service bundles, rebuilt kernels and enforced assembly
permissions (249 x86 native entries, one Arm). Arm used the authorized runner
because local forwarding ports are sandbox-blocked.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance device-registry-intel-final-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance device-registry-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18719 CATTEN_DEPLOY_HOST_PORT=18019 scripts/run-aarch64.sh --security-test --instance device-registry-arm-20261009 --fresh-storage --timeout 180
```

## Limits and remaining work

Lookup and sorted insertion are now linear scans, replacing logarithmic
`BTreeMap` lookup. Existing published capability ceilings remain 4,096 per
namespace, 65,536 node-wide and 49,152 ordinary. An empty namespace node follows
the root lifetime, not its current capability count. Fallible allocation is not
byte/principal admission, pressure fairness or an essential-progress proof.
Those SEC-07 requirements remain open.

Unified capability-table reservation/removal still has separate `BTreeMap`
metadata allocation/destruction under its serialization and lifecycle, sometimes
while the device registry is held. Backend domain/pin registry storage also
remains separate. This change qualifies **device payload/namespace storage**;
it does not make all work below the device guard allocation-free or close G4.

No new intentional table/data/root retention fixture is added. Existing
abandoned grant owners now also retain their two empty preparation nodes.
Table-only retained totals remain VT-d **38/29**, AMD-Vi **34/25**, SMMUv3
**40/31** charges/frames; independently charged retained roots/data frames and
unbudgeted retained heap metadata must not be folded into those totals.

Wider syscall/fixture masks, implicit destruction in other families, actual
outstanding I/O, maintenance timeout, partial physical release, cross-LP races,
pressure/progress and physical-device quiescence remain G1/G7 work. There is no
new administrative custody, retry or supervisor reconciliation. Passing guests
do not resolve the historical intermittent root-lease timeouts or close SEC-18.
