# Heap/image preparation abandonment without cleanup

Date: 2026-10-09. Source baseline: `e490592c`.
Scope: C17/C04 under G1 in the
[cross-category cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-07/18 remain partial; totals remain 20 corrected, six partial and four open.
No registry, controller, recovery authority or wire format is added.

## Problem and correction

`PreparingUserBacking::drop` previously invoked physical rollback. A caller
could still hold its exclusive address-space table borrow and unrelated guards,
so abandonment could enter the allocator or logger under that unknown context.
The active `PageCharge` field could also acquire its original backing pool to
refund an unused reservation. This differed from the required fallback contract:
retain uncertain work without physical cleanup or lock acquisition.

The owner now separates the outcomes:

- Ordinary tracking/allocation rejection refunds the unused reservation
  explicitly. Confirmed mapper rejection explicitly rolls back unpublished
  backing. Consuming `cancel_unpublished` reports physical rejection. Frame
  consumption and account retention precede allocator calls; rejected release
  is not retried or credited as free capacity.
- Abandonment mutates only the captured exclusive account and disarms the
  frame/reservation tokens. It takes no allocator, table or backing-pool lock,
  invokes no callback and logs nothing. The active charge becomes nonrefundable
  in the original domain/account/classification before its destructor could
  refund. Interrupted commit does not count the inert reservation twice.
- A reservation abandoned before allocation remains consumed even though it
  has no backing frame. Ordinary errors still refund; abandonment has no owner
  proving cancellation. Root teardown excludes those counts from refund and a
  successor never gains access to their admission.

`RetiredKernelRange::drop` also stops entering the early logger. Its existing
atomic quarantine diagnostic and retention remain; it performs no physical
release or invalidation. The atomic counter is not a new external telemetry API.

Production heap first-touch (`commit_user_heap_page_with_mapper`) and image
loading (`map_image_page_with_mapper`) were checked. Both retain their exact
address-space table borrow through preparation/mapping and ordinary rollback.
No caller changes or reusable-ASID cleanup were introduced. The fallback is
now independent of that guard's context, but **ordinary physical rollback still
occurs under the known table guard**. Moving it outside serialization with
complete ownership remains G1/G2; this does not declare C17 complete.
Raw frame/table/stack and IOMMU preparation destructors retain their own gaps.

## Regression evidence

For both heap and image admission, serialized kernel fixtures exercise actual
preparation Drop while the original table, physical allocator and backing pool
guards are held. They cover allocated/unallocated abandonment and three real-leaf
publication states: before account commit, after commit with an inert token, and
after removing that token before frame transfer. Free-frame counts remain
unchanged, installed leaf identity remains intact, retained domain ceilings
reject preparation before tracking/allocation, and charges survive root
destruction plus exact software-slot reuse.

Existing constructor errors, explicit cancellation, mapper rejection, zero/fill
preservation, success, physical rejection, ordinary/platform classification and
mixed owned/quarantined root teardown remain enabled. Explicit cancellation
failure is observed rather than reported as success. These fixtures retain
fourteen physical data frames and sixteen charges (eight heap, eight image);
two charges were abandoned before allocating a frame. Earlier kernel/root/
memory-object fixture quarantine is additional.

The existing one-page detached kernel-range abandonment probe now holds both
the kernel-table and physical allocator guards. Drop returns with unchanged free
frames and its atomic quarantine count incremented. No new kernel-range page is
intentionally retained beyond that fixture's existing page.

These are controlled owner-state/allocator-error probes, not real panic unwinding,
physical corruption, node exhaustion or a latency/fairness guarantee. A held-pool
fixture proves absence of pool re-entry by execution; logger absence also follows
from inspecting the fallback call chain, not a held-logger test.

## Execution and unresolved progress observation

Initial Clippy checks on both kernel targets and the host suite passed. The first
Intel VT-d execution passed the new probes, then failed later in
`user_isolation::Fixture::finish` while waiting for a user stack retirement lease
(`passed=14 failed=1`). This is a failed validation run, not a successful result
with an extended deadline.

The failing kernel SHA-256 was
`c0d2f33e3e76d94ef9da8d06b0149e9af0b8c352636e72046b45bb89fe99cbb5`.
The last recorded fault was divide-by-zero in ASID 68. This run predates the
root-generation and retirement-counter diagnostics; those values cannot be
recovered from its capture.

Selected failure output (guest timestamps are seconds since boot):

```text
[+     1.581786] FATAL USER FAULT: ASID=68 vector=0 error=0x0 RIP=VAddr(0x20009) address=VAddr(0x0)
self-test deadline expired while waiting for user stack retirement lease
[+    11.608540] SELFTEST COMPLETE: passed=14 failed=1 pending=0 passed_bitmap=0x1bbfe failed_bitmap=0x1 pending_bitmap=0x0
```

AMD-Vi passed 15/15 and Arm SMMUv3 security passed 19/19. A traced Intel execution
and a fresh untraced execution each passed 15/15 without changing the fixture
deadline or clearing any retained count. These runs did not establish the cause
of the initial failure or prove that stack retirement always makes progress.
Concurrent build/host work overlapped the first run, but no causal attribution
to host scheduling or this patch is established. C16/G1/G2/G7 retain that concern.

Final-source validation, after finalizing the explicit cancellation error result:

| Check | Result |
| --- | --- |
| Intel VT-d, four-LP default suite | 15 passed, zero failed/pending |
| AMD-Vi, four-LP default suite | 15 passed, zero failed/pending |
| Arm SMMUv3 security suite | 19 passed, zero failed/pending; scoped probe `0xffff`, generations 1/2, 4,804 cancellation requests |
| Kernel Clippy, both custom targets, `--locked -- -D warnings` | Passed |
| Host suite | Passed, including 29 retirement-slot tests and signer/policy regressions; no host-only simulation replaces the kernel probes |
| Rustfmt and diff whitespace | Passed |

Each final guest emitted the new backing-preparation and guarded kernel-range
markers; all kernel assembly-section checks passed. Final QEMU runs were started
in sequence. No full pressure/concurrent-host-scheduling qualification is claimed.
Only kernel source changed, so the already-built/signed service bundles were
reused; no syscall/runtime ABI changed.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance preparation-drop-intel-verified-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance preparation-drop-amd-verified-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18539 CATTEN_DEPLOY_HOST_PORT=17839 scripts/run-aarch64.sh --security-test --instance preparation-drop-arm-verified-20261009 --fresh-storage --timeout 180
scripts/run-host-tests.sh
CATTEN_X86_64_SERVICE_BUNDLE="$PWD/target/embedded-services/x86_64-unknown-none" cargo clippy --locked -p catten --target target_specs/x86_64-unknown-none-catten.json -- -D warnings
CATTEN_AARCH64_SERVICE_BUNDLE="$PWD/target/embedded-services/aarch64-unknown-none" cargo clippy --locked -p catten --target target_specs/aarch64-unknown-none-catten.json -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Limits and next work

This corrects the scoped abandonment boundary. It does not remove ordinary
physical rollback from a borrowed table, convert failed stack/mapping receipts
into retry owners, separate IOMMU maintenance, reconcile supervisor errors,
implement authenticated recovery, or qualify physical hardware. No address or
diagnostic can adopt abandoned backing. Next G1 work remains raw frame/table/
stack preparation and their complete caller/drop contexts; C16 progress failure
needs phase-aware evidence, not force-clear.
