# Security audit: memory-capability payload and namespace admission

Date: 2026-10-25. Reviewed baseline: `30e4aeb7`.

This returns to SEC-07 remediation after the ownership refactoring batches.
Memory-capability payload and namespace nodes previously allocated fallibly but
had no independent requested-byte/count admission. Unified authority counts do
not cover overlapping unused namespace candidates or payload metadata retained
after hidden authority has been retired. This is a source-confirmed allocation
gap; no sustained hostile EL0 exhaustion was reproduced before correction.

Both node layouts now reserve before allocation, retain original charges
through actual storage lifetime and reject without publishing a destination.
Defaults are **4 MiB / 32768 nodes total**, **3 MiB / 24576 ordinary**.
`Policy.memory_capability_nodes` groups both dimensions centrally; operators can
select a reviewed compile-time profile with `CATTEN_RESOURCE_POLICY`. Existing
defaults in other families are unchanged. The [current contract](../../reference/memory-capability-node-admission.md)
documents tuning, two-node preparation overlap and exclusions.

The [tracked evidence](evidence/2026-10-25-security-memory-capability-node-admission.json)
records source/artifact hashes, controls and relevant failure diagnostics.

## Acceptance work advanced

| Gate | Result in this batch | Remaining |
| --- | --- | --- |
| R7-2: allocation admission | Payload and namespace node reservations precede their fallible allocators and remain in the existing prepared/published/detached owners. | Other object/borrower/namespace/account metadata and the complete allocation inventory. |
| R7-3: aggregate resource limits | One ordinary/total requested-layout byte/count pool covers live, unused and retained nodes from this family. | Signed-principal metadata, general kernel heap, nested allocations and allocator overhead. |
| R7-4/R7-5: pressure | Real-node exhaustion, trusted reserve, public rejection and existing-resource cleanup at exhaustion in serialized controls. | Sustained concurrent EL0 pressure with measured management/timer/teardown progress. |
| C05/C18 and G1/G4 | Original typed node/backing/authority ownership remains; guarded publication, detachment and abandonment retain original charges. | Containing transfer/backing and wider outer IRQ/guard/destructor qualification. |

The ledger remains **29 corrected / 6 partial / 3 open**. SEC-07/18 stay partial;
the concrete enforcement above does not claim full completion of either finding.

## Reservation and lifetime

The existing `Preparing` owns authority/account references and its captured
namespace before either factory. It now captures trusted classification once
against that exact handle, before the factory hooks. Caller names, roles and
manifest principals provide no reserve. Later promotion or ASID reuse cannot
redirect a node's original admission/refund.

Each payload/namespace node reserves its complete requested allocation layout,
including link and typed charge, plus one count. The shared `AdmissionPool`
counter is a leaf mutex released before allocator entry. Quota or allocation
rejection returns the existing `ResourceLimit`; a second-node rejection
explicitly disposes the first before cancelling hidden authority. Data backing
allocation and payload publication have not begun on that path.

Existing namespaces still require a candidate during concurrent preparation.
Two temporary slots are therefore required even when only one new payload node
will remain published. The unused candidate is charged until explicit finish
after local guards. Counts are independent of unified authority and root limits;
raising either does not silently rescale this pool.

`PreparedEntry` carries the same charge through the existing map/list nodes and
retired capability/namespace owners. Linking, publication and detachment allocate
nothing and enter no new counter. Explicit destruction frees the payload and
actual node before its original charge returns, then follows the existing
independent authority disposal order. Source escrow, loan cleanup and complete
object/backing receipts retain their original physical completion requirements.

Preparation and retired-node fallback remain inert. Namespace authority teardown
can leave abandoned unused metadata behind; its original byte/count admission
now remains retained too. No root/pin, custody registry, physical proof or replay
owner is added. Refund proves only node disposal, never backing/hardware
quiescence. No lock/IRQ masking policy is relaxed.

## Verification

The kernel-coupled controls use the real production node allocator and public
memory operations, with no alternate host implementation or quota reset:

- **128 allocator-adapter rejections**, 64 for each layout, observe live byte/
  count admission and available local memory/table/heap/frame guards, then
  restore the exact baseline. This does not inject global allocator OOM.
- Captured ordinary classification survives promotion during the first factory
  hook, real root retirement and ASID reuse; explicit late finish refunds its
  original ordinary shares.
- Heap-held publication/detachment with the admission counter held shows that
  relinking neither allocates nor enters the counter. Detached storage remains
  charged until explicit post-guard release.
- Actual unused payload nodes exhaust ordinary admission. Sixty-four further
  attempts for each layout reject before their allocator. Public allocate,
  move, copy and both loan directions reject with original authority, backing
  usage, snapshot counts, source fences and source usability intact.
- With just one slot returned, two-node allocation rejects and restores that
  slot. Trusted creation succeeds while ordinary capacity is full. Total
  exhaustion then rejects trusted preparation too. Existing real capability
  close and namespace retirement free their original ordinary nodes despite
  promotion; a real successor consumes the freed capacity and closes
  independently. Draining actual candidates restores the exact initial baseline.
- Existing 192 construction-prefix controls now check metadata admission too.
  Guarded abandonment holds memory/table/heap/frame/shared-authority and the
  new counter guards. Four unused nodes retain their original **256 bytes /
  four reservations** after local published-node disposal, real root retirement
  and successor reuse. These are existing fixture metadata allocations newly
  charged, not additional quarantined data, roots or pins.

The serial summary's source-preservation result concerns source authority/data
and the final pin/fence state. Copy preparation still admits its normal temporary
snapshot/shared pin before destination metadata admission; returned rejection
explicitly releases it through the existing scoped owner. No denial-before-
snapshot claim is made for this path.

Both layouts request **64 bytes** on the qualified targets. Their count ceilings
bind before the configured byte ceilings; native controls do not claim actual
byte-ceiling exhaustion for these fixed layouts. The host policy harness exercises
both counter dimensions and rejects invalid reserve/machine-range settings for
the new field, in default/larger profiles and file selection tests.

Initial AArch64 Clippy rejected access to two private fixture helpers (`E0603`,
`available` and `payload`). Their visibility now permits the sibling admission
fixture within the same storage module; no production API was widened. Final
strict base-kernel Clippy passes on AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`.

Native fresh-storage, four-LP security/HTTP runs used the previously rebuilt
signed userspace bundles, sequentially across the three supported IOMMU backends.
Each first guest execution finished with `reason: complete`; both security
publication generations (1 and 2) passed on each backend:

| Backend | Deferred passed / failed / pending | HTTP checks passed | Ordinary / reserve unused payload nodes at pressure |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 24550 / 8189 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 24534 / 8189 |
| x86_64 / AMD-Vi | 18 / 0 / 0 | 14 | 24534 / 8189 |

Existing live/baseline nodes consume the rest of each common ceiling; the table
does not count them as newly allocated pressure candidates. Each run restores
its captured baseline after candidate drain and real cleanup, then confirms the
separate four-node guarded-retention footprint. Existing loan/cancellation,
returned-resource, DMA, root retirement and readiness controls remain enabled.
These are serialized pressure controls and normal integration, not sustained
hostile concurrent EL0 pressure or a new physical-failure campaign.

Each host HTTP run records four `ConnectionResetError` retry diagnostics: one
after EOF, one after abortive peer closes and two during the idle-client probe.
All affected probes subsequently pass. Evidence preserves all twelve diagnostics
and their eventual results. No guest failed or was repeated; no timeout, attempt
bound or existing quota was widened. Passing runs do not attribute historical
first-entry/scheduler, IPC, root-close, metrics or NIC failures.

The default/larger policy harness, formatting, whitespace, frozen source hashes
and local Markdown targets pass. The manual was rebuilt through two final TeX
passes and affected PDF pages 51, 198 and 237 were rendered and visually checked.
An initially overflowing policy-name line was corrected. The final build emits
190 overfull-box warnings in the full document; this scoped review
does not re-audit all 249 pages. No undefined references remain in that build.

## Remaining acceptance boundary

This fixes allocation admission for these payload/namespace nodes. Other object/
borrower nodes, account/control-block/observer storage, comprehensive principal/
heap admission and measured essential-service progress under sustained pressure
remain R7-1 through R7-5 work. The current node count binds before bytes; altered
profiles and future layouts need their own workload qualification. Broader
containing-operation/IRQ/destructor contexts and recovery custody remain C05/C18
and G1–G7 work. No formal model/refinement or historical-causality result is
added. SEC-07/18 remain partial and production trust stays disabled.
