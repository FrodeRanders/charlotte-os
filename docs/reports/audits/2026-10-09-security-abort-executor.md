# Concurrent domain abort retains its executor through root completion

This extends the [self-handoff correction](2026-10-09-security-abort-handoff.md)
within C01/C16 and G1/G2/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger stays **20 corrected, six partial and four
open**. The historical intermittent retirement timeouts remain unresolved.

## Defect and ownership boundary

Deferring only the sweep's own self-request left another window: a concurrent
sweep or unrelated abort could mark its executor during root admission, the
force-request callback or peer scanning. Run-queue selection and owner-LP
retirement treated any abort request as immediately terminal. A sweep could
therefore disappear with its separate `AddressSpaceOperation` still held,
permanently preventing root close. An off-CPU or blocked executor also needed
protection from immediate table removal and rejected wake admission.

`DomainAbortSweep` now contains an `AbortExecutor`. It captures the executing
TID/generation under short local-mask, LP-scheduler and thread-table serialization,
and installs an inline captured-LP fence before acquiring the root operation.
Those guards leave before lifecycle/root admission. Already requested lifetimes
reject before ownership; nested admission rejects without changing the existing
owner. Trusted boot fixtures without a current scheduler handle retain their
existing unstarted-context behavior.

An abort request remains recorded while the executor is owned. The shared
`Thread::abort_ready` predicate permits retirement only when the request exists
and no executor owner remains. Outgoing requeue, candidate selection, wake
admission and owner-LP retirement use that predicate. Placement prefers the
captured LP, cross-LP admission rejects and migration excludes the owner.
Immediate abort rechecks executor/current-handle state under LP/thread-table
serialization before removing queue or table ownership; abort routing rechecks
the captured executor LP before publishing its request. No new registry,
allocation or owner-family row is introduced.

The final IRQ-preserving handoff completes the root operation before consuming
the executor owner. Only then can a pending request retire the thread. Root or
force-publication rejection also explicitly completes admitted ownership in that
order. Failed root completion retains the executor instead of enabling retirement.
No peer scan, yield, IPI, hardware work or physical release enters this handoff.

## Phase classification and fallback

| Phase | Result and retained ownership |
| --- | --- |
| Executor admission rejected | No new executor or root owner; existing request/owner is unchanged. |
| Root admission rejected | Explicit executor release; no new root fence/lease. Any raced request becomes retirement-ready. |
| Force-publication or peer operation rejected | Root completion precedes executor release; the terminal thread-admission fence stays installed. |
| Pending mutual/remote request | Entire sweep retains its exact executor and root; timer wake and scheduling may resume it to complete. |
| Successful handoff | Root count completes, executor fence clears under the same mask, then ordinary off-CPU retirement/reaping proceeds. |
| Abandonment or failed completion | Root/executor/stack fences remain. No destructor locks, frees, logs or clears ownership; no retry/custody adapter is created. |

An explicit terminal self-abort now yields in a loop instead of reaching
`unreachable_unchecked` if a retained executor prevents retirement. That terminal
caller cannot resume an unsafe continuation or abandon its stack to the reaper.
Kernel panic still stops the LP without unwinding. Permanent retention is a
safety fallback, not a progress guarantee or recovery protocol. Arbitrary kernel
operations do not automatically acquire this sweep-specific owner.

## Deterministic execution evidence

The scheduled EL0 verifier creates two actual null-read faulting threads in one
admitted root. Both entries first wait on a mapped fixture word; the verifier
opens this gate only after both publications succeed. A probe armed before
either publication selects the exact root generation. Both sweeps rendezvous after executor/root admission, before either
peer scan. They then request each other and rendezvous again after scanning.
Each calls the production timer-sleep API and performs eight cooperative yields while its abort
request is pending. After every yield it verifies its exact generation/root,
captured LP, retained executor, migration rejection and non-ready retirement.

Each final handoff confirms masked execution and retirement readiness after
root-operation completion and executor release. The containing fixture requires
both normal thread retirement and successful close of the exact root under the
unchanged ten-second deadline. Scalar selectors/counters observe the episode;
they neither own resources nor authorize cleanup. The original one-fault/spinning-
peer fixture remains enabled, including stale-generation/wrong-root rejection.

A separate scheduled kernel probe rejects kernel-root admission, then rejects
force publication on a fresh root and verifies that executor ownership cleared,
its exact thread remains unrequested, IRQ state is preserved and root close
succeeds. Nested executor admission rejects in that callback and both faulting
sweeps. These checks cover ordinary rejection and completion. Destructive panic,
terminal self-abort and permanent abandonment are source-qualified fallback
paths; the verifier does not force-clear such owners to test subsequent reuse.

Selected guest results:

```text
[domain abort] scheduled root/publication rejection released executor ownership
[domain abort] two faulting executors: nested rejection, mutual abort requests, timer wake and eight deferred-retirement yields each, masked owner completion and exact root teardown passed
```

## Validation

| Final QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; probe `0xffff`, publication generations 1/2, 4,836 cancellation requests retired |

Final x86 kernel SHA-256:
`92dc0259f67d508ca06e7f340d8d4e0de0c473bd246fe721a2a543af70844eb1`.
Final Arm kernel:
`3af702f10d3820b66995d192a871a12714802292363114ece43afa76e4972277`.
Both targets passed strict default-feature Clippy with `--locked -- -D warnings`.
Rustfmt, whitespace, local documentation links/tables and the unchanged
18-family/seven-gate map passed. Runners reused validated service bundles,
rebuilt kernels and checked assembly permissions (249 x86 entries, one Arm).
Guest runs were sequential; Arm had local forwarding-port permission. No host
harness rerun was needed for these kernel-only changes.

The final suites emitted both selected executor results and the unchanged
one-fault/spinning-peer result. They did not emit the new recovered-contention
or drained-Pending-close diagnostics: those retry branches were not separately
forced in the final runs. Production sleep calls also retain their existing
runnable admission fallback; the fixture does not separately instrument each
sleep's blocked-state transition. Returned sleep and subsequent exact-root
completion demonstrate sweep progress, without serving as a separate timer
admission receipt.

An initial fixture without the userspace publication gate passed Intel and AMD
(15/15 each; kernel SHA-256
`c90c1776208554af721379eae6d79de26d69f2245bf7e33d2097e56d5fb2e12e`),
but failed on Arm (kernel
`51b04159324afb32773212281056b40a629b41fcacdd49958e75072e158811a5`).
The relevant Arm sequence was:

```text
[+ 1.790485] [user isolation] launching 8 bytes ... asid=103
[+ 1.790742] FATAL EL0 DATA/INST ABORT: ASID=103 ESR=92000007 ELR=20004 FAR=0
[+ 1.791298] Aborting user address space 103
Kernel panic at crates/catten/src/cpu/scheduler/mod.rs:106:38:
address space rejected thread publication: ThreadTerminated
```

The first published peer could fault and install the root's terminal admission
fence before the fixture published its second thread. That rejected fixture
publication stopped the kernel before authoritative suite completion; the runner
returned failure. The gated fixture removes this setup race. This episode is
separate from the historical retirement timeouts; initial passing x86 results
are not counted as validation of the final gated artifact.

A gated Intel run then passed 15/15, but the corresponding AMD run stopped
before the scheduled executor checks in an existing device-rollback availability
probe. Both used kernel
`00256f16df8e9360ac17b3a07ab241c8582e2c77fd803d17ab3e7e9da71dba73`.
Relevant AMD diagnostics:

```text
[+ 6.850961] [thread] abort tid=7 lp=1 asid=0
Kernel panic at crates/catten/src/device/recovery_tests.rs:31:14:
creation rollback holds heap
error: authoritative self-test result was not produced within 180s
```

That assertion rejected a single failed `PRIMARY_ALLOCATOR.try_lock()`. This
establishes contention at the probe, not who held the mutex; the capture did not
record its owner. The assertion's caller-ownership diagnosis was unjustified.
The three real maintenance/physical-release/creation callback probes now bound
global lifecycle/device/table/allocator acquisition by an architecture-counter
one-second deadline. They do not yield or explicitly enable interrupts, preserve
the entry IRQ state and still fail if availability never occurs. A lock retained
by the caller cannot pass. Success after contention emits a separate diagnostic;
no hardware completion, authority release or charge refund is fabricated.
Backend/config probes remain serialized pre-driver checks. This is a test-probe
correction, not proof that the original holder was another LP or qualification of
arbitrary production caller contexts. Final validation below uses this correction
and the publication gate; both failed runs remain distinct observations.

The next Intel run (kernel
`cc02fe73cff6e62f86a6cce8e79d48aa1b9e18a306231189f062afd71779437e`)
completed both executor fixtures and the full user-isolation fault checks, then
stopped in the asynchronous IPC waiter fixture:

```text
Kernel panic at crates/catten/src/ipc/waiter_tests.rs:344:60:
called Result::unwrap() on an Err value: OperationsInFlight
[+ 8.015586] SELFTEST WAITING: passed=11 pending=4 passed_bitmap=0x2bfd pending_bitmap=0x19002
```

The rejected close targeted the server's exact root immediately after the caller
observed the reply result and closed its call. `PreparedReply` publishes the
result under IPC, drops IPC, then completes its caller/server leases; polling
result visibility is therefore not a proof that both leases finished. The
capture does not identify the individual retained operation. The fixture now
retains both exact roots and retries only mutation-free `OperationsInFlight`
rejection under a shared five-second deadline, yielding between attempts.
Other close errors still reject and a permanently retained count still fails;
no root generation, fence or count is reset. A drained Pending close is recorded
separately when observed. Production publication and lease ordering are unchanged.
This is separate from both the earlier heap-contention probe and historical
retirement timeouts. Final validation uses this fixture policy too.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance abort-executor-complete-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance abort-executor-complete-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18639 CATTEN_DEPLOY_HOST_PORT=17939 scripts/run-aarch64.sh --security-test --instance abort-executor-complete-arm-20261009 --fresh-storage --timeout 180
```

## Remaining scope

The historical [Intel preparation timeout](2026-10-09-security-preparation-abandonment.md#execution-and-unresolved-progress-observation),
[Intel IOMMU recurrence](2026-10-09-security-iommu-preparation-abandonment.md#reproduced-retirement-timeout)
and [AMD recurrence](2026-10-09-security-dma-creation-rollback.md#validation-and-remaining-scope)
remain failed episodes. These deterministic executions establish the corrected
abort-sweep boundary; they do not identify the owner retained in those captures
or prove historical causation.

Other retained kernel operations, outer syscall/loader/supervisor contexts,
scheduler metadata and callbacks, partial-cleanup recovery and original failed-
pair custody remain G1/G2/G7 work. Initial IOMMU creation/reset waits, private
rollback and further backend phase separation remain G3 work. Abandoned sweep
ownership is terminal retention; authenticated reconciliation/recovery is absent.
No complete SEC-18 or hardware-recovery claim follows from this correction.
