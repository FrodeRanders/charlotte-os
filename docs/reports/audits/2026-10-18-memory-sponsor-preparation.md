# Memory sponsor metadata preparation

Date: 2026-10-18. Baseline `3a0454f8` plus the unchanged, uncommitted
[kernel-stack remediation](2026-10-17-kernel-stack-ownership.md).
This continues SEC-07 R7-1/R7-2 and SEC-18 C16/C18/G1/G4.
The 38 finding dispositions remain 29 corrected within scope, six partial and
three open. No new registry, cleanup family, ABI or wire/storage format is added.

## Source boundary

`Ledger::account` previously inserted generation accounts lazily through
`BTreeMap::entry`. That insertion and infallible waiter/scheduler-timer sponsor
construction ran beneath the memory ledger and address-space table guards,
including first memory-object reservation and thread sponsor capture. Retired
account removal also destroyed the node/sponsor fields beneath the ledger.
This was the next concrete allocation observation in the
[status review](2026-10-17-security-status-review.md), not attribution of a
historical stack/IPC/persistence/NIC failure.

## Preparation and publication

`PreparingAccount` captures the original policy limit and trusted classification
before fallible node and sponsor allocation. Its complete partial payload uses
`ManuallyDrop`; ordinary allocation rejection explicitly disposes unused fields
after allocator return. Abandonment retains every field without locks, callbacks,
allocation, refund or physical cleanup. Waiter and scheduler-timer constructors
have fallible runtime entry points. Mandatory trusted fixtures use the same
constructors with an explicit expect/unwrap, not a second allocation fallback.

`PreparingRegistration` captures the actual never-published root before preparing
its capability namespace and account. All early rejection paths finish this owner
after local lifecycle/table guards leave. A failed table publication returns the
same root into that owner before guard exit. On success the existing table hold
captures the assigned generation and publishes the prepared account and capability
namespace once. No metadata allocation or ASID re-resolution is needed for the
account insertion. The complete parent also retains every field on abandonment.

The existing ledger now uses shared `AdmittedMap` nodes. Runtime reservation,
limit setting, retirement and thread sponsor capture only look up existing
generation accounts. Admission rejects a missing account; trusted thread capture
asserts its publication invariant. No lazy compatibility path remains.

The kernel's account is prepared explicitly after heap initialization on the BSP,
before the first LP scheduler/thread and before AP startup. It retains the kernel's
original generation and trusted classification. New userspace account preparation
is ordinary; existing supervisor-only platform promotion remains unchanged.
No live sponsor is replaced or resized, and no manifest/role/name supplies class.

## Retirement and limits

Object page/count charges still follow the creating generation through moves,
retirement and numeric ASID reuse. A closed account with live memory charges
stays in its existing node. `forget` or the final charge detaches that exact node,
releases the local ledger guard, then disposes the node and its sponsor references.
Outstanding waiter/timer references independently retain their original retired
budgets, and cannot credit the replacement generation. Existing physical cleanup,
root/stack leases, limits and reserve classifications are unchanged.

Leaving the ledger is a local context correction. A last-charge caller may hold
another registry, lifecycle or IRQ guard; those call chains still need G1
qualification. Policy/usage/high-water maps also remain separate allocation sites.
The new account preparation does not provide aggregate principal or general heap
byte admission, prove essential-service fairness, or make all root publication
fallible. Quarantine is terminal retention, not a retry owner or recovery service.
The shared admitted map uses linear lookup/insertion scans; this batch does not
qualify worst-case ledger hold time with many live or retained generations.

## Controls and fixture cost

- Four ordered metadata rejection points stop before node, waiter and timer
  allocation and after complete preparation. Boundary checks verify that the
  local table/lifecycle/ledger/heap guards are free at these allocation points.
- Sixty-four private account publications run with the actual heap held, then
  detach and release explicitly. These use a local ledger, not live root identity.
- Sixty-four root-registration metadata rejections never reach the table publisher;
  the original root is disposed after local guards, with physical free counts and
  table occupancy restored. Existing table-publication rejection churn remains.
- A real root's first production sponsor lookup and memory reservation execute
  while the actual heap is held. Its closed account retains a counter-only memory
  charge across numeric ASID reuse; the replacement gets fresh sponsors. Old
  waiter/timer charges remain independent after the old account node is removed,
  and both original and replacement counters are checked through cleanup.
- Four captured/partial/complete account owners are abandoned beneath actual
  lifecycle/table/ledger/heap guards. They deliberately retain three nodes, two
  waiter sponsors and one timer sponsor, with no root or resource charge.
- One complete registration owner is abandoned beneath those guards plus the
  physical allocator. It retains its never-published root, capability namespace,
  one account node and both sponsors. Physical free count and table occupancy
  cannot change; no ASID or public authority is published by that control.

Together the new negative controls retain four memory-account nodes, three waiter
sponsor allocations, two timer sponsor allocations, one capability namespace's
node/account, and one never-published root with its private table ownership.
These are additional terminal fixture costs until guest shutdown, not leak or
recovery claims. The charge/reuse probe is counter-only; actual cross-domain
object backing and hardware pressure remain separate existing/broader tests.

## Validation

Fresh host validation passed **509 Rust tests, zero failed, four ignored**,
across 57 harness summaries, plus signing/tooling/Python controls and cluster-sign
self-tests. Strict kernel Clippy passed both custom targets with `-- -D warnings`;
workspace formatting and diff whitespace checks pass.

The same frozen Rust inputs completed five four-LP QEMU profiles, each with
fresh storage, a 180-second guest budget, assembly-section enforcement and
automatic failure capture. Every runner exited **0**, diagnostic result `complete`:

| Profile | Generation/lifetime probe | Final passed / failed / pending |
| --- | --- | --- |
| ARM / SMMUv3 | 0.952514 s | 17 / 0 / 0 at 10.563070 s |
| Intel VT-d | 2.163739 s | 15 / 0 / 0 at 21.088538 s |
| AMD-Vi | 0.984142 s | 15 / 0 / 0 at 14.604087 s |
| ARM security | 1.028652 s | 19 / 0 / 0 at 56.847442 s |
| Intel security | 0.979734 s | 17 / 0 / 0 at 61.741831 s |

Together these execute 320 root-registration preparation rejections, 320
heap-held private account publications, five exact reuse/lifetime controls and
320 scheduled IPC close/exit episodes with exact exit watches. They do not
establish real-backing exhaustion, sustained fairness or physical-platform safety.
No unexpected native panic, deadline or persistence fail-stop was observed.

All 15 modified/new Rust files were frozen across the matrix, including the
three unchanged files from the preceding stack batch. Both staged service-bundle
inventories match that batch byte for byte; no services were rebuilt or edited.
AMD reuses the Intel no-network kernel, with matching ELF hashes. Security forwards
are 19080/19081 on ARM and 19180/19181 on Intel. Documentation was finalized
after execution; existing unrelated edits were preserved.

The accompanying [version-controlled evidence](evidence/2026-10-18-memory-sponsor-preparation.json)
records commands/environments, the reviewed source hashes, kernel/boot-image/
initial-disk/UART identities, unchanged bundle inventories, concise probe markers
and explicit retained fixture cost. No temporary log or ignored build artifact
is required to assess the published results. This is not a full-tree or physical
platform attestation; historical failed samples remain separate.

SEC-07/18 and the production trust/channel/time milestones remain open within
their existing criteria. No passing repeat attributes an older failure.
