# Admitted DMA mapping records and SMMU walker cache

This continues the [backend registry-storage correction](2026-10-09-security-backend-registry-storage.md)
within C14/C15/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md). SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. No new registry or custody
interface is introduced.

## Defect and correction

Domain creation already admitted its registry nodes, but all three mapping paths
still inserted into an infallible `BTreeMap` after installing data leaves. SMMU
also inserted leaf-table cache metadata after branch links. Unmap removed and
destroyed its map node before hardware completion, keeping the pin separately
in a pre-reserved quarantine vector on failure. This split metadata ownership
from the maintenance obligation and left infallible allocation after publication.

Shared [`mapping_storage::Records`](../../../crates/catten/src/device/mapping_storage.rs)
now uses the existing admitted-map/retirement-list machinery for live and
quarantined records. `PendingPin` owns the exact pin and unused or detached node
inside its retaining fields. Fallible node preparation precedes data leaves;
successful publication only relinks. Unmap carries the original record, including
pin/pages/storage, through the existing typed `MappingMaintenance`. Rejected
completion relinks that same node into quarantine. Failed prefix cleanup consumes
its original pre-leaf node into quarantine. No growing pin vector, replacement
node or teardown snapshot remains. Duplicate checks include quarantined records.
Failed-prefix leaves have already been cleared; their quarantine record retains
the whole pin until domain destruction, without publishing a live IOVA or
authorizing range-based cleanup.

Ordinary unstarted failure or confirmed maintenance restores exact backend state
before explicit unpin/node disposal outside local backend guards. Domain
retirement keeps its cell/source claim through table release and every record's
pin/storage completion. Node abandonment retains its original pin/metadata,
while the existing containing maintenance/public operation retains domain,
actual command engine, root and capability claim. This adds no erased completion
proof or generic physical retry.

SMMU `WalkerCache` admits metadata before a cache miss allocates or links a new
branch. Completed cache publication only relinks the node, before its data leaf.
A partial physical walk keeps unused storage in that same domain for a later
cache miss; installed prefixes remain linked and charged. This is ordinary walk
continuation, not retirement retry. The cache's map and unused preparation have
inert field fallback. Explicit private cancellation and domain retirement now
dispose cached/unused metadata only after the table owner reaches **Released**;
the adapter asserts that state. Private-release rejection still returns the
complete frozen grant and never disposes its metadata or retries freed tables.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Mapping node rejects | No data leaves; exact pending pin is released after backend restoration. No mapping publishes. |
| Walker node rejects | No new branch allocation/link for that miss; unstarted pin/node cleanup follows restoration. |
| Partial sparse walk rejects | Linked tables stay charged; unused cache node stays domain-owned. Installed data prefix is cleared and requires existing typed rollback maintenance. |
| Prefix maintenance rejects | Original admitted node and pin relink into quarantine; duplicate/premature close reject. |
| Unmap maintenance rejects | Original detached record relinks into quarantine without allocation or destruction. No pin is released. |
| Confirmed map/unmap | Exact state restoration precedes post-guard pending-pin/metadata completion. Live mappings retain their original node/pin. |
| Table physical release rejects | Existing frozen complete owner, cache, records, original charges and cell/source claim remain. No metadata disposal or partial-release retry. |
| Confirmed table cancellation/retirement | Explicit record/pin/cache disposal outside backend guards, before domain-node/root completion. |
| Owner abandonment | Complete existing transaction and all unfinished metadata remain retained; no claim clearing, allocator/hardware work or custody ticket. |

## Execution evidence

The [metadata adapters](../../../crates/catten/src/device/mapping_storage/tests.rs)
reject mapping-node preparation and explicitly release its pin. Publication,
record detach and quarantine relink run while the **actual heap is held**. A
failed-unmap adapter checks the original record's value address survives relink;
duplicate mapping and early memory close reject. Confirmed private completion
then releases the pin/node. A failed-prefix adapter separately consumes its
pre-leaf node under the heap hold. Arm also rejects cache-node preparation,
publishes cache metadata under that hold and reuses one unused preparation.
These cache entries are explicitly metadata fixtures, with no physical adoption
or hardware link.

The existing private SMMU sparse walker now rejects cache metadata before any
new table charge, then covers cached reuse, partial physical prefixes, repeated
ceiling rejection and successful continuation. Confirmed private cancellation
explicitly disposes cache storage. These walkers borrow their data frame and are
never hardware-published.

The [real NVMe fixture](../../../crates/catten/src/device/recovery_tests.rs)
rejects a mapping node on every target and a fresh walker node on Arm. It requires
unchanged table charges and successful close of the exact rejected memory cap,
then uses the same domain for successful map/unmap. Existing failed-unmap and
sparse-prefix completion rejection retain pins until actual acknowledged domain
retirement. Existing creation/private rollback, reset exclusion/reassignment,
public busy-close and staged-root completion checks still pass. Injection
returns errors; it does not manufacture hardware acknowledgements or suppress
real device completion.

Selected diagnostics:

```text
[DMA metadata ownership] rejected admission releases pin; heap-held publish/detach/quarantine keeps exact node; duplicate and early close reject; confirmed private completion releases original storage
[DMA metadata rejection] mapping/walker admission rejected before new table/data leaves; exact pin cleanup and unchanged table charges; same domain remains usable
```

Allocation/disposal **entry** probes reuse the existing local backend/lifecycle/
device/capability, CPU-table and physical/heap availability checks, with preserved
entry IRQ state. The allocator itself is entered afterward. Counts below are
selected phase visits, including rejected admission, not whole-boot allocation
balances. Source inspection establishes leaf/link ordering; these serialized
probes do not qualify every enclosing caller or cross-LP interruption.

| Target | Registered tests | Metadata adapter preparation/disposal entries | Real NVMe metadata preparation/disposal entries |
| --- | ---: | ---: | ---: |
| Intel VT-d | 15 passed, zero failed/pending | 3 / 2 | 6 / 5 |
| AMD-Vi | 15 passed, zero failed/pending | 3 / 2 | 6 / 5 |
| Arm SMMUv3 | 19 passed, zero failed/pending | 6 / 4 | 16 / 14 |

The existing [guarded mapping-owner fixture](../../../crates/catten/src/device/mapping/tests.rs)
now retains one unused mapping node beside its pending pin. Arm also retains one
cached and one unused walker node inside that same complete synthetic domain.
Dropping maintenance and public operation under backend/lifecycle/device/CPU-
table/physical/heap/pool guards preserves root/capability fencing and original
charges/free-frame counts. This extends the existing one-table/two-data-frame/
exact-root fixture; **no additional root, authority, table charge or data-frame
retention** is introduced. Table-only intentional totals remain VT-d **38
charges/29 frames**, AMD-Vi **34/25**, SMMUv3 **40/31**. Earlier independently
retained CPU-root/authority metadata is unchanged.

Fresh-storage guests run sequentially, reusing validated embedded service bundles
because only kernel code changed:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance dma-metadata-intel-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance dma-metadata-amd-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18769 CATTEN_DEPLOY_HOST_PORT=18069 scripts/run-aarch64.sh --security-test --instance dma-metadata-arm-20261010 --fresh-storage --timeout 180
```

Final x86 passed bitmap is `0x1bbff`. Arm is `0x2003ffff`, with scoped security checks `0xffff` at publication
generations 1 and 2.
Kernel SHA-256:

- Intel/AMD x86: `c3c309f54bf213195daeb380c6f63d752e118a664d7899c338664e097defd4eb`.
- Arm security guest: `8ff32c8603c1f71262c7151d2dcdd0fe77ac87df35cc552296139036fb2b2ce6`.

Strict locked default-feature Clippy passes on both architectures (`-D warnings`),
as do formatting and diff checks. Runners verify native assembly section/load
permissions (249 x86 entries, one Arm entry). Shared admitted-map source/host
tests are unchanged; the preceding eight-test/full-host-harness evidence remains
in the [namespace report](2026-10-09-security-capability-namespace-storage.md).
That harness is not rerun in this kernel-only pass; new ownership adapters execute
in all three guests.

## Remaining boundary

This supplies fallible mapping/cache storage and original-node lifetime, without
general heap-byte/principal budgets or essential-service progress guarantees.
Lookup, duplicate detection and sorted publication use linear scans. Table
ledgers, other authority callers and active-token fallback, broader metadata and
all enclosing contexts still need qualification. Actual OOM/concurrent pressure,
outstanding device I/O, interrupted hardware publication, abandoned-owner custody
and physical-platform recovery remain open. Earlier intermittent user-stack lease
timeouts remain causally unresolved; passing guests do not explain them or close
C16/G1/G2/G7. The [current ledger](../../reference/security-remediation.md) retains
their separate failed-run/repeat evidence and the original SEC-07/18 criteria.
