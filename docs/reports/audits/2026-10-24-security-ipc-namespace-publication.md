# Security audit: root-owned IPC namespace publication

Date: 2026-10-24. Reviewed baseline: `cc5ecc26`.

IPC namespace construction now prepares its node and both budget accounts
fallibly in the existing root-registration owner, before publication guards.
Runtime IPC requires an existing namespace and cannot recreate one after close.
The [tracked evidence](evidence/2026-10-24-security-ipc-namespace-publication.json)
records source/model/artifact hashes, selected diagnostics and failed runs
separately from passing validation.

This addresses the concrete namespace-construction gap recorded in the
[root metadata batch](2026-10-24-security-root-metadata-admission.md), within
SEC-07 R7-2/R7-3 and SEC-18's owner/context work. The finding ledger remains
**29 corrected / 6 partial / 3 open**. SEC-07/18 remain partial; production trust
remains disabled.

## Construction and handoff

Previously `IpcRegistry::as_caps` lazily inserted an `AsIpcCaps` under IPC. That
allocated an outer `BTreeMap` entry and two infallible budget `Arc`s during first
use. It also allowed runtime operations to create a namespace after teardown.

`PreparingNamespace` now owns a fallible shared admitted-map node and two
fallible account allocations. `PreparingRegistration` retains that preparation
with its unpublished root, unified authority namespace, memory sponsor and root
metadata. Ordinary preparation/table rejection cancels unused fields after local
publication guards leave, then disposes the never-published root. Partial and
complete abandonment retain all fields without implicit allocator or registry
work.

A fresh registration takes its first exact-generation root operation lease under
lifecycle/table serialization. It then releases both guards, relinks the
prepared namespace under IPC alone, and releases the lease after IPC unlock.
The root handle is returned only after that handoff. Immediate close rejects
while the lease is active; staged cleanup must wait for it before entering IPC.
This avoids a root-table-to-IPC edge against existing IPC root lookups. A
registered root observed before namespace insertion rejects IPC admission.
Abandonment retains the same containing owner and lease, without a retry path.

The BSP prepares the immortal kernel namespace before scheduler/AP startup.
Serialized kernel fixtures explicitly prepare synthetic namespaces in the top
half of `usize`, outside the Vec-backed root table's possible slot range. They
receive no trusted reserve classification. Real roots use registration; no lazy
compatibility path remains.

## Teardown and accounting

Existing fenced loan/token/capability drain still precedes namespace removal.
Cleanup now detaches the empty complete node under IPC and explicitly releases
its node/account references after IPC unlock. Shared `RetiredEntry` fallback
retains the node and both account owners without locks or allocator entry.
Foreign connections, closed endpoint records and pending charges retain their
original account references independently, including after root/ASID reuse.
Node removal supplies no physical completion or charge-refund proof.

This extends C10 and the existing root registration/operation owners. No root,
pin, registry, recovery family, physical retry authority or custody is added.
The [cleanup row](../../reference/cleanup-recovery.md),
[contract](../../reference/ipc-namespace-publication.md),
[lock order](../../reference/locking.md) and Markdown/LaTeX manual are updated.

## Verification

Serialized controls run in both native kernels:

- **256 phase rejections** cover entry, node-only, node-plus-endpoint-account and
  complete unused preparation. Every rejection precedes ASID publication;
  root count, physical frames and original unified/root-metadata reservations
  return to their starting values. Hooks verify local lifecycle/table/IPC/heap
  guards are available at preparation boundaries. These are injected phase
  errors, rather than a claim of global allocator exhaustion.
- **64 real root handoffs** attempt endpoint creation before namespace insertion
  and receive `PermissionDenied`. Immediate root close reports
  `OperationsInFlight`. After publication the endpoint can be created/closed;
  root close completes, and subsequent endpoint creation cannot recreate it.
- **64 heap-held local publications/detaches** use prepared storage without
  allocator entry. Explicit post-guard node disposal drops its final account
  references. Partial and detached-owner abandonment under IPC/heap guards
  deliberately retains four standalone nodes and five accounts. The existing
  containing-root abandonment also retains its IPC preparation.
- Existing endpoint/connection sponsorship controls keep old account charges
  live across root retirement, delegated ownership and exact ASID generation
  reuse. Real scheduled loan, cancellation, endpoint and namespace retirement
  controls remain enabled.

The final ARM run passes **20 / 0 / 0**, and the first final-layout x86 run
passes **18 / 0 / 0**. Each completes two scoped-security generations and all
**14 host HTTP controls**. Healthy replies beside stalled peers take
**0.313–0.381 s** on ARM and **0.292–0.438 s** on x86. Trickled deployment bodies
reject in **30.09 s** and **30.23 s**; concurrent cancellation traffic retires
after **20,568** and **18,348** requests respectively.

Four earlier ARM integration runs failed at fixtures that still expected lazy
namespace creation: the endpoint-admission client reuse, close-watch inspection
client reuse, synthetic syscall owner, and UART verifier client. Each run had
already passed the new namespace/root-publication controls. Their exact panic
sites, messages and kernel/capture hashes are retained in the evidence. The
fixture setup now explicitly prepares each synthetic namespace, including the
service-restart verifier found during the broader caller review. No production
fallback, quota widening or timeout change was used. These failures are distinct
from the earlier, causally unresolved queued-loan mapping panic and root-close
timeouts.

All **15 shared collection host tests** pass, including admitted-map detachment
and explicit payload/node disposal. Strict base-kernel Clippy passes on both
architectures; workspace formatting, whitespace and shell syntax checks pass.
Userspace sources and embedded service bundles are unchanged. The tracked manual
PDF is preserved; living Markdown and LaTeX sources describe the new boundary.

## Formal qualification

Nine focused checks pass: two safe configurations and seven expected
counterexamples from `CharlotteRootRetirement` and `CharlotteNamespaceDrain`.
The root graph has **2,978 distinct / 3,733 generated states**, depth 33; namespace
drain has **58 / 71**, depth 6. Required safe actions execute; every negative
case fails its named invariant through the intended unsafe action.

No model/configuration changed. The existing root model projects the publication
lease as an active operation that prevents cleanup/detachment; namespace drain
projects the existing exact-token boundary. `CharlotteIpcPublication` assumes
already-live namespaces and does not model their allocations or this handoff.
Those implementation boundaries are qualified by the native controls above.
The preceding complete **190-case** run remains the full-suite checkpoint,
recorded in the root metadata report. It is not rerun or relabelled here.
These checks provide no executable refinement, allocator/IRQ proof or sustained
progress guarantee.

## Remaining work

Per-capability `AsIpcCaps::insert_admitted`, endpoint, pending-call and reply-token
maps still allocate under IPC. Their preparation/publication/disposal must be
qualified coherently; namespace preparation alone does not make IPC operations
fully fallible. Account/control-block byte and logical-principal ceilings,
allocator overhead, other vectors/observer/weak backing, enclosing IRQ/guard
contexts and complete heap accounting remain R7-2/R7-3 and G1/G4 work.

The admitted namespace registry uses linear lookup/insertion. Serialized
publication/rejection controls do not qualify contention, allocator/IRQ hold
time, concurrent hostile preparation or management/timer/cleanup fairness.
Existing failures retain their historical diagnostics and causal limitations;
passing this batch cannot explain or authorize replay of uncertain cleanup.
