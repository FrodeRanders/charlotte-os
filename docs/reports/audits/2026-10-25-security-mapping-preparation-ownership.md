# Security audit: mapping preparation ownership

Date: 2026-10-25. Reviewed baseline: `a06f6183`.

Mapping preparation now owns its partial node and shared snapshot in inert
fields before either factory. Ordinary constructor rejection and every constructed
return explicitly dispose storage after local guards. Abandonment retains partial,
complete unpublished and published-snapshot storage without allocator entry.
Final mapping admission also checks the captured full namespace generation.

This advances C05/C18 and G1/R7-2. The ledger remains **29 corrected / 6 partial /
3 open**; SEC-07/18 remain partial and production trust disabled. The
[tracked evidence](evidence/2026-10-25-security-mapping-preparation-ownership.json)
records source/model/artifact identities and selected diagnostics, including
failed host request attempts separately from guest completion.

## Complete metadata ownership and ordinary disposal

The previous mapping preparation kept its node and vector in automatically
destroyed fields. Ordinary lexical return freed them after local guards, but
abandonment could invoke allocator-backed field cleanup beneath unknown guards.
Its constructor also kept partial storage in ordinary locals.

One inert owner now exists before the node factory and retains both partial
allocations. Snapshot construction, fill and frame borrowing use the shared
storage owner already used by copy/DMA, without merging their pin/completion
policies. Constructor rejection explicitly cancels the unused prefix after
allocator return. Ordinary post-construction work runs inside its existing local
guard scopes; after those leave, every return explicitly finishes the owner.
This includes source/generation rejection, success, prefix rollback and terminal
cleanup errors. The private per-call disposal probe runs immediately before that
finish; production supplies an empty probe, with no callback registry.

Abandonment leaves every remaining field intact, without allocation, deallocation,
registry entry, callback or physical cleanup. After publication the original node
is in the mapping registry/pin, while the snapshot stays in the preparation until
explicit ordinary disposal. Original caller roots, backing pins, prefix metadata,
scratch completion and typed invalidation proof remain required. Metadata absence
or a freed snapshot grants no backing release, physical retry or replay authority.

Final admission rechecks the captured full namespace generation before snapshot
fill, node publication or pin acquisition, alongside its existing object/size and
rights/loan/pin/range checks. Public mapping callers already retain their exact
root lease. The private serialized reuse control deliberately retires a pin-free
fixture during preparation; it does not bypass an admitted public root. No new
root/pin, admission pool, registry, recovery family or custody is introduced.

## Verification

Native controls cover:

- Existing **192 preparation-prefix rejections**, 64 at each node/snapshot stage,
  now exercise explicit partial cancellation. Factory memory/heap/table/frame
  guards remain available; no frame identity or mapping pin exists. Original
  frame/authority/backing counts return. An actual capacity-overflow request
  rejects through the same owner without publishing a node or pin. These are
  serialized stage controls, not global allocator OOM.
- Source close after preparation rejects the unused owner; its disposal probe
  sees local guards available and an empty frame snapshot. Real root retirement,
  ASID reuse and successor capability publication reject the captured generation
  before mapping/table mutation or pin acquisition. The successor stays writable
  and unmapped, then closes normally. Capability scalar reuse does not select the
  old namespace or its cleanup owner.
- **64 real ordinary disposal entries**, 32 successful maps and 32 foreign-leaf
  prefix rollbacks. Immediately before explicit finish, memory/table/heap/frame
  guards are available, the two-frame snapshot remains owned and the original
  retirement pin is still armed. Original IRQ policy is preserved. Existing
  invalidation then confirms completion before pin release; ordinary unmap and
  peer cleanup complete through their original owners.
- Existing **64 heap-held metadata publications/detachments** now use explicit
  post-guard preparation finish. Existing 32 prefix-lifetime and deliberate
  uncertain-maintenance controls remain enabled, including installed-prefix
  metadata and original root/backing charge retention.
- New guarded partial/full/published preparation abandonment retains **two empty
  node allocations and two snapshots** with 35-entry capacity requests. The full
  unpublished snapshot holds no frame identity or pin. The isolated published
  metadata case installs no PTE or externally owned mapping; its original pin/node
  explicitly finishes afterward, then source backing/root close normally. The
  remaining heap allocations are never re-adopted. These controls add **no user
  data backing, root or lease quarantine**; their four allocations still consume
  kernel heap. Existing uncertain rollback's two data pages/root remain a
  separate pre-existing fixture footprint, not part of these four allocations.

Copy/DMA snapshot, actual backend map/rollback/unmap/destruction, loan/cancellation,
returned-resource, root custody and device readiness controls remain enabled.
Strict base-kernel Clippy passes on both architectures. The kernel-coupled path
is exercised directly, without an alternate host implementation.

Each backend's first guest execution completes successfully, with all **42 HTTP
checks** and both security-service generations per backend. Host request attempts
remain separate from guest results:

| Backend | Deferred tests (passed/failed/pending) | Healthy beside stalled peers | Trickle rejection | Cancellation requests |
| --- | --- | --- | --- | --- |
| Arm SMMUv3 | 20/0/0 | 0.244–0.330 s | 30.12 s | 19,668 |
| Intel VT-d | 18/0/0 | 0.262–0.393 s | 30.21 s | 17,704 |
| AMD-Vi | 18/0/0 | 0.267–0.393 s | 30.15 s | 18,064 |

No guest failure or repeat occurs in this batch. No deadline, attempt bound or
admission limit is widened. Passing runs do not attribute historical failures.

Initial metrics observations, retained with surrounding host output:

- **Arm SMMUv3:** HTTP transport TimeoutError; retry 1/2; then PASS within the original bound.
- **Intel VT-d:** HTTP transport TimeoutError; retry 1/2; then PASS within the original bound.
- **AMD-Vi:** PASS with no preceding retry diagnostic.

All transport retry diagnostics remain inline in the evidence, separately from
guest completion and passing repeats. The initial-metrics timeout recurs at the
same host probe phase recorded in the [DMA snapshot batch](2026-10-25-security-dma-snapshot-storage.md);
its cause remains unestablished. No successful retry supplies historical failure
attribution or physical/replay authority.

Workspace formatting, whitespace and local references pass. Native assembly
section checks remain enabled. Bundled services are reused; userspace and the
tracked manual PDF remain unchanged. Historical failed runs retain their own
records and verification limits.

## Formal qualification and remaining work

All **23 focused harness cases** pass: three safe configurations and **20 expected
counterexamples**. DMA operation explores **29,175 distinct / 62,003 generated
states**, depth **24**; IPC transaction explores **26 / 71**, depth **11**;
root retirement explores **2,978 / 3,733**, depth **33**. Required safe actions
execute; negative configurations violate their named invariant through the
intended unsafe action.

No model input changes. Existing root original-charge/physical-once, IPC
loan/rollback/non-terminal-timeout and DMA operation projections remain relevant.
None models these preparation fields, the CPU-map source-interleaving interval,
vector disposal or allocator/destructor/IRQ contexts. Those are native/source
qualification, without executable refinement, arbitrary-schedule or sustained
pressure/fairness proof.

Overlapping candidates and retained metadata still lack comprehensive requested
byte/count and aggregate signed-principal admission. Broader caller/destructor/
IRQ contexts, containing source-transfer/scoped rollback, worst-case list hold
times, essential-service capacity and the full concurrent hardware/recovery
matrix remain open. No new readiness or accepted-work replay policy is supplied.
Historical mapping panics, root-close timeouts and initial probe failures remain
separately unattributed.
