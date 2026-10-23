# Security audit: bounded root-record storage

Date: 2026-10-24. Reviewed baseline: `5a118028`.

Launch-limit, usage/telemetry and signed-authority nodes now share **4 MiB /
12,288 nodes** of requested allocation admission. Ordinary preparation is limited
to **3 MiB / 9,216 nodes**. Each root needs three independently owned nodes;
partial preparation also consumes the pool until actual disposal.

The [tracked evidence](evidence/2026-10-24-security-root-metadata-admission.json)
records source/model/artifact identities and selected results. This extends
SEC-07 R7-2/R7-3 through these root metadata layouts. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07 and SEC-18 remain partial and
production trust remains disabled.

## Admission and ownership

Each typed node reserves its exact requested layout and one count before its
fallible allocation. Classification uses the existing trusted value captured
from the unpublished root's translation account. No name, role, artifact class
or manifest field supplies a reserve claim. Later promotion leaves each node's
original classification and eventual refund unchanged.

The existing `PreparingMetadata` owns all partial fields before publication.
Quota/allocation rejection explicitly cancels unused storage without publishing
an ASID. One or two remaining slots cannot produce partial root publication.
Successful publication relinks the same charged nodes into the existing maps
without allocation or counter entry.

Sealed close detaches all three nodes into its existing root receipt. Failed
invalidation retains nodes and original charges. Confirmation permits explicit
payload/node destruction, then refund. This precedes the existing physical root
walk; a later physical failure cannot resurrect already-freed metadata, and its
separate backing charge remains governed by that owner. A losing principal-history
candidate keeps its own independent node charge inside the authority payload.

Unused preparation and confirmed disposal keep their existing post-guard
requirements. Abandonment retains all remaining fields without allocator or
counter entry. The new counter is an independent leaf; it leaves before allocation,
performs no root/registry lookup and cannot redirect refund through ASID reuse.

These are the existing C02/C03 owners. No root/pin, registry, custody, recovery
family, physical proof or retry permission is added. The
[cleanup rows](../../reference/cleanup-recovery.md),
[policy](../../reference/root-metadata-admission.md),
[lock order](../../reference/locking.md) and Markdown/LaTeX contracts are updated.

## Verification

Serialized kernel controls exercise real node allocations and root registration:

- **192 allocator rejections**, 64 for each layout, restore the starting
  reservation. The existing 320 preparation-stage rejection controls also check
  metadata counter restoration.
- Real ordinary saturation reaches **9,216 nodes**; 64 further attempts reject
  before invoking the node allocator. Ordinary registration rejects before ASID
  publication, with unchanged node reservations, root count and physical frames.
- With only **one or two slots free**, registration allocates its available
  prefix, rejects the incomplete transaction and returns every partial node's
  original bytes/count. No partial authority or root becomes visible.
- A trusted architecture constructor still registers and closes a complete root
  from reserve. Promoting an existing ordinary root changes no metadata charge.
- Real total saturation reaches **12,288 nodes** and rejects even trusted node
  preparation. Closing an existing root needs no fresh storage and returns its
  three original ordinary charges. Another ordinary root then registers/closes;
  fixture disposal restores the full starting byte/count reservation.
- ARM uses **9,153 ordinary + 3,066 reserved** fixture nodes; x86 uses
  **9,129 + 3,066**. Earlier occupancy differs between architecture fixtures;
  the policy ceilings are identical and were not widened.
- Publication runs while the heap and new admission counter are held, without
  allocation/refund. Guarded partial-owner abandonment also holds the counter
  and retains storage. Detached-root invalidation failure retains all three
  charges; confirmed release returns exactly their original bytes/count while
  an intervening root remains admitted. Existing generation-reuse checks remain.

Both first combined native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**.
Each completes two scoped security generations and all **14 host HTTP controls**.
Healthy responses beside stalled peers take **0.254–0.371 s** on ARM and
**0.338–0.406 s** on x86. Trickled deployment bodies reject in **30.15 s** and
**30.26 s** respectively; cancellation traffic retires after **20,256** and
**15,876** requests. No guest repeat, quota widening or deadline change was used.

All **15 collection host tests** pass, including charged-map detachment and
actual payload/node destruction before refund. Strict base-kernel Clippy passes
on both architectures. Workspace formatting, whitespace and shell syntax checks
pass. Userspace source and embedded bundles are unchanged; the checked-in manual
PDF is preserved.

## Formal verification

The complete expanded harness passes **47 safe configurations and 143 expected
counterexamples across 44 modules: 190 cases and 235 checked inputs**. This closes
the preceding history batch's full-suite verification gap. The command was:

```sh
JAVA_TOOL_OPTIONS=-XX:ActiveProcessorCount=2 docs/tla/check.sh /path/to/tla2tools.jar
```

The processor limit bounds Java workers; no invariant, configuration or required
action coverage was changed. Models/configurations are unchanged in this batch;
the full run verifies the registered expanded suite.

`CharlotteRootRetirement` already projects metadata retention through rejected
invalidation and confirmation before release. The three-node byte/count pool and
partial-allocation rollback are not explicit in that model. Those implementation
boundaries are qualified by the native controls and common charged-collection
host tests. The full formal run does not provide executable refinement, allocator
byte accounting, hardware/IRQ proof or sustained progress guarantees.

## Remaining work and observed boundaries

The pool covers these three requested node layouts, including partial preparation.
It excludes allocator overhead/rounding, namespace/account control blocks,
waiter/timer sponsors, observer/weak-reference backing, other vectors and the
complete physical heap ledger. Other resource families retain their own limits.
No one essential service has a guaranteed share of the common reserve.

A concrete remaining IPC boundary is `AsIpcCaps::new` and `IpcRegistry::as_caps`
in `crates/catten/src/ipc/mod.rs`: namespace creation is lazy under the IPC
registry; both endpoint and record-budget constructors use infallible `Arc::new`.
The namespace/capability `BTreeMap` paths also need prepared publication storage.
This requires a coherent namespace-publication change; changing only the Arc
constructor would leave allocation and disposal under the same guard. It is
recorded here as open R7-2/R7-3 and outer-context work, without a new finding count.

Concurrent hostile root preparation, long-held roots, allocator/lookup/IRQ hold
cost and sustained management/timer/teardown fairness remain unqualified by
serialized exhaustion. The earlier x86 queued-loan mapping panic remains
causally unresolved in the [mapping report](2026-10-23-security-map-admission.md).
These runs supply no retrospective attribution or recovery authority.
