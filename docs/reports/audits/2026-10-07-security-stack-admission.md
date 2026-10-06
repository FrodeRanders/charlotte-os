# Runtime stack backing admission

Date: 2026-10-07; work began 2026-10-06. SEC-07 follow-up after shared-table
admission at `149342e4`. This adds aggregate runtime stack capacity admission and
preserves the original reservation/root through uncertain stack-pair cleanup.
Broader allocator and metadata findings remain partial.

## Change

User-thread preparation reserves its configured maximum user pages plus sixteen
kernel pages before physical allocation. A separate node stack pool uses one
eighth of usable RAM, rounded down to pages with a minimum of one. Ordinary
domains can consume three quarters; trusted platform/kernel preparation can
use the remaining quarter within the same total limit. This is an initial
kernel policy, not a measured workload requirement or an application override.

The exact `StackSlot` owns its root lease, bitmap slot, captured capacity and
reservation class. Its existing 64-slot ceiling and maximum 64 user pages per
slot bound one root's combined stack footprint to 5,120 pages (20 MiB); lower
signed/adaptive limits still apply. Kernel-only threads reserve sixteen pages.
User backing remains demand-grown. Reservation covers future capacity rather
than eagerly allocating pages or guaranteeing physical availability.

Both architecture contexts now own one `Stacks` transaction for their user and
kernel ranges. The duplicated context-specific growth/cleanup routines and
manual constructor cleanup ladder are removed. Successful cleanup refunds only
after both physical ranges complete. Partial release, interrupted publication,
failed provisional release and uncertain kernel preparation/cleanup retain the
whole maximum reservation. User failures also keep their original bitmap slot
and root lease, preventing root teardown and ASID reuse from returning admission.
No per-frame ledger, recovery queue or administrative force-clear is introduced.

Demand growth borrows that owner, captures physical backing before zeroing,
revalidates the exact root generation and records committed progress before
relinquishing the provisional frame. Atomic per-frame physical-growth floor
checks complement the existing aggregate availability estimate. An uncertain
growth owner permanently fences the slot; subsequent cleanup of its ordinary
committed prefix cannot discharge lost or potentially reachable backing.

Kernel mapping rejection now moves its unpublished frame into the same
`RetiredKernelRange` as its detached prefix. Explicit post-guard release reports
that frame's release failure to the original stack admission owner; it is no
longer hidden in provisional `Drop`. Real `AlreadyMapped` rejection preserves
foreign leaves. Failed rollback and failed post-publication invalidation return
an incomplete-cleanup marker, retaining the owning transaction's admission.
Raw stack allocation/free is confined to the memory adapter used by `Stacks`.

Current contracts: [stack admission](../../reference/stack-admission.md),
[thread admission](../../reference/user-thread-admission.md) and
[kernel retirement](../../reference/kernel-frame-retirement.md).

## Regression evidence

- A real stack pair reserves twenty pages for a four-page user limit and its
  kernel stack. A foreign leaf forces partial demand growth; its contents remain
  unchanged, rejected preparation returns its frame, and retry reaches the full
  limit after removing the foreign leaf. Growth adds no reservation. Drop returns
  exact physical/admission counts and releases the slot.
- Ordinary node admission pressure rejects before the allocator callback while
  real platform-user and kernel-only stacks succeed without ordinary charges.
  Returning those owners reconciles counts; ordinary preparation recovers after
  pressure ends. This is counter pressure, not whole-node physical exhaustion.
- Initial allocation rejection and confirmed kernel-preparation failure refund
  the complete reservation and permit ordinary root close. Existing architecture
  thread tests still cover quota, all 64 slots, 128-round construction/Drop churn,
  publication/watch authorization, launch rollback and successor-ASID rejection.
- Five failed/abandoned cases retain five roots/slots and 85 reserved pages:
  unpublished release rejection, uncertain kernel preparation, committed user
  release rejection, interrupted growth and rejected cleanup of a real mapped
  kernel stack after successful user cleanup. Three provisional/user frames and
  one sixteen-page kernel range remain unavailable, in addition to retained
  private root hierarchies. Failures/interruption are injected owner states;
  actual panic unwinding and physical hardware failures are not reproduced.
- Kernel-range fixtures now observe both unpublished and installed-prefix frames
  retained until explicit post-guard release. Foreign-leaf preservation and the
  existing failed-barrier/retry tests continue to pass.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**, including the final five stack retention cases and kernel retirement fixtures. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**, including the same fixtures and operational storage. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; both scoped probes reported `0xffff`, publication generations 1/2, concurrent cancellation retired 4,828 requests. Socket-pressure clients retired 1,501/1,466 requests with time/cycle progress. |

Commands (instance names retain the starting date):

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance stack-admission-intel-20261006 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance stack-admission-amd-20261006 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance stack-admission-arm-20261006 --fresh-storage --timeout 180
```

The final Intel run includes the additional mapped-kernel cleanup case. Runners
rebuild kernels and check native assembly section permissions. Existing validated
embedded service bundles are reused; userspace and wire formats are unchanged.
Logs: `/private/tmp/charlotte-stack-admission-{intel,amd,arm}.log`,
`/private/tmp/charlotte-stack-admission-clippy-{x86,arm}.log`,
`/tmp/charlotte-x86-stack-admission-{intel,amd}-20261006-serial.log` and
`/tmp/charlotte-stack-admission-arm-20261006-serial.log`.

## Remaining scope

SEC-07 remains partial for inherited boot/CPU/interrupt stacks and translation
tables, IOMMU tables, general kernel heap and metadata/callback storage. Reserving
maximum stack capacity is conservative and can reject a thread whose actual
working set would fit. Independent pools are not a complete physical RAM ledger,
NUMA policy, fair-share mechanism or worst-case latency guarantee. Other consumers
can still cause real physical allocation failure; mandatory kernel callers retain
their existing fatal error policy.

SEC-18 general allocation under serialization, broader reset support and
abandoned-owner recovery remain open. Authentication, production provisioning
and security-time findings remain separate. QEMU does not prove physical-platform
quiescence or permanent CPU/device failure recovery. Retained root/slot/reservation
and backing owners stay fenced; this change introduces no reclamation bypass.
