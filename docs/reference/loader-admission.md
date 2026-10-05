# ELF and runtime-page admission

`try_load_domain` verifies the native ELF layout and signature before creating a
namespace. Loader backing now has independent admission, using the same owning
counter implementation as [demand heaps](heap-admission.md).

| Scope | Image/runtime physical backing ceiling |
| --- | --- |
| One address-space lifetime | 16,384 pages (64 MiB). |
| Node total | One quarter of usable RAM, rounded down to pages (minimum one). |
| Ordinary domains | Three quarters of the node image pool. |

Every mapped `PT_LOAD` page counts, including zero-filled BSS and page rounding.
The config, status, input, default CQ and four shard-CQ pages also count: eight
runtime pages in total. The image pool is separate from heap and memory-object
pools. Together those three pools promise at most three quarters of usable RAM
on normal machines; uncharged consumers can still cause physical exhaustion.
There is no deployment override or per-principal aggregate across generations.
Only trusted kernel platform launch policy can use the shared reserve.

## Validation and preparation

The allocation-free layout validator accepts at most 64 ELF program headers.
It keeps page ranges in a fixed array, checks overlap/permissions/entry point,
and excludes the entire maximum adaptive heap window, not just its default
capacity. This is a format-policy limit, independent of the page budget.
Checked/saturating aggregate page planning includes all runtime pages and
rejects an oversized signed image before namespace creation. Signatures still
bind the exact original image; none of these checks replace authentication.

An embedded `image_account` belongs to the owning address-space lifetime.
Each page operation validates its captured generation and retains the table
guard across admission, fallible frame-tracking preparation, allocation,
initialization, mapping and commit. `PreparingUserBacking` jointly owns the
reservation, zeroed frame and exclusive address-space borrow. ELF bytes are copied
through an exclusive bounded page borrow before the mapping is installed.
Confirmed rollback frees backing before refunding admission. Failed release
retains its original domain and node charge, even after root destruction and
ASID reuse. Unconfirmed publication retains backing without deallocation. See
[joint preparation](kernel-backing-preparation.md).

The backing allocator also preserves the existing one-eighth free-frame floor
under its frame lock. Root/intermediate page tables are not included in this
page reservation. Their private/lower-half preparation now separately checks
the floor; shared kernel tables may consume its reserve. See
[owned table preparation](page-table-preparation.md). The fallible loading path
reports `BackingAdmission`, `FrameTrackingAllocation`, `FrameAllocation`,
`PageMapping` and `StaleAddressSpace` through `DomainLoadError` rather than
panicking at these backing operations.

`PreparingDomain` retains ownership until successful preparation. An error
after partial ELF/runtime mapping or CQ installation closes the unstarted
namespace, its CQs and owned backing. Logical retirement denies new image
mapping but retains charges until physical address-space destruction. Mandatory
boot-service wrappers can still treat failed launch as fatal after preparation
cleanup. Initial x86-64 root construction now owns its provisional frame and
reports `AddressSpace(RootAllocationFailed)` before namespace publication.
It uses the same physical progress-floor check; AArch64 retains lazy roots.
Root/intermediate-table quota accounting and general kernel/registry allocation
failure handling remain separate work; this is not universal loader OOM safety.
Empty intermediate tables now stay linked for reuse until quiescent teardown;
their high-water cost is described in [page-table lifetime](page-table-lifetime.md).

The unused scalar-ASID public ELF/page mapping conveniences were removed.
Production service loading uses exact handles and the owning preparation.
Architecture/specialized self-test mappers remain kernel fixtures, not newly
budgeted production launch APIs.

## Verification scope

Kernel fixtures exercise real zeroed/filled read-only pages, duplicate mapping,
reduced domain quota, retired admission, exact ASID reuse, and release at
physical teardown. A kernel-only mapper adapter rejects before leaf publication
to check frame/charge rollback. A real signed name-service image fails after
its first mapped page under an injected one-page ceiling; a subsequent normal
load reuses its freed ASID and returns all backing/CQs on teardown. Existing
CQ exhaustion fixtures check rollback after partial CQ installation and platform
reserve access. Isolated counter tests exercise node/ordinary ceilings without
filling the live physical pool.

This is deterministic kernel testing and an AArch64 security-guest regression,
not a real-EL0 image-quota probe, full node pressure soak, real allocator
exhaustion/corruption or exhaustive cross-LP teardown proof. Shared preparation
fault adapters additionally check failed physical release, pool identity,
uncertain publication, domain ceilings and mixed retained/owned teardown.
Stack, page-table, kernel heap,
empty namespace/control-block and general metadata accounting remain open.

Constructor-failure fixtures verify the registration error, unchanged backing
and physical counts, and preserved reusable ASID capacity. An x86-only fixture
checks a rejected root allocator and owned inactive PML4 teardown; it requires
an x86 guest to execute. See the
[page-table investigation](../reports/investigations/2026-10-05-page-tables-locality-and-admission.md)
for locality and reclamation work that must accompany full table budgets.
