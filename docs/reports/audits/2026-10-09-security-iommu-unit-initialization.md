# Claimed IOMMU boot initialization outside backend serialization

This continues the [private-domain rollback correction](2026-10-09-security-dma-private-rollback.md)
within C14/C15/C18 and G1/G3/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger remains **20 corrected, six partial and four
open**. The eighteen-family/seven-gate map is unchanged.

## Defect and correction

All three backends lazily initialized their unit while holding its registry
mutex. A first DMA request could consequently allocate unit tables/queues,
wait for hardware and cancel a failed private prefix beneath the caller's
lifecycle/device serialization. A failure after hardware publication retained
table backing through its destructor, but left the empty global slot eligible
for another initialization. Retained backing alone did not fence that replay.

The existing typed backend slot now has `Vacant`, `Claimed` and `Installed`
states. Boot claims the slot under a short hold, leaves it, then prepares the
complete typed unit behind `DetachedDomain` before the first table allocation.
The owner includes the unit table ledger, original charges, command state,
descriptor addresses and all registry fields. A generic `Claim` retains this
payload through explicit cancellation or installation. No registry, spill
allocation, scalar recovery handle or new owner-family row is introduced.

Ordinary backend lookup never initializes hardware. Vacant lookup returns
`Unsupported`; claimed lookup returns `OperationInFlight`. An installed unit
whose command engine is detached still rejects early initialization, preserving
the existing maintenance fence. Only the boot entry in `main.rs` prepares a
vacant production unit; the recovery fixture's later `initialize_early` calls
inspect an already-installed engine. There is no unsafe lazy fallback for boot
modes which deliberately skip early initialization.

Allocation, preparation, hardware control/publication, waits and ordinary
private rollback now occur outside the local backend guard. Successful
installation requires published table state and consumes the exact complete
owner under the same slot hold. Repeat initialization of an installed, available
unit returns success without running preparation again.

## Phase classification

| Phase | Result and retained dependencies |
| --- | --- |
| Unsupported/configuration rejection before owned backing or hardware control | Explicitly release the unstarted slot claim. Existing kernel MMIO mappings are permanent platform mappings, not adopted unit backing. |
| Allocation/private preparation rejects before hardware control starts | Carry the complete typed prefix into unlocked private cancellation. Confirm physical release, dispose ledger/unit metadata, then clear the claim. |
| Private cancellation physically rejects or is interrupted | Preserve the whole charge, payload and slot fence. Physical release freezes before its first allocator call; no partial-release retry. |
| Hardware control starts, including disable before a new base is published | Any error retains the complete unit and claim. Private table state alone cannot authorize hardware initialization replay. |
| Hardware base/queue publication or later command wait rejects | Preserve all backing, command state and the slot fence. No automatic detach, shutdown, reconstruction or retry. |
| Abandonment, even before allocation | Keep the claim. `DetachedDomain` prevents implicit field destruction, allocator entry, callbacks, logging or physical work. |
| Confirmed successful initialization | Install the complete unit once; backing remains charged for its existing kernel lifetime. |

The hardware-control boundary precedes Intel `GCMD` disable, Arm `CR0` disable
and AMD base publication. Original register ordering and hardware acknowledgement
requirements are preserved. No timeout is converted into success. In particular,
Intel/Arm disable uncertainty is terminal here even though the new root/stream
table never became reachable. This is conservative retention, not retry custody.

## Execution evidence

Before normal boot installation, the selected real backend rejects immediately
after each private **allocation region**, and once after complete private
preparation. Rejected backing stays in its exact ledger, including an address
not yet assigned to its constructor field. These failures occur before any
hardware control/base write. Contiguous device/stream tables are one region;
the fixture does not inject a separate failure at every physical page.

| Backend | Allocation regions exercised | Complete unit backing | Real successful initialization waits checked |
| --- | --- | --- | --- |
| Intel VT-d | Root: one | One page initially; later context tables retain ordinary unit ownership | Three: disable, root-pointer acknowledgement, translation enable |
| AMD-Vi | Device table, command queue, event log, completion cell: four | 515 pages | Zero: this initializer has no hardware acknowledgement loop |
| Arm SMMUv3 | Stream table, command queue, event queue: three | 1,026 pages on QEMU's 16-bit StreamID implementation | Five: disable, command enable, real command SYNC, full enable, IRQ control |

Probes before preparation, physical cancellation, metadata disposal, real waits,
publication and installation require backend/lifecycle/device/CPU-table/heap/
physical-allocator availability and unchanged entry IRQ policy. The boot fixture
runs before AP schedulers are released. Each ordinary private failure restores
the exact unit/domain table-charge baseline and clears only its own claim.
Normal hardware initialization then installs once, and repeated admission never
reruns its preparation callback. The full QEMU suite subsequently exercises DMA
creation, mapping, retirement and reset against this actual installed unit.

Separate synthetic owners contain actual unit table backing, heap-backed
ledgers and destructor-bearing metadata. They check nested initialization rejection while claimed, unstarted
rollback, complete-payload and empty-claim abandonment under all original global
guards plus their own slot lock, partial physical rejection, and uncertain
control/published rejection. The second-frame release fails after exactly one
frame is returned; later initialization rejects before any preparation callback.
Metadata destructors do not run for any retained owner. The published fixture
only marks synthetic state; it does not fake a hardware timeout or completion.

These probes deliberately retain **five original unit charges and four actual
unit frames** per guest, with zero additional domain-table charges. They are
separate from live hardware units and earlier domain/root fixtures. Added to
the previous table-only totals, intentional retention is 35 charges/26 frames
on VT-d, 31/22 on AMD-Vi and 37/28 on SMMUv3. Independently charged user roots
remain outside these table-only totals. Empty abandonment also retains its slot
fence despite having no backing to charge.

Selected boundary markers, omitting routine boot output:

```text
[IOMMU unit initialization] N allocated region prefixes plus complete private preparation refunded; W real waits and publication outside backend/lifecycle/device/table guards; installed once, caller IRQ policy preserved
[IOMMU unit retention] empty/private abandonment under guards, partial release without retry and uncertain control/published rejection remain fenced; 5 original unit charges/4 frames and complete metadata retained
```

Here `N/W` is 1/3, 4/0 and 3/5 respectively.

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,912 cancellation requests retired |

Both x86 runs used kernel SHA-256
`8b9277068125a20ac7cad5f5984b7afe116a0d3fe7ff6603637558a1ebb0503a`.
Arm used
`58e84cac2d558ab67f8ed68a959bfb5da067a308de38dc8639a4e4b6d8fd0128`.
All three emitted the selected initialization/retention markers with their exact
region/wait counts. No guest validation failed. An initial Arm launcher attempt
was prevented from starting QEMU by sandbox denial of its local forwarding port;
port-authorized execution passed. That was a host launch failure, not a guest
security result. Earlier intermittent guest failures remain in their original
reports; passing this matrix does not establish their cause.

Strict default-feature kernel Clippy passed on both custom targets with
`--locked -- -D warnings`. Rustfmt, whitespace, local documentation links/anchors/
tables and the eighteen-family/seven-gate map passed. Runners reused validated
service bundles, rebuilt kernels and enforced assembly permissions (249 x86
entries and one Arm). Final guest runs were sequential after the last source
change and strict Clippy checks. The host harness was not rerun for kernel-only
changes.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance unit-init-intel-final-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance unit-init-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18659 CATTEN_DEPLOY_HOST_PORT=17959 scripts/run-aarch64.sh --security-test --instance unit-init-arm-20261009 --fresh-storage --timeout 180
```

## Remaining scope

The boot entry preserves its existing outer IRQ mask; no helper enables IRQs
under an unknown caller. Hardware waits are bounded existing polling loops,
not a latency/progress guarantee. The boot probes are serialized admission
evidence, not concurrent multi-LP initializer stress or outstanding-I/O recovery.
No real hardware timeout, panic unwinding, allocator corruption or physical-device
reset is injected. Production unit shutdown/recovery and custody remain absent.

Initial domain creation/reset and map/unmap waits still hold their existing
serialization. Domain allocation/ledger/registry preparation, reset fallback,
registry-node destruction, general metadata admission and full outer-context
qualification remain G1/G3/G4/G7 work. Boot MMIO mapping failure/rollback is its
existing platform mapping boundary; these unit tests do not qualify it.
No historical intermittent failure's cause is established by this change.
