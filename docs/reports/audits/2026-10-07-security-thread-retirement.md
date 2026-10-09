# Prepared deferred thread retirement

Date: 2026-10-07. SEC-07/SEC-18 continuation after slot-return storage at
`78a56a2c`. Deferred thread ownership now uses publication-prepared nodes;
general heap admission and broader scheduler/subsystem metadata remain partial.

## Finding and correction

Thread exit/abort already deferred stack destruction to its owning LP. However,
staging used a `BTreeMap<LpId, Vec<Thread>>`, and reaping partitioned a detached
vector into two fresh vectors before extending the staged vector again.
Owner-LP requested abort cleanup also allocated a request snapshot. These
infallible allocations could occur after scheduler/table mutation, including
while the staging guard was held. Allocation failure could prevent the final
handoff of a still-owned context and its physical stacks.

`Thread::try_new` now fallibly prepares an empty owning retirement node before
claiming a generation, allocating stacks or publishing the thread. Rejection
returns `ThreadPreparationFailed`. Each live thread carries that node; staging
consumes it and stores the entire thread inside it. Context boxing and stable
assembly pointers remain unchanged. There is no second prepared node inside a
staged thread.

The registry has 256 inline LP heads, matching the validated scheduler limit.
Staging, batch detachment, reversal, filtering and reinsertion move existing
nodes without allocation. Reaping preserves insertion-order release, invokes
callbacks and stack destruction after registry/table guards are released, and
retains nodes containing the executing SP. Arm additionally retains contexts
whose assembly `on_cpu` ownership flag is still set. x86 continues to use
pinned scheduled reapers and disables runtime migration.

`ReapBatch` owns detached/deferred nodes and the lifecycle transition marker.
Only explicit completed release/reinsertion clears that marker. Abandoned
published nodes/lists quarantine without payload destruction or recursive
cleanup; abandoned batches retain their publication fence. There is no recovery
or force-clear API. Requested-abort extraction now scans a captured table slot
ceiling, qualifying and consuming each exact occupant under the same table
hold instead of allocating a numeric-ID snapshot.

The first Arm validation exposed a related self-exit mismatch: the old path
removed the current scheduler handle before switching. The switch then treated
the outgoing context as absent, skipping its save and ownership-release
handshake. The stricter reaper correctly retained it, and an exit-watch test
timed out. Self-exit now retains the handle/context with an owner-side abort
request until the real switch completes, using the existing requested-abort
exclusion and post-switch extraction. The check was retained; the exit-watch
timeout was not relaxed.

Contract: [deferred thread retirement](../../reference/thread-retirement.md).

## Regression evidence

Four standalone host tests exercise preparation rejection, unused preparation,
exact node identity across heads, explicit release and abandonment. A
current-test-thread allocator tracer records **zero allocations/reallocations**
while staging, detaching, reversing, filtering, requeueing and explicitly
releasing **1,024 prepared payloads**. Callback allocations are outside that
claim. Two tiny generic fixture nodes deliberately remain retained to verify
abandonment; they contain no guest backing.

Guest fixtures reject node preparation 64 times and check that neither thread
generations nor physical frames changed. Actual never-scheduled kernel threads
then exercise executing-stack retention, independent LP heads, exact-generation
lookup, callback guard availability, in-flight retirement visibility and final
physical stack recovery. Arm also tests a synthetic ownership flag. Private
SP/head overrides only target never-scheduled fixtures; they do not authorize
cross-LP production reaping. This fixture retains no guest backing after success.

The scheduled exit-watch fixture now checks the real self-exiting worker's
exact current handle, master-table context, requested abort and Arm ownership
flag before switching. The worker must never resume, and its watch must complete
within the existing timeout. Natural-return workers, producer cancellation and
real remote abort/scheduler-lifecycle tests remain enabled.

## Validation

| Check | Result |
| --- | --- |
| Standalone retirement-list host harness | **4 passed, 0 failed**; 1,024 prepared payloads released without allocation. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Host-runner syntax | `bash -n scripts/run-host-tests.sh`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,676 requests. |

```sh
rustc --edition=2024 --test crates/catten/src/klib/collections/retirement_list.rs \
  -o target/host-self-tests/retirement-list-tests
target/host-self-tests/retirement-list-tests
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance thread-retirement-intel-final-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance thread-retirement-amd-final-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance thread-retirement-arm-final-20261007 --fresh-storage --timeout 180
```

The host harness uses the repository toolchain and is included in
`scripts/run-host-tests.sh`; the full host script was not rerun for this batch.
Guest runners rebuilt kernels and enforced assembly section permissions, reusing
validated embedded services for this kernel-only change. Arm required local
forwarding-port permission.
Intermediate runs of the corrected self-exit path also passed all three
guests before the new scheduled pre-switch assertions were added.

## Remaining scope

Each live/deferred thread adds one thread-sized node allocation in the global
heap. It is fallible and prepared before publication but has no independent
byte/principal budget. General SEC-07 heap/metadata admission remains open.
Whole-domain abort admission/snapshots, scheduler run queues, migration
snapshots and other subsystem registries still have separate allocation paths.

Callback internals and Arm's enclosing interrupt state remain unchanged. This
batch removes staging/filtering allocation and owns abandonment fences; it does
not establish complete SEC-18 locking or quiescence. Physical-platform device
reset, broader abandoned-owner recovery, authentication, production provisioning
and security-time findings remain unchanged.
