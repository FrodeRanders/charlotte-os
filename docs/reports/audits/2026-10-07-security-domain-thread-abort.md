# Inline whole-domain thread abort

Date: 2026-10-07. SEC-07/SEC-18 continuation after prepared deferred thread
retirement at `51bd2225`. Whole-domain abort no longer allocates a fence-map
entry or thread-ID snapshot. General heap/metadata admission and broader
locking/quiescence remain partial.

## Findings and corrections

`SystemScheduler::abort_as_threads` inserted into an infallibly growing
`BTreeMap<AddressSpaceId, generation>` and collected every matching numeric TID
into a vector. It held the publication gate across the entire abort sweep to
prevent TID reuse. These teardown allocations could fail while a domain was
already being force-terminated. The fence map also retained its numeric-ASID
high-water storage independently from physical root ownership.

The new `DomainAbortSweep` first admits an exact-root `AddressSpaceOperation`.
It then closes an inline `thread_admission_closed` flag on that retained root
under thread-publication serialization. Both architecture constructors initialize
the flag unfenced. No abort-map entry or snapshot is created; the shared gate is
an inline `Mutex<()>`. The sweep captures a slot ceiling, checks each exact root
owner under the thread table, and captures that occupant's generation before
calling `abort_thread_generation` outside the table/publication guards. Unrelated
publication and TID reuse are safe: the old generation cannot select a successor.

Fresh preparation rejects an aborting root before node/generation/stack mutation.
Stack-slot admission checks the flag again. Previously prepared threads cannot
publish after the fence, and their rejected owners drop after serialization
leaves. Explicit sweep completion releases only its operation lease. The terminal
fence remains closed; pending thread stacks retain their own root owners until
actual reaping. Abandonment retains the root operation/fence. No force-clear or
abandoned-owner recovery API is added.

Review also found prepared-thread publication accepted a staged-closing root.
A prepared stack already leased that root's lifetime, but a lease does not stop
staged close from fencing new admission. User publication now takes lifecycle
before its publication gate, qualifies its captured handle, rejects both abort
and closing fences, and retains lifecycle through master-table publication.
Fresh preparation likewise rejects closing. Root/table/lifecycle guards leave
before rejected thread callbacks and stack destruction.

Delayed node/deployment and verifier aborts now use retained handles instead of
re-resolving ASIDs. Forced node/deployment request publication occurs only after
exact-root admission/fencing and while its lease remains owned. Deployment force
abort retains its existing `Polling` registry claim, releases the registry guard
before lifecycle admission and restores ordinary waiting only after success.
Competing retirement remains pending. Rejection caches `ThreadAbortRejected`,
retains the entry/phase, and does not count a forced retirement or retry the
failed request. A stale or closing root cannot publish a force request. Physical
cleanup is still a separate `DomainTeardown` operation after thread quiescence.

Contract: [thread retirement](../../reference/thread-retirement.md).

## Regression evidence

The guest thread-admission suite now uses real never-scheduled user roots and
threads to verify:

- Fence publication rejects a prebuilt thread; its exit callback observes free
  publication, thread-table, root-table and lifecycle guards. Late preparation
  rejects 64 times without changing physical frames or stack-slot ownership.
- A thread slot is recycled to an unrelated domain between sweep capture and
  scheduler claim. The captured generation rejects and the replacement survives;
  the remaining target threads stage/reap normally. Their original roots and
  stacks retain ownership until explicit completion.
- Force publication runs outside lifecycle/table/publication guards while root
  close remains busy on its admitted lease. Sixty-four repeated sweeps release
  their temporary leases, permitting ordinary final close.
- ASID reuse starts unfenced and admits a new thread. Sixty-four obsolete-handle
  requests reject before their force callbacks and leave the successor live.
  Closing and kernel-root requests also reject before mutation.
- A prebuilt thread fails publication after staged close begins; fresh thread
  preparation rejects too. Final staged close succeeds after rejected owner
  cleanup.
- Isolated node/deployment bookkeeping receives a stale domain with dummy
  config/status addresses while a successor thread remains live at that ASID.
  Two polls return cached failure without reading/writing those addresses,
  counting force success, removing the entry or advancing device gating. Node
  Drop does not retry its failed claim; the successor survives.
- After ordinary cleanup all guest physical frames return to the warmed
  baseline. These new fixtures retain no root, stack or charge permanently.

The pre-claim callback substitutes only controlled scheduling interleaving.
The inspected test contexts never ran; existing real remote-abort, natural
thread exit, scoped deployment/cancellation and shutdown verifiers remain enabled.
This is not actual allocator exhaustion or hostile workload containment. No
new allocator instrumentation or direct interrupted-sweep test was added;
operation-owner abandonment is covered by the existing lease foundation.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,776 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance domain-abort-intel-final-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance domain-abort-amd-final-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance domain-abort-arm-final-20261007 --fresh-storage --timeout 180
```

Guest runners rebuilt kernels and enforced assembly section permissions, reusing
validated embedded service bundles for this kernel-only change. Arm required
local forwarding-port permission. No new standalone host harness was needed;
the full host script was not rerun. Final logs:
`/private/tmp/charlotte-domain-abort-{intel,amd,arm}-final.log`,
`/private/tmp/charlotte-domain-abort-clippy-{x86,arm}.log`,
`/tmp/charlotte-x86-domain-abort-{intel,amd}-final-20261007-serial.log` and
`/tmp/charlotte-domain-abort-arm-final-20261007-serial.log`.
The earlier Intel/Arm runs also passed before the closing-publication regression
and its complete lifecycle hold were added.

## Remaining scope

The root fence and sweep owner are inline and have no new allocation budget.
Thread node preparation remains fallible global-heap allocation without its own
byte/principal pool. Scheduler queues, migration snapshots and other subsystem
metadata still allocate separately; general SEC-07 admission remains open.

Successful abort is request submission, not proof of thread, device or root
quiescence. ARM's enclosing reaper interrupt state, arbitrary callback work,
exceptional Drop under unknown outer locks and other subsystem serialization
remain broader SEC-18 work. Cooperative drain/status access outside this forced
abort lease is unchanged. Physical-platform device reset, broader abandoned-owner
recovery, authentication, production provisioning and security-time findings
remain unchanged.
