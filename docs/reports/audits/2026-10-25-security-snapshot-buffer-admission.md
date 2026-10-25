# Security audit: shared snapshot-buffer admission

Date: 2026-10-25. Reviewed baseline: `115a80fd`.

Mapping/copy/DMA snapshot buffers now reserve requested bytes and one buffer count
before allocation: **4 MiB / 8,192 total**, **3 MiB / 6,144 ordinary**. The same
typed owner retains buffer and original admission through publication, root
retirement and abandonment. Actual buffer free precedes refund.

The [tracked evidence](evidence/2026-10-25-security-snapshot-buffer-admission.json)
records source/model/artifact hashes and selected diagnostics. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production trust
disabled. This batch adds enforcement in one previously unbounded metadata family;
it does not close the broader findings.

## What acceptance work advances

| Gate | Result in this batch | Remaining |
| --- | --- | --- |
| R7-2: allocation admission | Snapshot reservations reject before the buffer allocator/publication and retain their original owner. | Other metadata families and the complete allocation inventory. |
| R7-3: aggregate resource limits | One global ordinary/total requested-byte/count pool covers overlapping and retained snapshot buffers. | Signed-principal snapshot/general-heap limits, inline metadata and allocator overhead. |
| R7-4/R7-5: progress under pressure | Actual-buffer exhaustion, trusted reserve, and cleanup at exhaustion in serialized kernel controls. | Sustained concurrent EL0 pressure and measured management/timer/teardown progress. |
| R18/G1: ownership and context | Existing typed buffer/pin owners retain charges; guarded publication/abandonment and deallocation ordering are checked. | Broader caller/destructor/IRQ contexts, concurrent hardware failures and authorized recovery custody. |

Recent allocation-order, lock-context, generation and abandonment corrections
made these owners suitable for this admission boundary. Finding counts track
whole acceptance scopes; narrow corrections and passing guests do not imply
completion of the rows above. The [current contract](../../reference/snapshot-buffer-admission.md)
describes exact coverage and exclusions.

## Captured reservation and one allocation owner

Every factory now receives the existing caller's captured namespace handle.
Trusted memory-sponsor classification is captured once against that exact handle
before the counter/allocator; scalar ASID reuse cannot select a successor's
reserve. Caller names/manifest roles grant no reserve. Borrowed backing does not
substitute the backing owner's policy for the calling namespace. Later promotion
cannot reclassify an existing buffer's reservation or refund.

The new `Snapshot` holds vector storage and its charge in inert fields. Admission
reserves `pages * size_of::<PAddr>()` and one count before `try_reserve_exact`.
These are requested buffer layouts, excluding allocator overhead/excess storage,
inline owner fields, mapping nodes and other metadata. Snapshot quota rejection
returns `ResourceLimit` before its allocator. Allocation failure returns
`FrameAllocFailed` and explicitly cancels the unused reservation. Existing backend
error conversions and public ABI remain unchanged.

Publication transfers the same charged owner into `CopyPin`/`DmaPin`, or leaves it
in mapping preparation. No second charge or counter entry occurs. Ordinary
cancellation/disposal frees the buffer before refund. Abandonment retains original
allocation/reservation without allocator, registry or counter entry. Reservation-
only or post-free interruption likewise cannot fabricate a refund; concrete
allocator-interruption schedules remain separate qualification.

Snapshot cleanup still precedes independent backing-pin completion. Its refund
therefore proves no data/hardware quiescence and permits no map/unmap replay.
Original root/capability, mapping-retirement, loan and hardware maintenance owners
remain required. Empty source-escrow pins allocate no vector and consume no buffer
quota; they keep their original source fence. Zero-size ordinary snapshot
preparation rejects rather than taking that distinct no-buffer path.

## Verification

Serialized kernel controls exercise:

- **64 allocator-adapter rejections** with admission already live, local memory/
  table/heap/frame guards available, and exact baseline restoration. Zero-size
  and arithmetic-overflow requests reject without a reservation. This does not
  force global allocator OOM.
- **64 buffer-free-before-refund observations** confirm the original reservation
  remains live after actual vector disposal and refunds afterward. Promotion
  between capture and allocation leaves the original ordinary charge unchanged;
  source-root retirement/real ASID reuse cannot redirect its late refund.
- Real one-entry buffers reach **6,144 ordinary / 8,192 total count limits**.
  Sixty-four further ordinary attempts reject before invoking the allocator.
  Public mapping, copy-pin and DMA-pin requests reject without new pins, mapping
  publication or original source mutation. Empty source-escrow preparation still
  succeeds with no snapshot charge. A trusted source uses reserve while ordinary
  admission is full.
- At total exhaustion, the original source root retires with its already-owned
  unused copy pin/buffer still charged. Explicit old-pin cleanup frees the buffer
  before returning its original ordinary share despite promotion. A real successor
  uses that freed slot through its own captured generation and distinct object,
  then closes independently. Actual buffers are explicitly drained; exact
  admission baseline returns without resizing, reset or synthetic refund.
- Actual buffers up to **128 KiB** reach the **3 MiB ordinary / 4 MiB total byte
  ceilings** before count limits. A final exact remainder buffer fills each byte
  allowance. All three public snapshot paths reject without publication. Explicit
  deallocation returns the exact admission baseline; this is not a physical-RAM
  baseline because the kernel heap may retain free backing.
- Existing mapping/copy/DMA prefix controls now check admission restoration.
  Their heap-held publications and guarded abandonment also hold the new counter.
  Mapping's two retained snapshots keep **560 requested bytes / two reservations**;
  copy's unused/real snapshots keep **288 / two**; the bare DMA snapshot keeps
  **8 / one**, all in their original ordinary shares. These are existing fixture
  footprints newly charged, not new data/root quarantine or total kernel retention.

Existing actual backend map/rollback/unmap/destruction, loan/cancellation,
returned-resource, root-custody and readiness controls remain enabled. Strict
base-kernel Clippy passes on AArch64 and x86_64. The kernel-coupled path is exercised
directly; no alternate host implementation is introduced.

Each backend's first guest execution completes successfully, with all **42 HTTP
checks** and both security-service generations per backend. Host request attempts
remain separate from guest results:

| Backend | Deferred tests (passed/failed/pending) | Healthy beside stalled peers | Trickle rejection | Cancellation requests |
| --- | --- | --- | --- | --- |
| smmuv3 | 20/0/0 | 0.177–0.398 s | 30.13 s | 19,360 |
| intel-vtd | 18/0/0 | 0.291–0.376 s | 30.19 s | 17,808 |
| amd-vi | 18/0/0 | 0.340–0.415 s | 30.19 s | 13,892 |

No guest failure or repeat occurs in this batch. No deadline, attempt bound or
admission limit is widened. Passing runs do not attribute historical failures.

Initial metrics observations, retained with surrounding host output:

- **smmuv3:** PASS with no preceding retry diagnostic.
- **intel-vtd:** HTTP transport TimeoutError; retry 1/2; then PASS within the original bound.
- **amd-vi:** PASS with no preceding retry diagnostic.

All transport retry diagnostics remain inline in the evidence, separately from
guest completion and passing repeats. The initial-metrics timeout recurs at the
same host probe phase recorded in the [mapping preparation batch](2026-10-25-security-mapping-preparation-ownership.md);
its cause remains unestablished. No successful retry supplies historical failure
attribution or physical/replay authority.

Workspace formatting, whitespace and local references pass; native assembly
section checks remain enabled. Bundled services are reused; userspace sources and
the tracked manual PDF remain unchanged. Historical failed runs retain their own
records and verification limits.

## Formal qualification and remaining work

All **23 focused harness cases** pass: three safe configurations and **20 expected
counterexamples**. DMA operation explores **29,175 distinct / 62,003 generated
states**, depth **24**; IPC transaction explores **26 / 71**, depth **11**;
root retirement explores **2,978 / 3,733**, depth **33**. Required safe actions
execute; negative configurations violate their named invariant through the
intended unsafe action.

No model input changes. Existing root original-charge, IPC loan/rollback/timeout
and DMA ownership projections remain relevant. None represents these concrete
pool dimensions, per-buffer captured classification, allocator layouts or
destructor/IRQ contexts. Native/source controls qualify those boundaries without
executable refinement, arbitrary-schedule or sustained-pressure/fairness proof.

The acceptance gaps listed above remain open. A common platform reserve grants
no guaranteed share to one essential service, and signed-principal metadata
admission is still absent here. General kernel heap, other metadata families,
source-transfer/scoped cleanup and broader caller/IRQ/destructor contexts still
need work. Historical mapping panics, root-close timeouts and initial metrics
failures remain separately unattributed. Snapshot refunds supply no physical
completion, authorized recovery custody or accepted-work replay permission.
