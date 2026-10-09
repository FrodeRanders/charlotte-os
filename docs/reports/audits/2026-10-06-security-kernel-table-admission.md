# Shared runtime kernel-table admission

Date: 2026-10-06. SEC-07 follow-up after private-table admission at `68b525f8`.
This bounds fresh shared runtime translation-table frames on both architectures.
The broader memory-admission finding remains partial.

## Change

Shared higher-half allocation now reserves an owning charge before physical
allocation. Its independent node pool is one sixty-fourth of usable RAM, rounded
down to pages with a minimum of one. This is an initial kernel policy, not a
user-selectable limit or a measured worst-case service requirement. Shared
tables may consume the physical progress floor within this pool, but real
allocator exhaustion still rejects allocation. The pool guard is released before
physical allocation, initialization, architecture publication or rollback.

`PreparingTable` retains the charge alongside its physical owner. Only an unused
reservation or a confirmed unpublished physical release permits refund.
Rejected/abandoned release and unconfirmed publication retain admission and
backing. Successful publication makes the charge permanent for the kernel
lifetime. Partial linked trees and empty branches remain charged and reusable.
No teardown ledger, recovery queue, force-clear, scalar restoration or
reusable-ASID lookup is introduced.

Shared links copied or borrowed by user roots do not create fresh charges, and
user-root destruction cannot refund them. Private admission retains its separate
owning-root account and node pool. Both walkers derive scope from validated
architecture mapping context; users cannot select shared-kernel admission.
Existing callers retain their current fallible/fatal allocation error policy.

Bootloader-inherited tables predate the preparation owner and remain outside
runtime admission. No retroactive adoption or complete boot-table census is
claimed. This pool provides neither per-domain sponsorship/fairness nor reserved
platform headroom for shared tables. Stacks, kernel-heap data and IOMMU tables
still need their own admission.

Current contracts: [translation admission](../../reference/translation-admission.md),
[table preparation](../../reference/page-table-preparation.md) and
[table lifetime](../../reference/page-table-lifetime.md).

## Regression evidence

- Admission exhaustion rejects before the allocator callback runs. Unused
  reservations and successfully released zeroed preparation restore exact
  physical/admission counts without quarantining capacity.
- Real higher-half mappings build a sparse shared hierarchy. One admitted
  prefix survives rejection of the next table; repeated failure allocates
  nothing further. Sixteen cached unmap/remap rounds succeed at the ceiling.
  Restoring admission completes the prefix without duplicating its charge.
- Four user-root creation/destruction rounds see the same kernel alias and
  neither duplicate nor refund shared charges. Private charges return normally.
  Foreign data is detached and invalidated outside `KERNEL_AS` before release.
  Empty linked fixture tables remain kernel-owned, reusable and charged; exact
  physical deltas match the shared table count.
- Two additional failed/abandoned provisional preparations retain two physical
  frames and two charges, continue to exhaust their test admission, and remain
  charged through a later successful provisional rollback. Interruption is
  simulated; actual panic unwinding and hardware races are not reproduced.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**, including shared and private table fixtures. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**, including shared and private table fixtures. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; shared/private table fixtures passed, both probes reported `0xffff`, publication generations 1/2, concurrent cancellation retired 4,776 requests. Pressure clients retired 1,376/1,342 requests with time/cycle progress. |

Commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance kernel-table-intel-20261006 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance kernel-table-amd-20261006 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance kernel-table-arm-20261006 --fresh-storage --timeout 180
```

Runners rebuild the kernels and validate native assembly section permissions.
Existing validated service bundles are reused; no userspace or protocol change
is included.

## Remaining scope

SEC-07 remains partial for inherited kernel tables, IOMMU tables, stacks,
kernel heap and general metadata/callback storage. These independent pools are
not a complete RAM ledger, table compactor, NUMA policy or latency guarantee.
Hard shared-table admission does not establish denial-of-service isolation for
every kernel consumer; allocation failure still follows existing caller policy.
QEMU fixtures do not prove true node-wide physical exhaustion or physical-platform
races. SEC-18 allocator work under serialization, broader reset support and
abandoned-owner recovery remain open. Authentication, production provisioning
and security-time findings remain separate. No uncertain backing or charge is
reclaimed by this change.
