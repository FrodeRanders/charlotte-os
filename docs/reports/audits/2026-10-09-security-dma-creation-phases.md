# Complete-unit DMA creation outside local serialization

This continues the [PCI reset claim correction](2026-10-09-security-pci-reset-claim.md)
within C14/C15/C18 and G1/G2/G3/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger remains **20 corrected, six partial and four
open**, with eighteen owner families and seven acceptance gates. No registry,
recovery interface or finding closure is added.

## Defect and correction

DMA grants previously retained lifecycle and backend serialization through
controller reset, domain construction and initial hardware configuration.
Releasing those guards required retaining the actual unit state and containing
grant, rather than copying queue cursors or storing a numeric requester.

[`domain_creation::Preparing`](../../../crates/catten/src/device/domain_creation.rs)
now moves the complete installed unit into a retaining owner and leaves
its existing typed slot `Claimed`. The owner contains the actual command engine,
unit/domain table owners, source/domain registries, mapping/pin storage and all
other unit fields. It exclusively borrows the original grant's `DmaCreation`.
Ordinary backend lookup and initialization reject the claimed slot before any
reset or mutation. No collection snapshot or separate source registry is used.

Admission requires an available command engine and no detached domain cells.
This second check matters after successful destruction maintenance: its engine
has returned, but physical release still owns an empty domain cell and needs the
registered state to finalize. Creation rejects before extraction, so it cannot
strand that already-started physical transaction. Frozen domains remain fenced
by their existing source/retirement state.

Reset, constructor allocation, registry preparation and initial configuration
run with the complete unit owned outside its mutex. Ordinary success/error
restores the same unit, including actual queue/epoch and registry changes, before
the grant handles private or hardware-published rollback. Restoration does not
replay hardware or reconstruct state. Abandonment retains every field and leaves
the original unit slot permanently claimed; no implicit cleanup or forced reset.

The original grant leases its exact root before lifecycle admission and reserves
authority before hardware creation. Lifecycle leaves through creation, then
rechecks the captured root generation and closing state before capability
publication. A staged close can remain Pending while the older lease completes;
it must prevent publication and activation. Confirmed post-guard rollback
refunds the original reservation and finishes that lease, allowing the same
closing owner to progress. Publication retains the existing endpoint claim and
busy-cap activation contract.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Absent engine or detached domain cell | Reject before extraction/reset/allocation; the older complete maintenance/physical owner remains untouched. |
| Claimed installed unit | Ordinary lookup and initialization reject. Reset/construction/configuration retain the complete unit and original grant outside local guards. |
| Ordinary success or construction/configuration error | Restore the exact unit to its original slot. The containing grant still owns any reset/private/published rollback obligation. |
| Captured root starts closing before publication | Reject publication before activation; explicitly confirm backend/reset rollback, refund authority and complete the older lease. The original staged closing owner remains Pending until completion. |
| Uncertain rollback/reset | Existing complete grant/fences retain all dependencies; an error does not authorize replay or refund. |
| Complete-unit/grant abandonment | Retain actual engine, tables, metadata, pins, root, authority and claims; no physical work, lock acquisition or logging from fallback. No retry owner is returned. |

## Execution evidence

Real QEMU probes qualify the complete-unit claim, before domain construction,
before initial configuration and before exact restoration. Each requires bounded
backend/lifecycle/device/CPU-table/physical/heap guard availability, preserved
entry IRQ state, exact-root busy close and nested create/map/unmap/destroy/reset
exclusion. These are boundary observations, not allocator-lock probes inside
every frame allocation. Actual NVMe reset wait probes separately require those
local guards to be available, bus mastering disabled, config/MSI rejection and
captured BAR/ECAM exclusion.

The real post-drain destruction probe rejects new creation before its reset
callback while the detached domain still owns physical finalization. Another
fixture stages root close after real reset/configuration: it remains Pending,
rejects publication before activation, confirms post-guard table/authority refund
and completes that same closing owner. Existing private-prefix, configuration
rejection, map/unmap, real retirement, reassignment and storage verification also
run on the same unit.

Synthetic success/error restoration preserves actual command-vector changes.
Absent engines and empty domain cells reject before work. Complete-unit/grant
abandonment under lifecycle, compiled backends, device, root/kernel table,
capability, physical, heap, table-pool and original slot guards retains metadata,
one unit table and one domain table, two independently charged data frames,
an exact user root and its original authority reservation. Ordinary root close
and data close remain rejected. The retained payload is synthetic; no real
installed hardware unit is abandoned.

Table-only intentional totals increase by **two charges/two frames** to VT-d
**38/29**, AMD-Vi **34/25**, SMMUv3 **40/31** charges/frames. User roots and DMA data
frames have independent accounts. No reservation is reconstructed for cleanup.

## Validation results

Selected diagnostics omit routine boot output:

```text
[DMA creation phases] complete installed-unit claim, allocation, initial configuration and exact restoration outside backend/lifecycle/device/table/allocator guards; preserved IRQ policy, exact-root busy close and nested mutation/reset exclusion passed
[DMA creation closing] real configuration with retained root/reservation; staged close stayed Pending, publication rejected before activation, confirmed post-guard rollback refunded authority and original closing owner completed
[PCI reset claim] W real reset wait boundaries: local config/device/lifecycle/backend/table/allocator guards available, bus mastering disabled, ordinary config/MSI lookup rejected and captured BAR ranges fenced; IRQ policy preserved
```

| QEMU target, four LPs and fresh isolated storage | Authoritative result | Creation probe episodes / published grants / reset wait boundaries |
| --- | --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` | 2 / 3 / 11 |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` | 2 / 3 / 8 |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,860 cancellation requests retired | 2 / 3 / 12 |

Each creation episode checks all four named boundaries exactly once. Both final
x86 runs used SHA-256
`e6065ae57935467a5d85e178a7032fc864ecfd53820f1edd363faaf959698859`.
Arm used
`3e41666f5c419964f7f9ed99ec333ca8e233e96720a03da70c8026f9d1962355`.
All final guests emitted the ownership, staged-close and reset diagnostics.

The initial Intel attempt stopped before real creation at a new synthetic fixture
assertion: table counters were `(31, 17)`, while the assertion expected `(30, 17)`.
The counter pair means total/domain charges, not unit/domain charges. The fixture
retains one unit plus one domain table, so the correct increase is `(2, 1)`.
That assertion was corrected before the final runs. This was a deterministic
fixture accounting error, not a new intermittent guest-retirement failure.
Historical root-lease timeout episodes remain separately documented and unresolved.

Strict default-feature kernel Clippy passed for both custom targets with
`--locked -- -D warnings`; rustfmt and diff checks passed. Documentation links,
anchors and table shapes preserve the eighteen-family/seven-gate map. Runners
reused validated service bundles, rebuilt kernels and checked native assembly
permissions (249 x86 entries, one Arm). Final guests ran sequentially after the
last Rust change. The host harness was not rerun for kernel-only changes. Arm
used the authorized runner because its local forwarding ports are sandbox-blocked.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance dma-create-intel-fixed-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance dma-create-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18699 CATTEN_DEPLOY_HOST_PORT=17999 scripts/run-aarch64.sh --security-test --instance dma-create-arm-20261009 --fresh-storage --timeout 180
```

## Limits and remaining work

This qualifies local creation/reset/construction/configuration phase separation.
It does not enable IRQs or remove unrelated enclosing masks/guards. Moving the
complete unit does not itself lease every peer root referenced by older mappings;
those retain their own cleanup contracts. Interrupt fault handlers use their
existing independent published state; retaining unit backing prevents reuse.

General metadata allocation admission, implicit inner construction-field fallback
and the wider call-chain/IRQ inventory remain G1/G4 work. Serialized nested
probes do not establish cross-LP races, combined pressure, outstanding I/O,
actual reset/completion timeouts, activation readback failure or physical-device
quiescence. No unit shutdown, recovery custody, authorized operator policy or
supervisor reconciliation is implemented. Historical intermittent user-stack
root-lease timeouts remain causally unresolved; a passing run cannot resolve them.
