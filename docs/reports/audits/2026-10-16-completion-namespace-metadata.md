# Global completion namespace metadata

Date: 2026-10-16. Baseline `223e7774`, following CQ metadata.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gap and scope

The preceding batches prepared and retired admitted nodes for completion
capabilities, detached operations and CQs. The global `COMPLETIONS` namespace
map still used `BTreeMap`: fresh insertion could allocate while holding the
registry, and close/removal could free metadata before returning the payload
for disposal. The namespace's four original account allocations were also
infallible, even through runtime CQ setup returning `CqOpenError`.

These are source findings. The historical captured heap/registry cycle involved
another writer, removed in the detached-map batch. This batch is not attribution
of that failure or of unresolved IPC exit, stack, persistence or NIC incidents.

The global map now uses the existing shared `AdmittedMap`. Its ordered lookups
are linear in live namespaces. This introduces no separate aggregate bound for
namespace nodes or account allocations, and queue/record limits do not bound
empty namespaces. There is no new worst-case global scan-latency or hostile
pressure guarantee. This is local fallible storage preparation and complete
ownership; aggregate admission remains open.

## Complete preparation, publication and retirement

`namespace_preparation::Preparing` captures the current root identity once,
before allocating. One inert owner retains the empty global node and complete
private namespace, including its record, timer, CQ and watch accounts. The
node uses `PreparedEntry::try_new`; all four account constructors now have
`Arc::try_new` paths. An allocation-stage failure explicitly frees unused
metadata and partial account construction outside local registry guards, without
changing the old namespace. Partial unused account Arcs contain no published
payload, source or authority. Ordinary unused complete preparation explicitly
finishes; abandonment retains every field without cleanup.

CQ namespace preparation contains this same owner alongside its existing queue
preparation. Node/account failure propagates as `CqOpenError::AllocationFailed`
through the loader's existing typed error and rollback path. Bare kernel/demo
and mandatory serialized fixtures share the same preparation through a panic
wrapper; no alternate infallible map-allocation path remains. The existing
private bare-payload fixture wrapper also delegates to fallible account
construction. Other mandatory timer/watch/account convenience constructors
retain their existing caller policy through the same `try_new` implementation.

Publication borrows preparation under `COMPLETIONS`, validates the captured
generation/retirement, then relinks prepared metadata. CQ setup also preserves
its original account/capacity and physical-alias checks before initialization.
After a physical-ring write there is no later fallible admission: the shared
relink step assumes that validation under the same registry hold. No error is
newly returned after physical initialization. The captured identity is not a
root lease; caller-owned frame/mapping and quiescent-consumer requirements
remain unchanged. No ABI, wire format, deadline or notification producer changes.

Replacement and close detach the global node without destruction. Existing
authority revocation stays serialized and retains each detached authority in
its original capability payload node. `RetiredNamespace` owns the complete
global node and namespace, including sources, timer/cancellation fields, ring,
backlog, record/authority nodes and original accounts. Dropping it retains all
of these fields through the shared retired-node fallback.

Explicit post-registry finish uses the existing typed handoff to keep the
complete namespace alive across actual global node deallocation. It then
closes sources and drains payload nodes, preserving their existing original
charge and final strong/weak backing contracts. Global metadata frees before
payload/account disposal. Numeric ASID or capability reuse cannot redirect
that disposal into a successor. This is unstarted rollback or terminal
retention, not Pending, owner-preserving retry, physical release authority or
production recovery custody. Outer caller/IRQ guards and terminal timer/observer
destructors remain separately qualified.

## Controls and deliberate retention

Native heap-held controls remain in the synchronous phase before AP schedulers
start. Removing this final completion map allocator does not qualify unrelated
allocating writers or authorize moving those probes into concurrent boot work.

- Inject rejection immediately before each of the five allocation stages:
  global node, timer account, record account, CQ account and watch account.
  Each boundary reenters `COMPLETIONS`; the live namespace, in-flight buffer,
  authority and original charges remain unchanged. Successful construction
  also reenters at all five boundaries. These are injected stage rejections,
  not physical allocator exhaustion or complete heap-leak measurement.
- Prepare 64 real private namespaces, each containing a charged heap CQ.
  Publish all 64 fresh global nodes and remove all 64 under a heap hold.
  Original queue charges remain live until explicit post-guard namespace
  disposal, then return to the baseline without counter resets.
- Replace one namespace containing an in-flight buffer, a far-future timer,
  retained detached backlog, event watch and CQ waiter. Revoke authority under
  the heap hold without disposing payloads. Read the old ring through its sole
  private consumer while the retired owner retains it. Reuse the numeric
  capability before old disposal; only original accounts refund, and the
  successor's queue, record and waiter remain intact. An event-observer
  destructor reenters `COMPLETIONS` during explicit disposal after unlock.
  A retained old weak completion keeps its original record admission until
  final weak release; no fabricated completion/readiness is delivered.
- Prepare an empty namespace against a real user root, close that root and
  force the same numeric slot to a new generation. Publication rejects the
  captured generation under the heap hold without replacing or crediting the
  successor. Explicit unused preparation disposal releases its original account.

The existing guarded unpublished-CQ namespace control now also checks weak
references to all original accounts after abandonment, and the existing
guarded retired detached-namespace control now retains the actual global node.
The latter still checks buffer/watch/backlog/authority retention and numeric
successor independence. No additional charged negative fixture is created:
combined completion metadata controls still deliberately retain **ten records,
four timer events, four CQ payload charges, two watch charges and three authority
charges**, plus original nodes/list/buffer references. This batch additionally
retains the two global map allocations in those existing negative controls;
unpublished private account storage was already retained. These private raw
namespaces use no actual user root, live sponsor substitution, counter reset or
production retry custody. All retention ends only at guest shutdown.

The shared allocator-traced host handoff control from the capability metadata
batch checks payload lifetime across actual node deallocation without allocation
during handoff. Existing native partial CQ failure, unchanged physical bytes on
alias/retirement rejection, cancellation, full-ring abort, absolute deadlines
and returned-resource controls continue to execute.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Each profile uses four
LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Global namespace metadata fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.927942 s | 17 passed, zero failed/pending at 11.884473 s |
| Intel VT-d, no network | Passed at 1.340425 s | 15 passed, zero failed/pending at 18.732913 s |
| AMD-Vi, no network | Passed at 1.772830 s | 15 passed, zero failed/pending at 18.671814 s |
| ARM / SMMUv3, security | Passed at 1.635584 s | 19 passed, zero failed/pending at 56.920259 s |
| Intel VT-d, security | Passed at 1.318627 s | 17 passed, zero failed/pending at 61.677684 s |

Together these exercise **320 fresh heap-held global publications and 320
global-node removals**, 25 allocation-stage rejection controls, complete
replacement, reentrant disposal and real generation reuse. Each also completes
64 scheduled IPC close/exit episodes with exact exit watches, 320 total.
Both security profiles complete two scoped EL0 generations. These are bounded
observations, not sustained fairness or hostile-pressure qualification. ARM
retains certified runtime migration; x86 qualifies only never-dispatched
migration. No failed native sample, panic, deadline or persistence fail-stop was
observed in this matrix. Prior failed runs remain separately preserved in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md); these passes
do not establish their causes.

Commands share `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance completion-namespace-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance completion-namespace-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance completion-namespace-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance completion-namespace-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance completion-namespace-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel. Exact environment and actual network forwards
are recorded in evidence. Staged service files and every run's bundle inventory
match the preceding baseline byte for byte; no service/runtime library changed
or required rebuilding.

[Version-controlled evidence](evidence/2026-10-16-completion-namespace-metadata.json)
records reviewed hashes for all 12 Rust files, including the two new namespace
files, commands, artifact/UART hashes, unchanged inventories and concise
observations. Frozen source hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

All four completion maps now use prepared shared admitted nodes. Aggregate
namespace/account/heap/principal admission, integrated pressure/progress and
worst-case global scan latency remain open. Timer queue node/observer disposal,
other authority consumers' enclosing guards and the full outer IRQ/caller
inventory still need qualification. The historical IPC exact-worker exit,
stack, persistence and NIC incidents remain unattributed. No polling, forced
IRQ enabling, root pin, physical retry, readiness source or custody policy changes.

A concrete next source boundary is `timers::queue::Events::retain`: keyed
removal, eager exact-identity cancellation and cancelled-event purging still
destroy removed event nodes and their observer fields under the timer queue
borrow. This remains separate from preparing the namespace accounts or freeing
completion map metadata after local unlock. Moving that disposal will need to
preserve cancellation identity, original event/node backing admission, comparator
updates and the enclosing IRQ policy; no historical cause is inferred here.
