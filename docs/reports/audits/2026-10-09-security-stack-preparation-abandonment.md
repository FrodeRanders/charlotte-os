# Initial/growth stack preparation abandonment

Date: 2026-10-09. Follow-up to
[table/raw-frame fallback and stack phase diagnostics](2026-10-09-security-table-abandonment.md).
Scoped progress for [C16/C17 and G1/G2](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger remains 20 corrected, six partial and four open.

## Ownership and ordinary cancellation

`PreparingStackPage::drop` now quarantines its frame without physical release,
locks or logging. Implicit `StackSlot`, operation-lease and reservation destruction
retain the original bitmap slot, exact root generation and entire maximum
user/kernel admission. Even reservation-only abandonment remains charged.
There is no unused-slot cleanup destructor and no address/ASID re-adoption.

Normal initial-page allocation rejection, invalid layout and confirmed mapper
rejection invoke consuming `cancel_unpublished`. Physical frame ownership is
consumed before allocator entry. Physical rejection retains the original
root/slot/reservation; it returns a distinct preparation error from slot
completion rejection. Slot cancellation is only permitted before publication
or uncertainty, and verifies captured generation plus the reserved bit before
removal. Its table guard leaves before charge/lease completion.

`PreparingGrowthPage::drop` now only fences the exclusively borrowed parent
slot and quarantines provisional backing. Ordinary allocator rejection marks its
unused preparation completed, preserving a usable parent. Normal mapping
rejection explicitly rolls back after the mapper's local table guard leaves.
Only confirmed physical rollback clears the fence it just armed. Rejection or
interruption consumes no authority/refund and cannot trigger a second release
through Drop. Existing uncertain stacks cannot grow. Successful publication
records committed progress and transfers the frame into the containing stack.

The 64-slot admission fixture and successor-generation fixture now consume
ordinary unused slots explicitly. The six existing retained stack cases still
run, including initial physical-release rejection, incomplete kernel rollback,
user physical rejection, interrupted growth, kernel release rejection and
invalidation rejection. Their prior scope/retention remains unchanged.

## Immediate caller/context review

| Entry | Observed boundary | Remaining qualification |
| --- | --- | --- |
| `Stacks::user` → initial reserve/map | Captures original slot/root/charge before allocation. Ordinary local mapper failure leaves its address-space guard before explicit cancellation. | Kernel allocation failure after user publication still relies on published `Stacks` destruction; callers' outer state remains required. |
| `Thread::try_new_with_retirement` → architecture user context | Retirement storage/generation precedes stack construction. Prepared thread publication occurs later; context `Box::try_new` can still reject after stacks exist. | That failure's implicit context/Stacks cleanup and every upstream guard/IRQ state remain open. |
| `grow_current_user_stack` → context/Stacks growth | Exact thread generation/ASID is checked. Growth uses the existing parent lease; fallback takes no guards and explicit local mapper rollback leaves the address-space guard. | The master thread-table write guard remains held through growth and ordinary physical rollback. Fault entry/masking and that enclosing guard are still G1 work. |
| Thread reaper → published context/Stacks destruction | Existing owner-LP active-stack/CPU checks and x86 scheduled reapers remain. | Published pair teardown remains physical work in Drop; Arm reaping precedes restoration of incoming IRQ state. No new recovery receipt/controller is provided. |

Both architecture walkers reject ordinary single-page mapping before leaf
publication; post-publication interruption is conservative retention. These
probes simulate interrupted states, not panic unwinding or hardware races.
This review does not complete the full R18-1 upstream/implicit-drop inventory.

## Guarded evidence and classification

New probes run with lifecycle, both address-space table guards, the physical
allocator and original stack admission pool held during Drop. They perform no
physical release, pool refund, root lookup or lease completion there.

For ordinary and platform admission, initial probes cover bare slot,
reservation-only preparation, allocated unpublished preparation and interrupted
publication. Growth probes cover reservation-only, allocated unpublished,
interrupted publication and explicit rejected physical rollback. The last checks
one allocator attempt; Drop cannot retry. Successful committed-prefix retirement
runs outside the probe guards and leaves the abandoned growth admission fenced.

The sixteen retained roots/slots consume 272 reservation pages, including 136
ordinary pages, and ten provisional physical frames. Private root hierarchies
also remain live because their exact leases remain admitted. Rejected root close
and subsequent slot admission are checked without force-clearing or decrementing
anything. No retained frame/charge is recovered by a later owner.

Normal cancellation is separately checked for raw slots, allocated initial pages,
invalid initial layout, initial allocator rejection, allocated growth pages and
growth allocator rejection. Free counts/admission return as expected and growth
parents stay usable. Existing actual foreign-leaf collision, 64-slot quota,
128-round thread preparation and launch rollback fixtures remain in the suites.

| Phase | Result |
| --- | --- |
| Ordinary unused admission / confirmed unpublished rollback | Explicit consuming cancellation; confirmed physical success precedes original admission release. |
| Initial/growth physical rejection | Terminal retention of original slot/root/reservation; no retry owner. |
| Initial or reservation-only abandonment | Terminal retention without cleanup; no implicit unused refund. |
| Growth publication/unused-preparation abandonment | Fence borrowed containing stack and retain original capacity, root and backing; no standalone receipt. |
| Published stack retirement | Existing physical cleanup and uncertainty rules; caller/Drop separation and complete owner-preserving retry remain open. |

## Validation

Final source passed:

| Check | Result | Local log |
| --- | --- | --- |
| Complete host harness | Passed, including 29 slot/lease probes and 13 signer tests. | `/private/tmp/charlotte-stack-preparation-host.log` |
| Kernel Clippy, both custom targets, `--locked -- -D warnings` | Passed. | `/private/tmp/charlotte-stack-preparation-clippy-{x86,arm}-final.log` |
| Intel VT-d with fresh storage | 15 passed, zero failed/pending. | `/private/tmp/charlotte-stack-preparation-intel.log` |
| AMD-Vi with fresh storage | 15 passed, zero failed/pending. | `/private/tmp/charlotte-stack-preparation-amd.log` |
| Arm SMMUv3 security suite with fresh storage | 19 passed, zero failed/pending; probe `0xffff`, publication generations 1/2 and 4,772 cancellation requests retired. | `/private/tmp/charlotte-stack-preparation-arm.log` |
| Formatting, whitespace and documentation | `cargo fmt --all -- --check`, `git diff --check`, relative links/anchors and unchanged 18-family/seven-gap map passed. | Local checks. |

Each runtime includes the guarded stack-preparation marker, original stack
failure diagnostics and thread-admission collision/quota/reuse evidence. Kernel
assembly permissions verified 249 x86 native entries and one Arm trampoline.
QEMU runs were sequential after host/Clippy work completed. No stack-lease
deadline expired; this does not explain or resolve the previous batch's failure.

Commands (existing signed service bundles reused; no userspace/ABI changes):

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance stack-preparation-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance stack-preparation-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18559 CATTEN_DEPLOY_HOST_PORT=17859 scripts/run-aarch64.sh --security-test --instance stack-preparation-arm-20261009 --fresh-storage --timeout 180
```

Executed kernel SHA-256:

- x86: `df8883f238efa47d8703ca9c2f62b703d2584e3c79bc9e3194298740d676e054`.
- Arm security: `c0e14e5e56b9c3a551d2e70b4c5b1ab3736f2389ac2635cbc6f393fe4850d5c0`.

The initial Clippy attempts found a new fixture file accidentally duplicated
from its parent module, causing a missing recursive module. The file was
corrected before final passing Clippy and all runtime executions. There were
no failed QEMU runs in this batch; prior failure evidence remains preserved.

## Limits and next boundary

The earlier Intel user-stack lease timeout remains unexplained. Its phase
observations remain active and its original deadline is unchanged. These fallback
changes do not clear retained leases, repair already abandoned owners or establish
that the timeout is resolved.

Published stack retirement still performs physical release from thread/context/
Stacks destruction. The complete pair, including root lease, bitmap slot and
original maximum charge, must be retained if that boundary is later made explicit;
a kernel range or scalar stack address alone is insufficient custody. Constructor
failures after publication, growth's enclosing master thread-table guard, Arm
reaper context and IOMMU preparation/backend serialization remain separate G1–G3
work. No new registry, generic retry API, supervisor reconciliation or operator
mutation policy is introduced.
