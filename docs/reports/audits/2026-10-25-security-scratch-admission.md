# Security audit: bounded scratch metadata storage

Date: 2026-10-25. Reviewed baseline: `85ee351c`.

Scratch namespace and extent nodes now reserve requested-layout bytes and a node
count before allocation. The shared ceiling is **512 KiB / 8,192 nodes**, with
ordinary preparation limited to **384 KiB / 6,144 nodes**. Unused concurrent
candidates and retained metadata consume those same limits until actual disposal.

This extends SEC-07 R7-2/R7-3 through the existing C02/C03/C05/C13/C18 owners.
The ledger remains **29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial
and production trust disabled. The [tracked evidence](evidence/2026-10-25-security-scratch-admission.json)
records source/model/artifact identities, failed executions separately from
passing repeats, and selected diagnostics.

## Admission follows allocation lifetime

Each typed node reserves its complete requested layout, including its payload,
link and charge, before the fallible allocator. One preparation owns two nodes:
an extent and a namespace candidate. An existing namespace leaves the unused
candidate in that same preparation until explicit post-guard cancellation.
No production uncharged scratch-node factory remains. The generic host scratch
core retains the identical first-fit/map algorithm without kernel counter globals.

Classification is captured once from the trusted memory sponsor for the exact
retained root, before either factory. Names, manifest roles and caller inputs
grant no reserve access. Later promotion does not change either node's original
classification or final refund. A leaf counter mutex leaves before allocation;
publication/detachment still only relink admitted nodes under scratch serialization.

Quota and allocator rejection return `ResourceLimit` without publishing a range.
Ordinary partial cancellation explicitly frees each allocated node before its
original admission refunds. Partial/retired abandonment remains inert, retaining
every allocation and charge without allocator, registry or counter entry.

Sealed cleanup carries the charged scratch namespace and all its extents in the
same final-root metadata receipt. Failed invalidation retains their original
bytes/count. Confirmation permits post-guard node disposal and refund before
root backing/slot completion. A later physical rejection cannot resurrect freed
metadata or discharge separately retained backing. Exact ASID reuse cannot
redirect the old allocation's refund. There is no new root/pin, registry,
recovery family, custody budget or physical/replay authority.

## Controls and execution

Serialized native controls add:

- **128 allocator rejections**, 64 for each actual node layout, restoring the
  original requested bytes/count. The allocator callback confirms its reservation
  is already live and the local scratch/heap/table/frame guards are available.
- Actual extent-node allocations reach the **6,144 ordinary / 8,192 total count
  limits**. Rejected quota attempts do not enter the allocator. With one slot
  free, the two-node preparation allocates its prefix, rejects the second node,
  restores the exact starting reservation and preserves the existing extent.
- Actual namespace-node allocations independently reach both **384 / 512 KiB
  byte ceilings**, before their count ceilings. Explicit disposal returns the
  exact starting snapshot; no budget is resized, reset or synthetically refunded.
- A platform sponsor prepares/publishes/releases scratch while ordinary nodes
  are exhausted. Closing an existing root needs no fresh scratch admission at
  total exhaustion; its original ordinary nodes refund despite later promotion.
  A successor reuses the real slot with a different generation and consumes only
  its own new charge, then returns it on confirmed close.
- Promotion injected between the two factories cannot reclassify the namespace
  candidate: both nodes use the single earlier capture.
- Existing 192 preparation-prefix rejection controls now check exact admission
  restoration. Existing guarded partial and retired-namespace controls retain
  **six charged metadata nodes**, while holding the new counter mutex too.
  Those selected controls retain no data/root/lease; their heap allocations and
  original scratch admission stay consumed. Other failure fixtures retain their
  corresponding scratch nodes; these are not total kernel quarantine counts.
- The existing complete-root custody control checks unchanged scratch bytes/count
  after rejected final invalidation, exact refund before physical root release,
  stale-ticket/detachment rejection after ASID reuse, and charge retention during
  guarded complete-owner abandonment. No extra data page/root quarantine is added.

The 22 standalone host cases pass: six production scratch cases, including the
6,000-operation bitmap oracle, and sixteen shared admitted-collection cases.
The collection's existing disposal controls establish payload/node free before
charge refund. Strict base-kernel Clippy passes on both architectures.

Corrected native runs pass: **AArch64 20 passed / 0 failed / 0 pending** and
**x86_64 18 / 0 / 0**, with all **28 HTTP checks** and both
security-service generations per run. Healthy responses beside stalled peers
take **0.264–0.356 s / 0.364–0.417 s**, respectively;
trickle rejection completes in **30.15 s / 30.22 s**. Cancellation
pressure retires after **19,388 / 17,964 requests**.

Host retry diagnostics are retained inline in the evidence, separately from guest
results. No test deadline, attempt bound or admission limit is widened. Workspace
formatting, whitespace and local references pass; native assembly-section checks
remain enabled. Bundled services are reused; userspace and the tracked manual PDF
are unchanged.

## Failure record

The initial AArch64 and x86 runs both panic at
`scratch_namespace/tests/admission_tests.rs:92:49`: the reserve-path fixture
unwraps `Err(ResourceLimit)` after exhausting ordinary nodes. Neither reaches
HTTP checks or self-test completion. Both native artifact identities, raw panic
backtraces, result records and selected LLDB frames are recorded separately from
the passing repeats. The fault occurs on the BSP synchronous test path; AP frames
are in their startup barrier.

The fixture used `try_new_platform_user`, which classifies translation preparation,
but omitted the independent trusted memory-sponsor installation. Scratch correctly
classified that published sponsor as ordinary and rejected its exhausted quota.
The correction explicitly checks that missing sponsor classification, then calls
the existing kernel-only `mark_platform` before testing reserve. Only this test
file changes between failed and passing builds; production admission code,
limits and deadlines remain identical. This explains these two fixture panics,
without attributing earlier historical failures.

## Formal qualification and limits

Both focused safe configurations and all **seven expected counterexamples** pass
(the harness checks nine cases). `CharlotteIpcTransaction` explores **26 distinct /
71 generated states**, depth **11**; `CharlotteRootRetirement` explores
**2,978 / 3,733**, depth **33**. Required safe actions execute; negative controls
violate their intended invariant through the named unsafe action.

The models do not represent these concrete byte/count pools, allocator overhead,
virtual extents, first fit or counter-lock context. Those are host/native/source
qualification; there is no executable refinement or sustained-pressure fairness
proof. Model inputs are unchanged and earlier full-suite checkpoints stay separate.

These ceilings bound requested scratch layouts on one node. Aggregate signed
principal metadata, allocator bookkeeping, other metadata families and effective
essential-service capacity remain open. One platform service has no guaranteed
share of the common reserve. Linear first-fit/registry hold times, sustained
concurrent EL0 pressure, scalar range identity, raw fixture disposal and broader
factory/rollback/destructor/IRQ contexts still need qualification. Neither
admission rejection nor range absence supplies physical proof or replay authority.
Passing repeats do not explain historical mapping panics, root-close timeouts
or unrelated initial probe failures.
