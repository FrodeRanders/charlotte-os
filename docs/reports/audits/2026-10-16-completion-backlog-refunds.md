# Completion backlog refunds and captured fixture failures

Date: 2026-10-16. Baseline `2fbe5137`, following exact timer-cancellation
identity. This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
the numbered findings remain 29 corrected within scope, six partial and three
open. No new cleanup family, registry, recovery owner or physical proof.

## Delivered charges leave the local registry guard

A detached completion retains its record charge and submission slot while its
result waits in a full CQ's non-lossy backlog. Previously, successful backlog
flushing dropped that charge under `COMPLETIONS`, acquiring the original domain
and node budget locks there. This affected admission, observation, CQ waiting
and both capability-backed and detached producers. A terminal record already
had an owning handback, but previously delivered backlog charges did not.

Successful ring publication now moves each detached charge into `Releases`.
It holds at most two inline charges, separated by their original ordinary or
platform classification. Charges coalesce only when their retained domain Arc
matches exactly. The first owner keeps that allocation alive while redundant
references drop; collecting a charge neither refunds nor takes budget locks,
allocates, or frees the account. A namespace's classification can change while
older records remain ordinary, so the batch must preserve both classes.

The private record counters use the existing checked `VectorBudget<1>` rather
than a loop of scalar refunds. Limits remain 1,024 per domain, 8,192 per node
and 6,144 ordinary node records. Each retained class refunds its count to the
original domain/node accounts after local registry unlock. No shared lifecycle
API, service source or service binary changes.

Admission and observation declare the refund owner before their registry guard,
including error paths. CQ waits dispose it before preparing or parking on their
captured interest. Capability producers refund before transition destruction
and notification. Detached producers carry it in their existing terminal
receipt, disposing it before the record and exact-source notification.

All entries that fit still flush in FIFO order during the same registry hold.
Only successful detached publication releases a logical submission slot;
failed publication requeues the unchanged entry and charge. The refund remains
charged until post-guard disposal. Numeric ASID reuse cannot credit a successor
account. This is logical charge disposal, not a root pin, hardware completion,
retry owner, or qualification of arbitrary outer guards and IRQ contexts.

## Native controls

The synchronous pre-scheduler fixtures exercise the real implementation:

- Collect 64 charges across both classes while holding the original domain,
  node and heap locks. The batch preserves 32 of each without refunding there;
  post-guard disposal restores both original totals.
- Complete 128 detached records against a 64-entry ring. A full ring flushes
  nothing. After draining its 63 usable entries, one flush publishes all 63
  entries that fit, in order, while the completion registry/domain/node/heap
  guards are held. Logical slots decrease immediately while original charges
  remain. Close and replace the namespace before dropping the batch: only the
  predecessor account receives the delayed refund.
- Exercise both producer paths with an existing detached backlog. Return from
  publication sees the earlier delivered charge refunded; capability records
  remain charged through explicit close.
- Reject plain and timer capability publication after a real backlog flush by
  removing its hidden authority namespace. The delivered charge refunds after
  unlock; the rejected operation and timer stay completely owned until explicit
  unstarted rollback.

Existing controls retain mapping/preparation rejection, full CQ/backlog behavior,
terminal cancellation, replacement identity, completion/watch admission and
ordinary/platform exhaustion. No synthetic maximum heap allocation or general
hostile-pressure guarantee is claimed.

## Two failed Intel samples, kept separately

The first Intel no-network run panicked in the IPC claim fixture at its
registration-implies-Blocked assertion, during round 4, producer LP1/worker LP3.
All four CPU stacks were captured. Source shows that scheduler preparation
registers interest before publishing the worker's Blocked state. That assertion
therefore accepted an observation that did not establish its next premise.
The assertion did not log the exact sampled state or worker generation, so the
later capture cannot reconstruct which non-Blocked state occurred at that instant.

The corrected fixture observes registration and Blocked state for the exact
worker generation under one original five-second deadline, with separate short
guard holds. Its bounded one-millisecond sampling is test-only observation of
park publication. It never releases the held claim early or substitutes for the
mandatory exact-generation exit completion before roots close. Timeout diagnostics
now include TID, generation, state and fixture progress. A deterministic scheduler
control keeps a real registered thread Ready before Blocked publication, directly
exercising the legitimate window; spontaneous sampling need not hit it.

A subsequent Intel run, with that correction and the final private counter
implementation, reached its **180-second deadline**. The all-CPU capture shows:

| CPU/debugger thread | Captured path |
| --- | --- |
| LP0 / #1 | Heap-held CQ waiter fixture requests `COMPLETIONS.read` in `CqInterest::publish_waiter`. |
| LP1 / #2 | Detached publication holds `COMPLETIONS.write`; `BTreeMap::insert` requests an 896-byte heap allocation. |
| LP2 / #3 | Idle loop. |
| LP3 / #4 | Another heap allocation waiter in IPC capability-record preparation. |

The fixture acquired heap then registry after concurrent boot work had started;
the producer held registry then needed heap. This is a concrete captured lock
cycle. The heap-held fixture now runs with the other completion admission probes
in the synchronous phase, before the AP scheduler barrier is released. Its
success/full/ready/stale-source checks and heap hold remain intact. Private ASIDs
alone do not isolate shared registry and heap locks. Scheduled device and CQ
wake tests still run concurrently afterward.

This removes the fixture's reverse-order edge. **Production map allocation
under `COMPLETIONS` remains open**, now supported by a live stack. Moving one
insertion alone would not qualify other allocating writers or namespace/map
retirement. This batch does not claim that the production completion registry
is allocation-free.

The [panic excerpt](evidence/2026-10-16-backlog-refund-ipc-failure.txt) and
[heap-cycle excerpt](evidence/2026-10-16-backlog-refund-heap-cycle.txt) preserve
relevant UART lines, all four CPU stacks, artifact hashes and interpretation
limits. The first kernel used an intermediate counter implementation; a complete
per-file manifest for it is unavailable. Its unchanged baseline fixture hash
and binary/capture identities remain recorded. The second failure has per-file
Rust hashes, including the untracked backlog fixture. Passing ARM samples
before the second correction are recorded as intermediate runs, not evidence
that the unsafe fixture ordering was acceptable.

Neither failure establishes the cause of historical exact-worker exit timeouts,
stack retention, persistence fail-stops or NIC incidents. Production IPC and
scheduler behavior are unchanged. Readiness, worker-return markers and sampled
states remain distinct from exact exit and physical quiescence.

## Execution results

The final source passes **499 host Rust tests, zero failed, four ignored**
across 57 harness summaries, plus the host runner's signing/tooling/Python
controls. Strict kernel Clippy passes for both custom targets with denied Rust
warnings. Workspace formatting and diff whitespace checks pass.

All five final profiles use four LPs, fresh storage, a 180-second guest budget,
assembly-section checks and automatic all-CPU failure capture. Runtime Rust was
frozen before this matrix. Each runner exits zero with diagnostic result
`complete`; each executes the deterministic registered-but-Ready control and
the relocated heap-held CQ waiter controls before scheduled boot work:

| Final profile | Backlog refund fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.738850 s | 17 passed, zero failed/pending at 11.696900 s |
| Intel VT-d, no network | Passed at 1.235292 s | 15 passed, zero failed/pending at 14.193940 s |
| AMD-Vi, no network | Passed at 3.126163 s | 15 passed, zero failed/pending at 18.457865 s |
| ARM / SMMUv3, security | Passed at 1.293093 s | 19 passed, zero failed/pending at 55.814229 s |
| Intel VT-d, security | Passed at 1.225226 s | 17 passed, zero failed/pending at 61.581727 s |

The no-network Intel and AMD runs use the identical kernel ELF. All five
complete 64 scheduled IPC close/exit episodes apiece, **320 total**, retaining
exact exit watches. Their spontaneous pre-Blocked sample counts are zero;
the deterministic Ready control supplies the direct negative observation.
Both security runs complete two scoped EL0 security generations. Concurrent
cancellation traffic retires after 14,704 requests on ARM and 14,088 on Intel.
These are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration. No further panic, timeout or persistence fail-stop
was observed in this final matrix. That does not erase the two failed samples
above or establish historical failure causes.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance backlog-refund-arm-qualified
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance backlog-refund-intel-qualified
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance backlog-refund-amd-qualified
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance backlog-refund-arm-security-qualified
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance backlog-refund-intel-security-qualified
```

AMD additionally sets `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes.
Security runs set `CATTEN_ALLOW_SANDBOX_NETWORK=1`; effective deployment host
forwards are 8081 for ARM and 19181 for Intel. Exact supplied environment and
QEMU network arguments are recorded in the JSON. The service bundles match the
preceding baseline byte for byte, verified against every staged file and each
run's inventory. Services were neither edited nor rebuilt.

[Version-controlled evidence](evidence/2026-10-16-completion-backlog-refunds.json)
records final reviewed Rust hashes, commands, complete unchanged service
inventories, artifact/UART hashes, concise observations and both failed samples.
Documentation was finalized afterward. Inline results and checked-in evidence
are sufficient to review this report; temporary logs and ignored build output
are not its published evidence. This is not a full-tree or physical-platform
attestation.

## Remaining boundary

Next is completion map metadata: detached/capability record insertion/removal,
CQ/global namespace nodes and complete namespace teardown require preparation
and owning post-guard retirement together. The shared admitted-map machinery is
available, but partial conversion must not lose records, callbacks, timer state,
original charges or fallback ownership. Existing backlog storage mutation and
namespace destruction, terminal capability timer cleanup, timer queue/outer
producer guards, aggregate heap admission and integrated progress remain open.
No new polling, IRQ enabling, readiness contract, custody or replay policy.
