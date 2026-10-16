# Detached completion metadata ownership

Date: 2026-10-16. Baseline `56b2cc3f`, following backlog charge refunds.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial; numbered
findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical receipt or recovery custody.

## The captured boundary

The [preceding batch](2026-10-16-completion-backlog-refunds.md) preserved an
Intel deadline with all four CPU stacks. A heap-held fixture requested the
completion registry while another CPU held that registry and allocated a
`BTreeMap` node for detached submission. Moving the fixture before concurrent
boot work removed its reverse lock edge; production map allocation remained
explicitly open. This batch removes that specific detached-record insertion
and removal allocation boundary. Other completion maps still need conversion.
The earlier capture and failed result remain unchanged historical evidence.

## One admitted node through delivery and disposal

The existing per-namespace detached map now uses the shared
`retirement_list::AdmittedMap`. Preparation fallibly allocates one empty
`PreparedEntry` outside `COMPLETIONS`, alongside the original record charge,
exact root descriptor, account/classification, CQ source, and optional prepared
timer node/cancellation/observer. There is no second identity allocator or
per-operation registry. The existing record admission bounds live/prepared
records; this is not aggregate heap byte accounting.

Borrowed publication revalidates the same root, accounts, capacity, retirement
and exact CQ source before relinking the prepared node. Sorted lookup and
insertion are linear scans bounded by the namespace's existing 1,024-record
limit. Empty-map insertion and removal do not allocate or free. Ordinary
construction/publication rejection explicitly finishes the complete preparation
after local guards; abandonment retains every field without cancellation,
refund, field destruction or allocation. Accepted timers still enqueue once,
with their original event deadline and close-before-enqueue cancellation.

Terminal publication detaches the complete admitted node under the same hold
as result publication. The existing terminal receipt carries timer observer,
cancellation, captured CQ source, any direct-result node and flushed-node
refund batch. Its fallback retains all those fields. Explicit finish leaves
local registry guards first, frees the direct node, disposes timer fields and
returns the original record charge before exact-source notification. Duplicate
completion still rejects and cannot adopt or replay the node.

A full CQ must retain **the admitted node and its original record charge** in
its backlog. Moving only the charge there would allow a flush to refund it
while a terminal metadata node remained retained elsewhere. Timer fields leave
in the terminal receipt; the backlog keeps the node with its remaining scalar
record and charge. Successful flushing moves that same node into the refund
batch's shared retirement list and coalesces its charge by original account and
ordinary/platform class. No disposal allocation is needed. After unlock the
batch releases every detached node before its two checked charge refunds.
Failed writes retain the unchanged entry/node/charge; full FIFO flushing and
logical slot updates remain serialized. Queue disposal explicitly drains
remaining backlog nodes after its local registry hold.

A complete `AsCompletions` owner now retains the namespace's capability map,
detached map, all CQs/backlogs, sources, original accounts and captured root
descriptor on abandonment. Ordinary close/replacement explicitly revokes
published authority under the existing serialization, then finishes that owner
after unlock: cancel public subscriptions, drain detached nodes and dispose
remaining queue/backlog payloads and accounts. Unpublished namespace preparation
has the same complete retaining fallback and an explicit ordinary finish;
constructor and publication failures use it. This prevents the admitted map's
inert Drop from silently leaking ordinary closes while sibling fields are freed.

The root descriptor is not a new operation lease or physical pin. Retention
is terminal abandonment, not Pending, retry custody or physical quiescence.
Existing authority revocation and CQ/global/capability-map metadata remain
separate local boundaries. Wider caller/IRQ, active timer queue and allocator
contexts are not qualified by merely leaving `COMPLETIONS`.

## Native controls and their limits

The synchronous probes run before the AP scheduler barrier is released, so a
heap-held probe cannot introduce the concurrent reverse-order fixture cycle:

- Inject detached-node allocation rejection while reentering `COMPLETIONS`.
  The original provisional charge is live during allocation and returns after
  ordinary rejection, with no inserted record. Successful node construction
  also reenters the registry.
- Prepare 64 alternating plain/timer records, then publish all 64 into a fresh
  detached map with the heap locked. Detach all 64 through production terminal
  publication under a second heap hold. Records/32 timers remain charged until
  post-guard explicit disposal; the final full-ring backlog node stays charged
  until actual FIFO delivery and node release. Duplicate completion rejects.
- Close and replace namespaces containing live timers and retained backlog
  nodes. Original record/timer/CQ accounts reach zero; the successor stays at
  its own zero record usage. Existing CQ replacement, physical-frame alias
  rejection and rejected whole-namespace preparation controls still apply.
- Drop complete published and unpublished namespace preparations under both
  heap and registry guards. Separately abandon plain/timer preparation and
  direct/flushed terminal receipts under those guards. Original nodes, charges,
  timers, CQ backing, watch subscription and buffer remain; successor accounts
  receive no credit, callbacks/destructors do not run and IRQ state is restored.

The deliberate abandonment controls use private raw namespaces, never a real
user root or substituted live sponsor. They permanently retain a bounded
**seven record charges, three timer events and two CQ payload charges**, plus
their original metadata/list/watch references until guest shutdown. These are
negative cleanup controls, not successful reclamation or production custody.
Accounting checks use the actual retained original accounts; other fixtures
measure their own baselines. This fixture cost must not be mistaken for a
production leak or erased by resetting node counters.

Existing tests cover stale/replaced/missing/retired publication, event/observer
allocation failure, actual timer firing/cancellation, direct/backlog/missing-CQ
terminal paths, weak/source identity, ordinary/platform charge aggregation,
namespace generation reuse and post-guard callback reentry. Mapping failures
are unchanged CQ/frame controls; no new user mapping or DMA path is introduced.

## Execution results

The host runner passes **499 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus its signing/tooling/Python controls. Strict kernel
Clippy passes both custom targets with `-- -D warnings`. Workspace formatting
and diff whitespace checks pass.

All five native profiles use four LPs, fresh storage, a 180-second guest budget,
denied Rust warnings, assembly-section checks and automatic all-CPU failure
capture. Each returns runner exit zero and diagnostic result `complete`:

| Profile | Detached metadata fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 2.325614 s | 17 passed, zero failed/pending at 11.784489 s |
| Intel VT-d, no network | Passed at 1.389924 s | 15 passed, zero failed/pending at 14.460547 s |
| AMD-Vi, no network | Passed at 1.421687 s | 15 passed, zero failed/pending at 19.954084 s |
| ARM / SMMUv3, security | Passed at 1.742820 s | 19 passed, zero failed/pending at 56.394420 s |
| Intel VT-d, security | Passed at 1.312675 s | 17 passed, zero failed/pending at 61.352260 s |

Together these execute **320 fresh heap-held detached publications and 320
heap-held removals**, ordinary namespace drain/replacement, both guarded
retention controls and existing original-account refund probes. Each also
completes 64 scheduled IPC close/exit episodes, 320 total, with exact exit
watches. Both security profiles complete two scoped EL0 generations; concurrent
cancellation traffic retires after 14,432 requests on ARM and 14,144 on Intel.
These are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration.

No failed native sample, panic, deadline or persistence fail-stop was observed
in this matrix. The preceding batch's two failed Intel samples remain separate
published evidence and do not change disposition. Runtime implementation was
unchanged throughout this matrix. After the no-network kernels were built, a
comment and targeted Clippy allowance were added only to the intentional
unpublished-namespace Drop control: the inert `ManuallyDrop` wrapper is exactly
what that negative control must drop. The three no-network profiles use that
pre-annotation source variant; security profiles use the final source. Both
variant hash sets are recorded. This qualification avoids representing earlier
kernels as builds of later text.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance detached-map-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance detached-map-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance detached-map-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance detached-map-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance detached-map-intel-security
```

AMD sets `CATTEN_SKIP_KERNEL_BUILD=1` after Intel finishes, using its identical
no-network ELF. Security runs additionally set `CATTEN_ALLOW_SANDBOX_NETWORK=1`
and the HTTP/deployment port inputs 19080/19081 for ARM and 19180/19181 for Intel;
actual network forwards are included in evidence. All staged service files and
run inventories match the preceding baseline byte for byte. No services,
shared runtime libraries, syscall ABI or wire formats change.

[Version-controlled evidence](evidence/2026-10-16-detached-completion-metadata.json)
records frozen Rust hashes including the new fixture, commands, unchanged
service inventories, kernel/boot-image/initial-disk/final-UART identities and
concise observations. Published results and evidence do not depend on ignored
build artifacts or temporary logs. Documentation was finalized afterward; this
is not a full-tree or physical-platform attestation.

## Remaining work

Capability-record and CQ/global namespace maps still allocate/free metadata
under existing guards. Authority revocation, capability close/terminal timer
cleanup, wider queue/producer/IRQ contexts, aggregate heap/principal admission
and integrated pressure/progress remain open. Only the detached map has been
converted; the production completion registry is not allocation-free. The
serialized fixture must not move back into concurrent boot work on that basis.
Historical IPC exact-worker exit, stack, persistence and NIC incidents remain
unattributed. No polling, forced IRQ enabling, readiness source, physical release
policy or replay permission changes.
