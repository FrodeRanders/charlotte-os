# Completion-queue metadata

Date: 2026-10-16. Baseline `e3ce31c2`, following capability-backed completion
metadata. This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gap and complete ownership

CQ preparation already allocated its charged ring/backlog and waiter source
outside `COMPLETIONS`. A fresh queue still allocated its `BTreeMap` node under
that lock, and replacement freed metadata there. Returning just `CqState` did
not retain the map allocation. This is a source finding; no historical panic
or timeout is attributed to this particular node operation.

The per-namespace queue map now uses shared `AdmittedMap` storage. The existing
queue preparation reserves its original count/backing admission, then allocates
one empty `PreparedEntry` fallibly before waiter-list and ring preparation.
Metadata rejection cannot publish a queue or touch a physical ring. All
constructor rejection explicitly finishes the complete preparation outside
local registry guards, freeing unused metadata before its provisional or
payload charge. The existing queue limits are unchanged; the count bounds one
prepared node per admitted queue, while the byte dimension still measures
ring/backlog storage, not aggregate metadata bytes.

Borrowed publication validates captured namespace root, original account,
capacity and retirement, checks physical aliases, initializes a borrowed frame
only after those checks, and relinks the prepared node. Queue lookup and sorted
insertion scan at most the existing 32-queue namespace limit. No ABI, wire
format, deadline, notification producer or identity allocator changes.

Replacement detaches the entire old node into `RetiredCq`, retaining its source,
ring, backlog and original charge. Logical detached backlog slots are returned
under serialization; the original record charges remain with those backlog
nodes until actual disposal. Explicit finish uses the shared typed handoff to
hold `CqState` across metadata deallocation, then closes its source and destroys
backlog/ring storage before queue admission refunds. Complete namespace close
and replacement explicitly drain the same owners after local registry unlock.

Both unpublished preparation and retired queue fallback retain every field,
including metadata and original admission, without cleanup. Ordinary rejection
is explicit unstarted rollback; abandonment is terminal retention. Neither is
Pending, a public retry owner or recovery custody. An independent waiter token
can still cancel its own subscription, but that cancellation does not dispose
an abandoned ring/backlog or refund their charges.

Physical rings remain borrowed from the original mapping owner. Capturing a
root identity is not a lease or pin; the caller must quiesce its consumer.
Metadata release supplies no physical completion or permission to replay work.

## Controls and deliberate retention

Heap-held controls run in the serialized synchronous phase before AP schedulers
start. The global namespace map remains allocating; this conversion does not
authorize moving those probes into concurrent boot work.

- Inject metadata rejection while reentering `COMPLETIONS`, with the original
  provisional charge live. Verify complete refund and no waiter/ring allocation.
- Prepare 32 real heap queues while successful node allocation also reenters
  the registry. Reject the 33rd before any node/source/ring allocator runs.
  Publish all 32 fresh entries with the heap locked. Remove the global namespace
  outside that hold, then detach all 32 actual CQ nodes under heap and registry
  holds. Charges remain until explicit post-guard release.
- Replace a queue containing a full ring, one retained detached backlog row
  and a live waiter. The old ring remains readable by its sole fixture consumer.
  Reuse the numeric namespace before disposing the old queue; only original
  accounts refund, the old waiter cancels without fabricated readiness, and
  the successor queue and waiter remain intact.
- Abandon one complete unpublished queue and one complete retired queue with
  heap and registry held. Preserve their original queue/record counters through
  namespace reuse. Later independent waiter cancellation releases only its own
  subscription charge; no abandoned queue backing is reclaimed.

The physical-frame control now publishes a fresh CQ node with the heap held,
without pre-inserting a heap-backed queue. Existing alias/stale/retired failures
still leave physical bytes and pending results unchanged. Partial list/ring
failure, source cancellation, full-ring abort cleanup and returned-resource
controls continue to run. The shared allocator-traced host control from the
preceding batch checks that payload ownership outlives actual node deallocation
without allocating during handoff.

These private raw namespaces use no actual user root or live sponsor
substitution. This batch deliberately retains **two queue charges and one
backlog record** until guest shutdown, with their original nodes/list/buffer
references. The independent waiter is later cancelled, so it adds no persistent
waiter charge. Together the completion metadata controls retain **ten records,
four timer events, four CQ payload charges, two watch charges and three authority
charges**. These negative controls are separate from successful reclamation;
no counters are reset to hide retained backing.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus its signing/tooling/Python controls. Strict kernel
Clippy passes on both custom targets with `-- -D warnings`. Workspace formatting
and diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. Every runner
exits zero with diagnostic result `complete`:

| Profile | CQ metadata fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.672096 s | 17 passed, zero failed/pending at 11.210450 s |
| Intel VT-d, no network | Passed at 1.339625 s | 15 passed, zero failed/pending at 14.287190 s |
| AMD-Vi, no network | Passed at 1.346748 s | 15 passed, zero failed/pending at 14.339958 s |
| ARM / SMMUv3, security | Passed at 1.337850 s | 19 passed, zero failed/pending at 55.583262 s |
| Intel VT-d, security | Passed at 1.305118 s | 17 passed, zero failed/pending at 61.477415 s |

Together these execute **160 fresh heap-held CQ publications and 160 CQ node
removals**, plus the replacement/backlog/source, guarded retention and
original-account controls. Each also completes 64 scheduled IPC close/exit
episodes with exact exit watches, 320 total. Both security profiles complete
two scoped EL0 generations. These are bounded observations, not sustained
fairness or hostile-pressure qualification. ARM retains certified runtime
migration; x86 qualifies only never-dispatched migration.

No failed native sample, panic, deadline or persistence fail-stop was observed
in these runs. Prior failed samples remain separately preserved in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md); passing this
matrix does not establish historical failure causes.

The no-network commands use `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance cq-map-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance cq-map-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance cq-map-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance cq-map-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance cq-map-intel-security
```

AMD additionally uses `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their
no-network kernel ELF hashes match. Security runs add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel. Exact environment and actual network forwards are
recorded in evidence. All staged service files and each run's bundle inventory
match the preceding baseline byte for byte; no service/runtime library changed
or required rebuilding.

[Version-controlled evidence](evidence/2026-10-16-cq-metadata.json) records
commands, reviewed Rust hashes including the new fixture, artifact/UART hashes,
unchanged service inventories and concise observations. Published results do
not depend on temporary logs or ignored build artifacts. Documentation was
finalized afterward; this is not a full-tree or physical-platform attestation.

## Remaining boundary

The global `COMPLETIONS` namespace map still allocates/frees metadata under its
existing guard and is the next local metadata boundary. Wider timer/observer
destruction, other authority consumers' enclosing guards, outer IRQ/caller
contexts, aggregate heap/principal admission and integrated pressure/progress
remain open. Historical IPC exact-worker exit, stack, persistence and NIC
incidents remain unattributed. This batch adds no polling, forced IRQ enabling,
root pin, physical retry, readiness source or custody policy.
