# Manual and living-documentation source comparison

Date: 2026-10-18. Source baseline: `cf9f2cf` (the committed reconnecting-client
example). This pass changes documentation only. No numbered security finding
changes disposition: **38 findings, 29 corrected in scope, six partial and
three open**. Historical reports retain their original observations.

## Scope and method

The sweep covered the manual's 21 chapter sources and five appendices, with
targeted source comparisons for capability identity/rights, bootstrap authority,
RPC recovery, waiting, resource admission, driver startup, executable formats,
trust boundaries, storage and build/CI workflows. It also checked the matching
living guides and architecture/reference pages. This is a structured source
review, not a line-by-line proof of every manual statement or every code path.
Research comparisons and historical lessons were checked for current Charlotte
claims; external systems and cited publications were not re-researched.

Source and tests define behavior. Existing runtime reports establish only their
recorded configurations and revisions. A mechanism found in source is not fresh
execution evidence, and rebuilding this PDF does not qualify a device, failure
scenario or production deployment. Broad vision chapters retain their explicit
distinction between implemented mechanisms and proposed APIs.

## Corrections and source anchors

| Area | Mismatch corrected | Source used |
| --- | --- | --- |
| Capability model | Replaced the invented ten-kind/right table with the six authority families. IPC uses `MINT_CONNECTION`, memory uses `MAP_READ`, `MAP_WRITE`, `TRANSFER`; CQ selectors and timer completions are not independent capability kinds. | [namespace](../../../crates/catten/src/capability.rs), [IPC rights and mint attenuation](../../../crates/catten/src/ipc/mod.rs), [memory rights](../../../crates/catten/src/memory/object.rs), [device payload kinds](../../../crates/catten/src/device/mod.rs) |
| Stale identity | Replaced reusable capability-index/generation prose with consumed namespace-local serials and separate root/namespace generations. A staged serial stays consumed after cancellation. | [record insertion](../../../crates/catten/src/capability/record.rs), [admission contract](../../reference/capability-admission.md) |
| Launch and discovery | Ordinary deployed applications borrow a grant-controller connection and immutable signed descriptor; trusted platform services may receive NS authority. Services normally create endpoints. Removed remaining bearer-key gating and fictitious bootstrap minimums. Launch-page ABI is **2.2**, distinct from runtime syscall ABI **3**. | [launch constants and records](../../../crates/charlotte-launch/src/lib.rs), [runtime ABI](../../../crates/charlotte-launch/src/execution_target.rs), [Context](../../../crates/catten-rt/src/lib.rs), [grant controller](../../../crates/catten-services/src/bin/grantctl.rs) |
| Restart and upgrade | Rebinding a name does not revoke a live old endpoint. Removed the blanket failed-request replay sketch; linked the compiled client and distinguished queued endpoint-closed results from delivered cancellation/uncertain effects. The raw reference manager remains explicitly limited. | [NS](../../../crates/catten-services/src/bin/ns.rs), [IPC cancellation/close](../../../crates/catten/src/ipc/mod.rs), [manager](../../../crates/catten-services/src/bin/servicemgr.rs), [echo handoff](../../../crates/catten-services/src/bin/echo.rs), [client](../../../crates/catten-services/examples/reconnecting_client.rs) |
| Completion/CQ contract | Removed unbounded-overflow claims and a fictitious wait-returning-vector ABI. Backlog is preallocated, bounded by accepted submissions, and charged by CQ backing admission. A wait reports readiness/deadline/admission status; mapped-ring reads and exact-resource observation remain separate. | [backlog publication/reconciliation](../../../crates/catten/src/completion/mod.rs), [CQ budgets](../../../crates/catten/src/completion/cq_budget.rs), [typed wait](../../../crates/catten-rt/src/cq.rs), [completion owners](../../../crates/catten-rt/src/owned/completion.rs) |
| Resource admission | Updated claims that shared capability, runtime stack, shared translation and IOMMU admission were all still absent. Preserved the remaining general heap, aggregate principal and metadata limits, and the distinction between independent pools and a complete RAM ledger. | [capability budgets](../../../crates/catten/src/capability/budget.rs), [stack](../../reference/stack-admission.md), [translation](../../reference/translation-admission.md), [IOMMU](../../reference/iommu-table-admission.md), [current ledger](../../reference/security-remediation.md) |
| Retirement | Replaced obsolete blanket lock-held/unbounded x86-rendezvous descriptions with composed root owners, post-local-guard invalidation and bounded epoch checks. Outer IRQ/guard contexts, abandoned-owner recovery and physical-platform quiescence remain unqualified. | [operation leases](../../../crates/catten/src/memory/operation.rs), [root retirement](../../../crates/catten/src/memory/retirement.rs), [x86 IPI epochs](../../../crates/catten/src/cpu/isa/x86_64/interrupts/fixed/ipis/mod.rs), [quiescence contract](../../reference/hardware-quiescence.md) |
| Driver and bootstrap status | Removed a supported unprotected-DMA fallback claim; missing protection skips/rejects startup. Updated AArch64's ordinary embedded set from five to eight, distinguished x86/HVF/test sets, and retained grant preparation before sibling driver threads. TX descriptor reclamation exists; general batching optimization and hardware qualification remain open. | [service store](../../../crates/catten/src/service/store.rs), [launch composition](../../../crates/catten/src/service/launch.rs), [virtio-net drain_tx](../../../crates/catten-services/src/bin/net.rs) |
| Peer security | Removed authenticated-route/retraction claims from the manual, networking and observability designs. Peer/MAC, session and generation checks provide correlation, not cryptographic peer authentication. Kept production trust disabled, key custody/rollback installation open, and unauthenticated time/HTTP limitations explicit. | [discovery decoder](../../../crates/charlotte-protocol-disco/src/lib.rs), [reliable-message protocol](../../../crates/charlotte-protocol-msg/src/lib.rs), [DNS relay contract](../../reference/dns-registration-relay.md), [development gate](../../../crates/catten/build.rs), [boot trust](../../reference/bootstrap-trust-policy.md) |
| Programming examples | Corrected `PerLp`/`ShardLocal` construction and the kernel `ShardMailboxSet` name. Removed the false claim that unwind leaves the borrow flag set. Labelled executor/architecture sketches separately from compiled application source. | [PerLp](../../../crates/catten/src/cpu/multiprocessor/spin/per_lp.rs), [ShardLocal/BorrowGuard](../../../crates/catten/src/cpu/multiprocessor/spin/shard_local.rs), [mailboxes](../../../crates/catten/src/cpu/multiprocessor/shard_mailbox.rs) |
| Build and CI | Kernel commands now prepare signed bundles and set their environment inputs; Sitas uses its helper. CI includes both kernel/boot targets and the formal-model job, without the claimed main-only boot condition. These are checked-in workflow facts, not a newly observed hosted CI pass. | [AArch64 bundle](../../../scripts/build-catten-services.sh), [x86 bundle](../../../scripts/build-catten-services-x86_64.sh), [Sitas bundle](../../../scripts/build-catten-user.sh), [CI](../../../.github/workflows/ci.yml) |

The memory chapter also described transfers as automatically installing receiver
mappings and removing sender mappings. Current
[`validate_source_transfer`](../../../crates/catten/src/memory/object.rs)
requires moves/write loans to start unmapped and permits only read-only existing
mappings for a read loan. Receivers explicitly map delivered capabilities.
The corrected chapter preserves the distinction between transfer publication,
delivery observation, CPU mapping and confirmed loan revocation.

One additional documentation overclaim warrants follow-up: the generic
256-entry IPI queue does **not** reserve mandatory-message slots.
[`IpiCmdQueues::push_to`](../../../crates/catten/src/cpu/multiprocessor/ipi.rs)
retains a `force_push` fallback that can evict the oldest entry. `try_push_to`
instead returns the rejected work. This review did not reproduce saturation or
establish a live failing caller; it corrects the unsupported reservation/progress
claim without asserting a new numbered exploit. Current x86 synchronous
shootdown uses a separate checked epoch/acknowledgement channel.

## Checked claims retained

- AMD-Vi fault MSI routing is still absent: `handle_interrupt` returns false
  and `fault_count` drains the event log. The manual's polling limitation stays.
  See [backend](../../../crates/catten/src/device/amd_vi.rs).
- Current internal formats remain `CDEPLOY6`, `CHS3PF1`, `CHKAFP7`, object-store
  v4, full-name NS/DNS records and runtime ABI 3. Mentions of retired formats
  describe rejection, not compatibility promises. See [API simplification](../../reference/api-simplification.md),
  [object-store format](../../reference/object-store-format.md),
  [execution targets](../../reference/execution-targets.md),
  [S3](../../reference/s3-client.md) and [Kafka](../../reference/kafka-client.md).
- The two-object Raft pair has no cross-object atomic commit. Partial or corrupt
  state rejects rather than silently resetting; live-quorum power-loss behavior
  is not freshly qualified. See [persistence contract](../../reference/raft-persistence.md).
- General remote capability delegation, arbitrary-service zero-downtime upgrade,
  production enrollment/key custody and recovery of abandoned physical owners
  remain open. Durga and the richer application API remain proposals.
- TLA+ checks are bounded model evidence, not a machine-checked refinement proof
  of the complete Rust implementation. This sweep did not rerun TLC.

## Verification

Results from this documentation pass:

- Checked **107 repository-path links in 27 manual TeX files** and **1,255
  relative links in 105 Markdown files**, including the living documentation,
  research pages and the updated report/index. All target paths exist. This
  does not check external URLs or Markdown heading anchors.
- Checked all four marked source ranges used by the included reconnecting-client
  listings; their start/end markers remain unique and ordered.
- Rebuilt the tracked manual with `pdflatex -no-shell-escape`, using the existing
  unchanged figure cache, until references settled: **225 pages**, no undefined
  references or rerun requests. Rendered and inspected 28 selected pages covering
  the changed tables, API listings, recovery, build/CI and status text; rechecked
  the final status-page flow after shortening its duplicated summary.
- Existing whole-manual overfull-box warnings remain. This pass introduced no
  new or changed paragraph overflow warning against the preceding manual build;
  it is not a complete typography audit of the book.
- `git diff --check` passes. The sweep changes only documentation; no Rust source
  or executable behavior changes. No new kernel, host, multi-node, TLC or hardware
  qualification is claimed. The client build/test results belong to the preceding
  example change, not a fresh execution of this documentation sweep.

Tracked artifact: [manual PDF](../../manual-v2/charlotte.pdf), **1,066,921 bytes**.
SHA-256:

```text
f0f72a99c76174d6d4734cf19b624bff1614b602ee62697292f5692f17cba820
```
