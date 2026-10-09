# Private DMA constructor rejection retains its complete grant

This extends the [published-creation correction](2026-10-09-security-dma-creation-rollback.md)
within C18/C14/C15 and G1/G3/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger stays **20 corrected, six partial and four
open**. No new owner family, registry or recovery controller is introduced.

## Defect and correction

Domain constructors previously used `Tables::prepare_unpublished` or cancelled
their table owner directly when root, CD, MSI-walk or VT-d context-table
preparation rejected. Those constructors ran beneath backend serialization and,
for capability grants, the enclosing lifecycle/device serialization. Known-private
backing could therefore enter physical release, pool refund and ledger/domain
metadata destruction while those guards were held. Returning an error did not
carry that private rollback obligation in the enclosing grant.

Each constructor now creates its complete private domain before allocation and
returns an error together with an owning `DetachedDomain<Domain>`. The payload
includes its initialized prefix, table ledger, descriptor fields, sparse metadata,
mapping/quarantine collections and original table charges. No constructor error
cancels backing or destroys that metadata. VT-d also retains the complete private
domain if subsequent unit context-table allocation rejects.

`DmaCreation` retains the typed `PrivateDomain` enum in `PreparedDmaDomain`,
beside the captured user-root operation and original capability reservation.
Its registered ID and private payload are mutually exclusive. An armed private
owner rejects reuse before reset, allocation or hardware initialization. Successful
construction follows the existing registry admission and published-creation
contract; private failure publishes no domain descriptor or capability.

The grant's one-shot rejection path cancels the private payload only after its
local backend, lifecycle, device and config guards leave. `Tables::cancel_private`
checks never-published state, freezes before physical release, then refunds only
confirmed backing and disposes its ledger. Successful private cancellation next
disposes the remaining typed domain metadata, then refunds authority and completes
the root operation. No hardware maintenance receipt is needed for backing that
never entered a hardware descriptor.

The existing `DetachedDomain` retention machinery keeps every implicit field
behind `ManuallyDrop`. Failed physical release returns the whole frozen private
payload to the containing grant; the original root operation, authority and
whole table charge remain. Abandonment performs no registry, allocator, callback,
logger or physical work. No standalone private-table registry or scalar recovery
handle is created.

## Phase classification

| Phase | Owned result |
| --- | --- |
| Admission rejected before physical allocation | Only unused reservation admission refunds; any prepared ledger storage still belongs to the complete private payload. |
| Constructor rejects a physical prefix or complete private preparation | Typed private payload returns into the original grant; no hardware authority or application capability is published. |
| Confirmed private cancellation | Post-guard physical release precedes metadata disposal, authority refund and root completion. |
| Physical rejection/interruption | Entire private payload/root/reservation and original table charge retain; the table walk and grant are terminal, never retried. |
| Abandonment | Complete typed grant retains its fields and dependencies without destructor cleanup. |
| Error after hardware publication | Existing registered-domain maintenance/drain owner applies; private cancellation cannot qualify published tables. |

## Execution evidence

The serialized real QEMU NVMe fixture rejects immediately after each allocated
private prefix, and once after complete preparation before registry/descriptor
publication. The allocation remains in the exact private ledger when rejection
fires, including a root whose constructor field has not yet received its address.
VT-d exercises four root/MSI pages, AMD-Vi one root, and Arm five root/CD/MSI
pages. The complete Arm payload includes its sparse MSI metadata.

Before physical release and before domain metadata disposal, probes require
backend, lifecycle, device, root-table, physical-allocator, heap and PCI-config
availability with disabled bus mastering. Global availability probes retain their
one-second bound and entry IRQ policy from the
[executor/fixture follow-up](2026-10-09-security-abort-executor.md).
Actual private release then restores the exact table/authority counts and allows
close of the rejected grant's root. No I/O command is submitted, no acknowledgement
is suppressed, and no published domain is relabelled private.

Separate complete-grant fixtures contain actual private tables and heap-backed
metadata. They cover:

- Ordinary cancellation restores the exact table-account baseline, refunds the
  original capability reservation and closes the root. A registered-domain
  destruction adapter must never run for this private payload.
- Armed private creation rejects before its reset callback. Dropping the entire
  grant while backend/lifecycle/device/table/capability/heap/physical guards are
  held performs no physical or metadata destruction. Its exact root stays leased
  and authority remains charged; a successor cannot reuse/refund that root.
- An injected second-frame physical rejection returns the same complete grant
  after exactly one frame was released. The whole original table charge remains.
  A second grant cancellation rejects before its adapter, and the frozen table
  owner independently rejects before any returned address is visited. Guarded
  abandonment retains the failed payload, exact root and authority too.

This adds the following intentional guest-lifetime retention, separate from live
hardware backing and independently charged root backing:

| Backend | Original domain-table charges retained | Actual domain-table frames retained | Exact root operations / authority reservations retained |
| --- | --- | --- | --- |
| VT-d | 8 | 7 | 2 / 2 |
| AMD-Vi | 4 | 3 | 2 / 2 |
| SMMUv3 | 10 | 9 | 2 / 2 |

Earlier table fixtures retain 22 charges and 15 frames. Their unchanged retention
plus these private-grant probes totals 30/22 on VT-d, 26/18 on AMD-Vi and 32/24 on
SMMUv3. These are deliberate injected terminal owners, not newly unaccounted
leaks or recovery candidates. Ordinary rejected constructors refund exactly;
physical corruption, panic unwinding and outstanding-I/O recovery are not injected.

Selected results summarize only this boundary:

```text
[DMA private rollback] all N allocated prefixes plus complete preparation rejected; physical/metadata cleanup outside backend/lifecycle/device/config guards, exact charge/capability refund and root close passed
[DMA private retention] complete grant Drop under guards, armed rejection and partial release with no retry passed; original domain charges, remaining frames, both exact roots and authority reservations retained
```

Here `N` is four, one and five respectively; the retention table preserves the
actual charge/frame counts rather than repeating routine boot output.

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; probe `0xffff`, publication generations 1/2, 4,848 cancellation requests retired |

Both x86 runs used kernel SHA-256
`9351daf37884e5aa45145c6f257e55cf9b7cff9dcb6ccc9c5df80428922d787e`.
Arm used
`7377418789423688cb49b59024e9d6e4d4b70963cf04a12a1ee1876d1175cf4a`.
All three emitted both selected results with their exact prefix/retention counts.
No guest validation failed in this follow-up. Prior intermittent episodes remain
preserved in their original reports and the linked executor follow-up.

Strict default-feature kernel Clippy passed on both custom targets with
`--locked -- -D warnings`. Rustfmt, whitespace, local documentation links/tables
and the unchanged eighteen-family/seven-gate map passed. Runners reused validated
service bundles, rebuilt kernels and enforced assembly permissions (249 x86
entries and one Arm). Runs were sequential after the final Clippy checks; Arm
had permission to bind its local forwarding ports. The host harness was not
rerun for these kernel-only changes.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance dma-private-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance dma-private-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18649 CATTEN_DEPLOY_HOST_PORT=17949 scripts/run-aarch64.sh --security-test --instance dma-private-arm-20261009 --fresh-storage --timeout 180
```

## Remaining scope

Only the grant's local serialization is separated here; unrelated enclosing
masks/guards and general call chains still require G1 qualification. Allocation,
ledger/registry preparation and unused-reservation refund remain beneath existing
serialization. Unit-private initialization still uses its separate helper and
can cancel under its backend guard. Initial creation/reset, map/unmap and unit
initialization waits, reset fallback, registry-node destruction and aggregate
metadata admission remain G3/G4 work. The actual VT-d context-table allocation
rejection branch uses the new owner but is source-qualified rather than separately
forced in the real fixture, which already has its bus context table cached.

Failed private cancellation has no retry/custody controller. Published hardware
maintenance retains its distinct engine/domain completion proof. Physical-device
reset, concurrent outstanding I/O and whole-node exhaustion remain outside these
QEMU probes. This does not close SEC-18 or establish the causes of the historical
retirement timeouts and other preserved intermittent observations.
