# Security audit: prepared memory capability and namespace storage

Date: 2026-10-24. Reviewed baseline: `956c0cd4`.

Memory capability and namespace maps now publish prepared nodes. Direct
allocation, copy, move and loan prepare their payload and namespace storage
fallibly before publication. Complete mixed memory/IPC commit now needs no
heap allocation or node disposal. Retirement carries detached payload and
original authority together in the existing cleanup owners.

The [tracked evidence](evidence/2026-10-24-security-memory-capability-storage.json)
records source/model/artifact hashes, results and selected diagnostics. This
advances SEC-07 R7-2 and C05/C06/C18. The ledger remains **29 corrected /
6 partial / 3 open**; SEC-07/18 remain partial and production trust disabled.
Historical reports retain their earlier checkpoints.

## Prepared publication

The previous per-capability and global namespace `BTreeMap` insertions could
allocate after shared authority became live. Both maps now use the shared
admitted collection. Exact destination reservation precedes fallible payload
and namespace-candidate preparation, outside `MEMORY_OBJECTS`. Direct allocation
prepares before backing allocation; source transfers before escrow/pin mutation;
copies retain their already acquired source pin. Rejection disposes the unused
prefix after local guards without publishing a payload or destination.

A candidate captures its exact address-space generation. Batch/direct validation
rejects an existing namespace from another generation before shared publication.
A missing namespace consumes its candidate; an existing exact namespace keeps
the unused candidate in the containing preparation for post-guard disposal.
Empty live namespaces remain until the existing root cleanup. This adds no
namespace account, policy override, root lease or generation reconstruction.

Complete mixed commit validates, atomically publishes authority, extracts moved
source nodes and installs object/borrower/capability/namespace payloads by
relinking prepared storage. It neither allocates nor frees unused candidates.
The old authority-only test callback is removed; fixtures now hold the heap over
the complete production commit. Hidden-delivery, loan restrictions, nonreused
identity and returned-resource observation rules are unchanged. Wire grammar,
runtime ABI, capacity and timeout policies are unchanged.

## Complete retirement owners

A detached capability record pairs its payload node with exact shared authority.
Post-registry release frees payload metadata before authority metadata and its
original count refund. Successful moves keep their detached source payload in
`PreparedSource` alongside the original escrow/pin; source cleanup frees payload
before the same escrow releases its retired authority.

Read-loan revocation pairs the full payload/authority record with the existing
borrower node, retaining borrower-before-payload-before-authority disposal.
Write revocation carries that same typed payload/authority receipt. Uncertain
revocation keeps its prior map and existing fence/pin; no physical operation or
accepted request is replayed.

Direct close and final DMA/copy/mapping-pin destruction retain all removed
capability nodes in the complete object/backing receipt. Physical completion
precedes their explicit disposal, object-node free and original backing refund.
Failure or abandonment now retains original authority charges as well as backing
charges and complete metadata. No independently movable metadata ticket or
recovery registry is introduced. Namespace close detaches its whole node and
pairs remaining authority in place; post-guard release drains children before
freeing namespace storage. The serialized scalar fixture shares the same
metadata detachment but remains a fixture context.

Review additionally found a cancellation field-order gap in the prior transfer:
its destination reservation field preceded unused read-borrower storage. The
borrower field now precedes destination preparation, so ordinary cancellation
frees unused borrower storage, destination payload/candidate nodes, then refunds
the destination reservation. This corrects that specific implicit destruction
order; broader containing IPC/IRQ/destructor contexts remain separate work.

There is no new root, pin, registry, owner family, recovery custody, timeout,
physical completion proof or retry authority. Shared capability/root count limits
are not comprehensive metadata-byte admission. In particular, an unused namespace
candidate retained after successful publication has no independent byte/count
charge; containing-operation lifetime and broader admission remain separate.

## Verification

Serialized native controls exercise:

- **192 preparation-prefix rejections**: 64 before payload storage, 64 after
  payload storage and 64 after namespace storage. Shared admission precedes each
  boundary; memory/heap/frame allocator are available. Original counts and frame
  baselines return; no namespace appears. These are stage-adapter rejections,
  not exhaustion of the global allocator.
- **64 two-capability/namespace publications and detachments** use actual
  production node types with the memory registry and heap held. Removing the
  first record preserves the second. Both detached owners keep original shared
  counts until post-guard disposal; unused candidates remain owned at commit.
  Isolated metadata lives only in an unstarted fixture root/local registry;
  it installs no synthetic object into the global memory-object map.
- **Exact ASID reuse** rejects a fresh preparation against the retained old
  generation's isolated namespace before authority publication. Ordinary unused
  disposal restores the original count baseline without replacing old storage.
- The **257-member mixed batch** holds the heap through complete production
  validation, authority publication, object/borrower/payload/namespace relinking
  and source extraction. Hidden owning destinations stay unavailable until
  delivery. Late transfer/extra-namespace rejection also runs with the heap held;
  no member publishes and ordinary cancellation restores original baselines.
- Existing detached backing controls now check shared authority remains charged
  after extraction and across exact root retirement/slot reuse. Confirmed
  physical release returns its one original authority count; guarded abandonment
  retains it. Existing intentional failed physical release likewise retains its
  payload/authority node inside the complete object. No new data-page quarantine
  or budget reset is added; existing quarantines now retain more metadata/counts.
- Existing loan, DMA/copy pin, failed mapping/invalidation/scratch, hidden
  delivery, returned-capability, cancellation and scheduled endpoint/namespace
  controls remain enabled. The common collection's host tests separately check
  actual node deallocation before charge release and inert abandonment.

Final native runs pass on both architectures: **AArch64 20 passed / 0 failed /
0 pending**, and **x86_64 18 / 0 / 0**. Each completes all **14 HTTP checks**
and both security-service generations. Healthy responses beside stalled peers
take **0.207–0.397 s** on ARM and **0.389–0.824 s** on x86. Trickle checks
reject after **30.14 s / 30.27 s**; cancellation pressure retires after
**19,980 / 13,328 requests**, respectively.

Host retries remain separate observations. Each final run records an initial
metrics `TimeoutError` and four `ConnectionResetError` retries. Subsequent
attempts pass within the existing retry/deadline limits. No limits were widened;
these observations provide no retrospective causal explanation.

All **16 shared collection host tests** pass. Strict base-kernel Clippy passes
on both architectures. Workspace formatting, whitespace and local reference
checks pass. Bundled services are reused; userspace sources and the tracked
manual PDF remain unchanged. Both native runners retain assembly-section checks.

Both earlier images also pass their native suites and all 14 HTTP checks. Final
review then corrected cancellation field order and refreshed the borrower-control
marker; both architectures were rerun for that final source. Earlier artifact
identities and results remain separate from final-source validation. No native
failure occurred in this batch. Passing executions do not attribute historical
mapping panics, root-close timeouts or network startup/probe intermittence.

## Formal qualification and remaining work

All **20 focused cases** pass: three safe configurations and 17 expected
counterexamples. Required safe actions execute; negative cases violate their
named invariant through the intended unsafe action.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |

No model/configuration changes. Existing atomic authority/delivery, rollback,
non-terminal timeout, exact-root, physical-once and original-charge projections
remain. Concrete candidate construction, field destruction order, payload/node
free-before-refund and complete heap-held publication are native/host
qualification. The earlier full 190-case checkpoint remains separate; no
executable refinement or sustained-pressure fairness proof is claimed.

Mapping/scratch metadata and source-escrow preparation/destruction still have
separate allocating or guarded boundaries. Original private backing rollback,
unused preparation disposal and wider enclosing IPC/IRQ/destructor contexts
remain unqualified. General metadata bytes, allocator overhead and aggregate
principal ceilings remain incomplete. Capability/namespace lookup, sorted
insertion and repeated object/namespace scans are now linear; existing count
limits and these serialized tests do not qualify worst-case hold time,
contention or sustained management/timer/teardown progress.
