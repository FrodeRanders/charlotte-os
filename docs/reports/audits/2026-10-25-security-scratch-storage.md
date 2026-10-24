# Security audit: prepared scratch namespace and extent storage

Date: 2026-10-25. Reviewed baseline: `819b5f80`.

Scratch namespace and extent nodes now prepare fallibly outside the scratch
registry. Publication and exact release only relink admitted nodes; disposal
leaves that guard. Sealed root cleanup carries complete scratch metadata in the
existing root receipt through final invalidation. MMIO retains its existing root
and descriptor claim while its scratch factory also leaves the device registry.

This advances SEC-07 R7-2 and C02/C03/C05/C13/C18. The ledger remains **29 corrected /
6 partial / 3 open**; SEC-07/18 remain partial and production trust disabled.
The [tracked evidence](evidence/2026-10-25-security-scratch-storage.json) records
source/model/artifact hashes, selected diagnostics and results. Historical
reports, including earlier unresolved failures, retain their own checkpoints.

## Fallible storage and exact identity

The former global `BTreeMap` insertion could allocate infallibly under the
scratch registry. Each window's checked vector growth also allocated there.
Namespace destruction freed vector/tree storage while lifecycle was held, before
final root invalidation. Both maps now use the shared admitted-node collection:
one namespace node and one node per live extent. Implicit gaps preserve first
fit and exact range matching without vector shifting/coalescing storage.

Preflight checks bounded nonzero page-aligned size and current availability.
One inert preparation retains extent storage, namespace candidate and the exact
captured handle before either factory. Ordinary prefix rejection explicitly
finishes after allocator return and local guard exit. Partial abandonment retains
all fields without cleanup. Existing callers keep an operation/closing root or
serialized fixture context; the scratch handle is not a new root lease.

Final publication rechecks namespace identity and current first fit. A competing
preparation can publish first; the loser keeps its unused candidate until
post-guard finish, and its extent takes the next current gap. A different root
generation rejects without resetting or replacing any namespace. Exact extent
release detaches its original node before explicit post-scratch disposal.
The 512 MiB virtual limit, range semantics and public ABI are unchanged.

## Complete root retirement and MMIO admission

Sealed cleanup transfers the complete namespace and remaining extent nodes into
the existing `RetiredMetadata`, already held by `ClosingAddressSpace` and
`RetiredAddressSpace`. Final invalidation rejection keeps them with the exact
root, leased slot, backing, authority/policy metadata and original charges.
Confirmation permits explicit child/namespace node disposal after local
lifecycle/scratch/other registry guards, before root backing/slot completion.
Guarded abandonment retains every field without allocator or counter entry.

This extends the same complete C02/C03 receipt and its existing bounded
invalidation custody. There is no separate metadata ticket, registry, owner
family, root/pin, retry budget or hardware operation. Uncertain memory/MMIO
cleanup still prevents root sealing; scratch absence or range matching supplies
no replacement physical proof or accepted-work replay authority.

MMIO scratch preparation now leaves `DEVICES` while the original exact-root
operation and MMIO claim remain armed. The same physical descriptor/pages and
unmapped claim revalidate before table work. Source review found that the
previous `MmioOperation::begin` rechecked capability/type/in-flight/reset state
but omitted its mapped/unmapped expectation after the preliminary probe. Another
completed operation could change that state between probe and claim. Claim
admission now checks that expectation under the same device hold as its in-flight
bit, rejecting `AlreadyMapped`/`NotMapped` before mutation. This is a
source-confirmed admission correction, not an executed concurrent exploit.

Ordinary mapping, reset fencing and uncertain teardown rules are unchanged.
Other MMIO table-walk/outer contexts remain separate qualification. No owning
capability is reconstructed from a scalar cleanup selector.

## Verification

The **22 host cases** pass: six production scratch cases and sixteen shared
collection cases. Scratch checks include first fit, implicit gap reuse, exact
release rejection, missing preparation, invalid/exhausted/overflow requests,
independent windows and a **6,000-operation bitmap oracle**. The standalone host
crate imports the same collection implementation; it contains no alternate
production allocator. Shared controls trace allocation-free relinking, inert
abandonment and actual node/disposal ordering.

Serialized native controls exercise:

- **192 preparation-prefix rejections**: 64 before extent storage, 64 after
  extent storage and 64 after namespace storage. Scratch, heap, table and frame
  allocator are available at the boundary. No namespace/range publishes; original
  frame/authority counts return. These are stage-adapter rejections, not global
  allocator exhaustion.
- **64 heap-held namespace/extent publications and detachments**, including
  first and reused namespace paths, an unused candidate retained through commit,
  implicit gap reuse and post-guard disposal. The isolated metadata controls
  install no page tables and expose no external reservation; they claim no
  hardware completion. IRQ state is preserved.
- Invalid and full-window requests reject before factories. Competing fresh
  namespace publication recomputes current first fit. A preparation captured
  before real root retirement/ASID reuse cannot publish into, release from or
  detach the successor namespace; the successor's extent remains intact.
- Guarded partial preparation and isolated retired namespace abandonment retain
  **six metadata nodes**: three across partial phases and a namespace with two
  extent nodes. No data page or root/lease is retained by these isolated controls;
  their six allocations still consume kernel heap. The real root closes afterward.
- Existing root-custody controls now populate actual scratch metadata before
  staging. Rejected final invalidation retains its original extent/namespace in
  that same receipt. Confirmed retry disposes scratch metadata after local guards
  and before root physical release. Exact ASID reuse preserves the successor;
  stale scratch extraction returns no owner. Two scratch release entries check
  local guard availability and the original IRQ policy.
- The existing guarded abandoned root-custody attempt now also holds the scratch
  registry while dropping its complete owner. Its namespace and extent remain
  retained with that already-quarantined root/slot, mailbox/queue backing,
  authority and original charges. This adds **two retained metadata nodes** to
  that existing footprint, with no additional data page/root quarantine or reset.
  Other existing failure paths likewise retain their corresponding scratch nodes;
  these selected counts are not the total kernel quarantine.
- MMIO controls reject both wrong mapped-state expectations at claim admission,
  and inject scratch-preparation rejection while proving `DEVICES` is available.
  Existing real MMIO map/unmap/reuse, device close/reset, memory/loan mapping,
  cancellation, returned resources and scheduled namespace tests remain enabled.
  These controls do not prove arbitrary schedules or exhaustive outer contexts.

Both native runs pass: **AArch64 20 passed / 0 failed / 0 pending**, and
**x86_64 18 / 0 / 0**. Each completes all **14 HTTP checks** and both
security-service generations. Healthy responses beside stalled peers take
**0.239–0.382 s** on ARM and **0.266–0.435 s** on x86. Trickle checks reject
after **30.15 s / 30.30 s**; cancellation pressure retires after
**19,652 / 17,700 requests**, respectively.

Each run records four host `ConnectionResetError` retry diagnostics, separately
from guest results. Both initial metrics attempts pass; subsequent retrying
checks also pass within the existing limits. No timeout/attempt bound is widened,
and these observations provide no retrospective causal explanation.

Strict base-kernel Clippy passes on both architectures. Workspace formatting,
whitespace and local reference checks pass. Both native runners retain their
assembly-section checks. Bundled services are reused; userspace sources and the
tracked manual PDF remain unchanged. These are the first native executions for
this source; no native failure or guest repeat occurred in this batch. Passing
runs do not attribute historical mapping panics, root-close timeouts or initial
network/probe failures.

## Formal qualification and remaining work

Both focused safe configurations and all **seven expected counterexamples**
pass their harness checks. `CharlotteIpcTransaction` explores **26 distinct /
71 generated states**, depth **11**; `CharlotteRootRetirement` explores
**2,978 / 3,733**, depth **33**. Required safe actions execute; negative cases
violate their named invariant through its intended unsafe action.

No model/configuration changes. Root metadata/physical-once/original-charge and
loan/rollback/non-terminal-timeout projections remain. These models do not
implement virtual extents, first fit or the MMIO preliminary-probe race. Concrete
node construction/disposal, exact identity and allocator/IRQ/field-destructor
contexts are native/host qualification, without executable refinement or
sustained-pressure fairness proof. Earlier full-suite checkpoints stay separate.

Scratch storage has no independent metadata-byte/count or aggregate principal
admission. Concurrent unused candidates and per-node allocator overhead still
consume the general heap. First-fit/global lookup/insertion are linear; page and
resource counts do not qualify worst-case hold time or management/timer/teardown
progress. Scalar range matching is not an allocation nonce and still requires
its containing owner/proof. Raw serialized fixture disposal and broader
factory/rollback/destructor/outer IRQ contexts remain open. Extent storage gives
no new readiness, physical cleanup or replay authority.
