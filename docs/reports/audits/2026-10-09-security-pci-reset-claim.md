# PCI reset claim retained through DMA publication

This continues the [DMA mapping correction](2026-10-09-security-dma-mapping-maintenance.md)
within C13/C14/C18 and G1/G2/G3/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger remains **20 corrected, six partial and four
open**. No owner-family row, administrative retry interface or registry is added.

## Defect and correction

The QEMU NVMe `ResetSource` previously retained a PCI config mutex guard through
controller polling and new-domain configuration. Its destructor wrote the PCI
command register to disable bus mastering. That implicit hardware work inherited
unknown caller locks/IRQ state. Reset failure also destroyed this local owner
before the containing grant knew that hardware state was uncertain.

Reset now installs a logical claim in the exact endpoint's existing config cell.
The claim captures validated BAR extents and the endpoint's physical ECAM page;
range overlap is an exclusion check, never authority to reset or retry. Admission
uses device serialization and rejects mapped, foreign or in-flight overlapping
MMIO authority. Short config holds capture BARs and command state; range admission
runs after that hold leaves, followed by exact command and all-six-BAR revalidation before claiming.
This prevents recursive config-lock acquisition during range checks.

All ordinary PCI config/MSI discovery helpers reject a claimed cell. Overlapping
MMIO grant, map/unmap, explicit close and namespace device preparation reject
before mutation. Claims remain visible even with no MMIO capability record, and
also exclude ECAM aliases. A conservative publication hint avoids forcing lazy
topology construction under early grant locks; it is not a claim registry and
never authorizes access. Synthetic fixtures initialize the global topology
outside guards before exercising the hint; the HVF compatibility profile skips
this ECAM-dependent fixture.

`DmaCreation` owns `ResetSource` beside its private/registered backend obligation
inside the original `PreparedDmaDomain`. It records the claim before reset writes.
Reset marks hardware uncertainty before changing command/BAR/controller state.
Temporary BAR probes restore original values before fallible checks. Config and
device guards leave before `CC.EN`/`CSTS.RDY` polling; original lifecycle/backend
serialization still remains. No helper enables IRQs under an unknown caller.

Only confirmed reset plus backend configuration and capability publication permit
explicit consuming activation. The new capability is published with its existing
in-flight bit set. Completion validates the exact cap/domain claim under device
serialization, enables and verifies memory decode/bus mastering, releases the
endpoint claim and clears the capability's bit. Ordinary root/reservation
completion follows after unlock. Activation uncertainty retains the endpoint,
published busy capability, backend obligation, root and original reservation.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Model/topology/BAR/range admission rejects before claim | No hardware mutation or reset owner; ordinary grant rejection refunds its unused reservation/root. |
| Claimed but reset not started | Explicit consuming cancellation confirms disabled bus mastering before clearing the endpoint claim. Abandonment retains it; it does not fabricate a hardware reset acknowledgement. |
| Reset writes/polling uncertain | Cancellation/activation reject. The complete grant retains the exact endpoint claim, root, reservation and any backend obligation; no replay or destructor hardware work. |
| Confirmed reset, private constructor or published configuration rejects | Keep the endpoint claim and disabled bus mastering through existing post-guard backend rollback. Only confirmed backing cleanup permits explicit reset cancellation and grant refund/root completion. |
| Capability publication/configuration succeeds | Exact busy-cap validation precedes consuming activation; only confirmed activation releases endpoint/capability claims. |
| Activation/cancellation command verification rejects | Retain the claim and containing grant. No forced clearing, refund or retry. |
| Complete grant abandoned under guards | No config write, lock acquisition, physical/metadata cleanup or logging; all dependencies remain retained. |

There is no `ResetSource::Drop` hardware fallback. Its endpoint reference, command,
BAR descriptions and phase are inline. The containing grant's `ManuallyDrop`
retains all other resources; a numeric requester or overlapping address is not a
replacement owner.

## Execution evidence

Real QEMU staged unstarted admission rejects overlapping BAR and ECAM grants,
existing MMIO map/unmap and explicit close without consuming original authority.
Explicit cancellation releases that claim with bus mastering disabled, after
which normal reset/domain creation proceeds.

At actual reset wait boundaries, bounded probes require config/device guard
availability, preserved IRQ state, disabled bus mastering, exact BAR/ECAM claim
visibility and rejection of ordinary NVMe config/MSI lookup. Separate post-
publication/pre-activation probes require busy DMA capability close/unmap, exact
root-close exclusion, ECAM grant rejection and disabled bus mastering. Existing
private-prefix and published-creation rejection fixtures require post-guard
cleanup and exact refunds while this reset claim stays retained. Real reset,
reassignment and operational storage verification follow on the same unit.

RAM-backed config fixtures independently check admission rejection, intervening
unused-BAR type substitution, duplicate
claim exclusion, explicit unstarted cancellation, synthetic ready-state activation
and refusal to activate/cancel uncertain state. Two complete grants are abandoned
under config, lifecycle, both compiled backends, device, root/kernel table,
capability, physical and heap guards. Command bytes remain unchanged. They retain
**two additional exact user roots and original authority reservations**, plus
stable RAM config/topology metadata and endpoint claims. There is no new domain-
table or DMA data-frame charge; table-only intentional totals remain VT-d 36/27,
AMD-Vi 32/23 and SMMUv3 38/29 charges/frames. These synthetic states never supply a
physical-device or hardware completion proof.

Selected diagnostics omit routine boot output:

```text
[PCI reset ownership] RAM claim rejection, explicit unstarted cancellation/activation and complete-grant abandonment/uncertain cancellation under config and all guards passed; two exact roots/reservations and endpoint claims retained without hardware writes
[PCI reset publication] P real grants: exact root and published capability remain busy with BME disabled until explicit post-publication activation
[PCI reset claim] W real reset wait boundaries: config/device guards available, bus mastering disabled, ordinary config/MSI lookup rejected and captured BAR ranges fenced; wider lifecycle/backend guards remain
```

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result | Publication/wait probes (`P/W`) |
| --- | --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` | 3/10 |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` | 3/7 |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,876 cancellation requests retired | 3/11 |

Both final x86 runs used kernel SHA-256
`921c3592f303b58051acac1fcd02e8b58216eba2deedcc4642a532be90d75bbd`.
Arm used
`eaefdfdc73a7791890d4b03b9dfa1c7576351350b13818355931da6711406237`.
All three emitted all selected ownership/publication/reset markers. No guest
validation failed. Earlier passing iterations preceded the final publication
probe and exact unused-BAR substitution test; the table records final-source
runs only. Arm launches used the port-authorized runner because earlier turns
established the sandbox's forwarding-port restriction. Historical intermittent
guest failures remain separately documented and causally unresolved.

Strict default-feature kernel Clippy passed on both custom targets with
`--locked -- -D warnings`. Rustfmt, whitespace, local documentation links/anchors/
tables and the eighteen-family/seven-gate map passed. Runners reused validated
service bundles, rebuilt kernels and enforced assembly permissions (249 x86
entries and one Arm). Final guest runs were sequential after the last Rust
change. The host harness was not rerun for kernel-only changes.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance reset-claim-intel-checked-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance reset-claim-amd-checked-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18689 CATTEN_DEPLOY_HOST_PORT=17989 scripts/run-aarch64.sh --security-test --instance reset-claim-arm-checked-20261009 --fresh-storage --timeout 180
```

## Remaining scope

Initial domain construction, hardware configuration and reset still retain their
wider lifecycle/backend serialization. Complete backend preparation ownership
must precede releasing those holds; the endpoint claim alone cannot protect unit
tables, registry slots, command-engine state or original grant admission.
MMIO kernel-map preparation and general metadata allocation/destruction still
need outer-context qualification. This step does not close G1/G3/G4 or SEC-18.

Real timeout/withheld acknowledgements, failed activation readback, outstanding
I/O, concurrent multi-LP reset pressure and physical-device qualification are
not injected. Existing private command tests cover their separate timeout
queue/epoch contracts. There is no shutdown/custody/reset retry controller or
supported reset contract for other device models. Historical intermittent guest
failures remain unresolved; passing this scoped matrix does not establish cause.
