# Security model synchronization — 18 October 2026

## Scope and result

Reviewed against implementation `1c6963a2`. The executable formal suite had
last changed on 13 September, while the conformance map had received narrower
implementation updates through 13 October. The security work introduced
ownership and failure phases absent from several existing atomic actions.

This batch adds two focused safety models and eleven required counterexamples:

- [`CharlotteRootRetirement`](../../tla/CharlotteRootRetirement.tla) projects
  live root leases, staged close, complete-owner invalidation retry,
  abandonment, physical release and original backing charges.
- [`CharlotteCheckedWait`](../../tla/CharlotteCheckedWait.tla) projects an
  accepted operation and its buffer borrow through fallible wait admission,
  source episode changes, unrelated wakes, timeout and result observation.

The earlier address-space generation and timed-wake race models remain in the
suite. The new models split additional failure boundaries rather than changing
those earlier contracts. No production Rust changes are part of this batch.

## Source comparison

The root projection follows `AddressSpaceOperation` and `SlotLease`,
`ClosingAddressSpace`, `RetiredAddressSpace`, and the bounded root recovery
registry. Source anchors are
[`memory/operation.rs`](../../../crates/catten/src/memory/operation.rs),
[`memory/retirement.rs`](../../../crates/catten/src/memory/retirement.rs),
[`memory/retirement/recovery.rs`](../../../crates/catten/src/memory/retirement/recovery.rs),
and [`registration/metadata.rs`](../../../crates/catten/src/memory/registration/metadata.rs).
It covers the represented ownership boundaries of C01–C03 in the
[cleanup map](../../reference/cleanup-recovery.md), including retained
mailbox/authority/policy metadata through final invalidation.

A physical release rejection is distinct from failed invalidation. Once final
invalidation succeeds, the physical walk may finish and the software slot may
complete even when rejected backing retains its whole original charge. That
terminal backing has no physical retry owner. The model preserves this
distinction and permits a later generation to use the slot.

The wait projection follows
[`try_block_until_with_constraint`](../../../crates/catten/src/cpu/scheduler/mod.rs),
scheduler [`prepare_wait` / `publish_wait`](../../../crates/catten/src/cpu/scheduler/system_scheduler/wait.rs),
and owned [IPC](../../../crates/catten-rt/src/owned/ipc.rs) and
[completion](../../../crates/catten-rt/src/owned/completion.rs) observation.
Watchdog rejection precedes source preparation; time can advance during
preparation. An ended claim episode wakes its old interest without completing
the enclosing accepted operation. Unrelated notifications and rejected wait
admission can lead to another wait on the same operation and original deadline.
Readiness cannot adopt a returned resource or end the borrow.

Both projections use explicit terminal retention. This is not a claim that
every concrete Rust Drop is inert: runtime owners may perform confirmed
cancellation or follow a domain-abort policy. Those typed policies and their
outer execution contexts remain separate implementation obligations.

## Bounds and regression obligations

The root instance has one reusable numeric slot, two generations, two possible
operation leases, two abstract backing pages and at most two whole invalidation
invocations per generation. The attempt bound abstracts the implementation's
initial invocation plus separately bounded controller attempts; it does not
claim to reproduce the exact eight-entry registry or inner CPU epoch budget.

The wait instance has one accepted request, a two-tick absolute budget, time
through tick three, two source episodes and one abstract returned resource.
It permits time to advance independently of preparation or wake delivery.

Each unsafe configuration enables only its named weakened transition alongside
the safe actions. These are regression obligations for the contract, not newly
observed defects in the current Rust code.

| Model | Unsafe transition | Required violated invariant |
|---|---|---|
| Root | Detach with an active/abandoned lease | `LeasesRetainRoot` |
| Root | Release before acknowledged invalidation | `PhysicalRequiresInvalidation` |
| Root | Retry a partially rejected physical walk | `PhysicalOnce` |
| Root | Reconstruct an abandoned owner | `TerminalAbandonment` |
| Root | Refund a charge for retained backing | `BackingCharged` |
| Wait | Release on waiter admission rejection | `PendingRetainsOwnerAndLoan` |
| Wait | Resubmit an accepted request after timeout | `OneSubmission` |
| Wait | Extend the deadline after a wake | `DeadlineUnchanged` |
| Wait | Adopt a result from a wake alone | `ResultNeedsTerminalProof` |
| Wait | Park after completion without the final recheck | `ParkedTerminalHasWake` |
| Wait | Report admission rejection as an early timeout | `TimeoutRespectsDeadline` |

## Validation

Validation results and checked-input hashes are recorded in the accompanying
[evidence manifest](evidence/2026-10-18-security-model-sync.json).

`docs/tla/check.sh /path/to/tla2tools.jar` exited 0. All 26 safe configurations
finished with zero states left on their queues; all 41 negative configurations
violated the named invariant after exercising the required unsafe action.
Every required safe action had nonzero total successor coverage. Across the
separate safe configurations, TLC generated 77,772,262 states and found
10,054,333 distinct states. These are sums across different models, not a
single composed operating-system state space.

| New safe instance | Generated states | Distinct states | Depth |
|---|---:|---:|---:|
| Root retirement | 3,733 | 2,978 | 33 |
| Checked wait | 2,515 | 866 | 13 |

The two new safe instances also passed individual checks before the complete
suite. No invariant failure was observed in a safe configuration. Bash syntax
and whitespace checks passed. Manual Chapter 21 was compiled twice into a
temporary output directory and its changed model/counterexample table pages
were rendered for visual review; the existing tracked PDF was preserved.

The tools JAR is the CI-pinned TLA+ Tools v1.7.4 release, which reports TLC
2.19, revision `5a47802`. Its SHA-256 is
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.
Local Java is Azul OpenJDK 25.0.3; CI uses Java 17. Action coverage and named
expected invariant failures are enforced by `docs/tla/check.sh`.

Initial invocations of both new models parsed successfully but could not start
exploration in the filesystem sandbox: TLC's worker coordination failed with
`java.rmi.server.ExportException`, caused by `java.net.SocketException:
Operation not permitted`. The subsequent checks ran outside that sandbox.
Those initial environment failures are separate from model-checking results.

## Documentation and remaining work

Updated the model inventory, manual Chapter 21, action conformance map and
cleanup coverage notes. The suite now contains 26 safe configurations and 41
expected counterexamples. Corrected stale model counts, the stale-flow negative
invariant name and lease-expiry scenario, missing stack-growth/placement
inventory rows, and descriptions of retired bearer-key lookup and nine-register
IPC receives. Public/bootstrap registrations still share policy-store generation
bookkeeping; NS publication class checks separate them from authorized lookup
and replacement. That distinction is now described without a compatibility
claim. Historical reports are unchanged.

The next model batches remain:

1. Capability reservations, source escrow and hidden IPC attachments through
   receive/first observation, including failure and namespace retirement.
2. DMA pre-claim rejection versus started maintenance, exact unit readiness,
   uncertain completion and non-replayable map/unmap owners.
3. NS public/policy publication separation and death-watch admission, then
   boot trust installation ordering with explicit platform/custody assumptions.
4. Rejected application snapshot restoration and durable publication ordering.
   The conformance map describes the rejection, but the snapshot model still
   represents successful activation only.

These finite checks establish modeled safety within their bounds. They do not
establish liveness, a Rust refinement proof, hardware acknowledgement validity,
thread quiescence, allocator/IRQ/lock behavior, complete sponsor accounting,
registry ticket reuse, or interruption inside a physical walk. The root model
assumes typed subsystem cleanup proofs; the wait model assumes a real producer
notification contract. Broader SEC-07/18 qualification and the production trust
gaps remain open. No new host, native boot, multi-node or hardware qualification
is claimed by this model-only batch.
