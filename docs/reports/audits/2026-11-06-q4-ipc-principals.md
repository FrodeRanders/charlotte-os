# Q4 IPC buffer principal admission

Dated 2026-11-06 (Europe/Stockholm); campaign preparation timestamps are UTC
and begin on November 5. Baseline `9ad27b1f`. This batch adds original-principal byte funding for IPC
descriptor copies, parsed entries, transfer/cap-ID staging, token loan origins,
reply/cancellation work and attachment retirement catalogs. **Q4 remains in
progress**. Payload nodes, endpoint queues, account backing, allocator overhead
and joint heap capacity remain separate. The [24-pool checklist](../../reference/metadata-admission.md#principal-coverage-and-remaining-reconciliation)
records qualified scope and work still to reconcile; pool counts are not a
weighted readiness estimate.

The [qualification evidence](evidence/2026-11-06-q4-ipc-principals.json) records
frozen inputs, artifacts, diagnostics, layout measurements and final campaigns.
No user application, SDK, wire format, resource ceiling or signed bundle changed.

## Captured funding and ownership

The existing principal-history kernel-buffer counter supplies the same 16 MiB
requested-byte ceiling already used by snapshots, primitive inputs, diagnostics
and root heap/image catalogs. IPC retains its 12/16 MiB ordinary/total family
bytes and 73728/98304 node ceilings. There is no new registry, pool, eager backing
or per-buffer principal allocation.

Calls retain storage class from their original logical record reservation and
capture the exact leased caller root's prebound principal outside IPC. Sends
likewise capture the exact sender principal after leasing their roots, preserving
the already captured trusted classification. Capturing funding supplies no root
lease or cleanup authority. Signed application roots cannot fall back to an
unidentified sponsor. Permanent kernel work and explicitly prepared synthetic
kernel-test namespaces retain their family-only boundary. Empty placeholders
allocate nothing and receive captured funding before production growth; boolean
allocator conversions exist only in test builds.

Vectors use the common sponsor-aware byte allocator. Principal and family
admission precede allocation through separate leaf holds. Growth reserves the
whole candidate while the old capacity stays charged. Cloning policy reserves
nothing; popping or clearing does not refund live capacity. Actual free precedes
both refunds. Family/allocation rejection returns only the unused prefix.

Cap-ID storage moves into its existing attachment owner with the same allocator.
The fixed retirement catalog pairs a linear `ByteCharge` with its existing
family reservation, borrowing that original sponsor. The same inert owner keeps
caps, complete capacity and charges through queueing, delivery, FIFO retirement
and callback interruption. Preparation checks exact capacity before publication.
Explicit disposal frees storage before both refunds; abandonment retains it.

Reply/cancellation work clones the accepted token's loan-vector allocator.
A server or namespace-close worker therefore pays from the original caller's
buffer allowance, even after promotion, close or ASID reuse. Principal denial
occurs before preparing revocations or publishing a claim, preserving accepted
calls, reply authority and mapped loans. Ordinary disposal still leaves IPC and
finishes children/buffers before completing exact roots. No physical proof,
retry custody, replay permission or new active destructor is added.

## Capacity consequences

Native controls measure 120/272/200/200-byte capability/endpoint/pending/reply
nodes, 352-byte prepared transfers, 136-byte retirement entries and
984/528-byte reply/cancellation stack parents. The maximum 255-entry transfer
catalog therefore requests 89760 bytes; revocation work remains 51000 bytes.
Native x86 DWARF from the completed Intel campaign confirms the principal-bearing
allocator is **24 bytes, alignment 8**, and `MemoryAttachments` occupies 120 bytes.
The source and measured kernel identity are recorded in the evidence.

The composite fixed reservation and enlarged allocator add 40 inline bytes per
queued attachment owner. At existing ordinary/total queue-slot ceilings that
adds an upper bound of 960/1280 KiB before allocator overhead. The conservative
node-only mix now requests 8564736/11419648 ordinary/total bytes, still below
12/16 MiB before buffers, queues and other overhead. Requested node layouts and
queue allocation sizes include their current fields. Independent ceilings do
not guarantee simultaneous fit in the shared 72 MiB default mapped heap.

No tested bound requires increasing limits or reducing protocol maxima here.
An increase would permit more retained memory without guaranteeing cleanup
progress. In particular, work capacity is still requested after call acceptance:
family or principal saturation can reject reply/cancellation and delay retirement.
The complete accepted owner remains retained; Q7 still needs earlier cleanup
capacity or demonstrated bounded drainage and latency under real pressure.

## Controls and qualification

The host test uses the actual attachment owner and System deallocation tracer
with an explicit host sponsor adapter. It observes fixed retirement bytes still
charged at actual free, then both budgets restored. Principal rejection of the
second buffer cancels its already allocated cap-ID prefix. Existing allocation,
FIFO interruption and guarded-retention controls remain in the same full suite.
Host adapters do not qualify kernel IRQs, signature verification or native layouts.

Native fixtures bind private principals at the trusted constructor boundary;
combined security campaigns separately exercise verified artifact launch.
A private 96-byte ceiling checks shared sibling denial, full growth overlap,
unchanged pointer/data, cross-family diagnostic denial, trusted promotion and
an independent healthy sponsor. Retained buffers survive root close/same-slot
reuse and refund the old principal. An 8-byte ceiling admits a cap-ID prefix but
rejects retirement capacity without adopting a real source resource. Complete
metadata abandonment under history/IPC/heap/family guards retains 144 bytes
(8 cap-ID bytes plus one 136-byte retirement entry) after root close; that
intentional retention is separate from exact-recovery cases.

A zero-byte fixture ceiling rejects 64 public vector sends before parsing or
transfers, preserving source/descriptor and exact byte baselines; restoring
capacity permits the same request to deliver. A real mapped read-loan case sets
its private principal ceiling to currently retained bytes, then rejects 64
reply and 64 cancellation preparations without claim publication, loan loss or
charging the server. Restoring capacity permits confirmed revocation and exact
buffer refund. Fixture limit changes preserve every live charge; these are
serialized admission controls, not sustained concurrent EL0 saturation.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting and full host suite | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19, unchanged 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each, unchanged 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed, unchanged 90-second budgets |

All five campaigns reuse the exact signed bundle identities qualified by the
preceding snapshot-principal batch. Frozen source/input hashes remained unchanged
through the final campaigns. These results qualify the named buffer paths and
controls, not aggregate admission, production trust, arbitrary enclosing
caller/IRQ contexts or physical-platform isolation.

## Preserved preflights and limits

Initial compile checks caught the new composite charge's missing Debug derive
and pressure fixtures still using the former reservation alias. Further checks
caught a private reply constructor and memory test helper being called across
their visibility boundary; the control now uses the public reply operation and
a narrowly test-only history-guard helper. No production visibility was widened
for those controls.

A temporary layout harness first omitted its required selected-policy input.
Its corrected 2-byte result described the host adapter's boolean sponsor and
was not used as a production layout measurement. Native DWARF supplies the
24-byte production result above. The existing native capacity log's 1-byte
allocator value also describes its family-only default control type. Both
scope distinctions and the failed harness diagnostics remain explicit.

ARM display second-grant-probe, Intel retirement-pin and ARM exit-watch incidents
remain unresolved. Headless passing repeats do not explain or close them.
General caller/IRQ/implicit-Drop review, physical recovery and sustained service
progress remain Q5–Q8 work. Finding dispositions, all aggregate gates and
production trust remain unchanged.
