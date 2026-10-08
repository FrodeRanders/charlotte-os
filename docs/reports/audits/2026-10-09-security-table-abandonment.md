# Provisional tables/raw frames: abandonment and stack diagnostics

Date: 2026-10-09. Follow-up to the
[heap/image fallback correction](2026-10-09-security-preparation-abandonment.md).
This is scoped progress for [C17/G1/G2 and C16/G7](../../reference/cleanup-recovery.md),
not SEC-18 closure. The security ledger remains 20 corrected, six partial and
four open findings.

## Changes and ownership boundaries

`PreparingUserFrame::drop` no longer deallocates or logs. It retains abandoned
backing without acquiring any guard. Standalone raw owners are boot-fixture
foreign backing; their normal teardown now consumes the owner through explicit
release, after inactive-root destruction or confirmed detach/invalidation.
Heap/image/table/stack adapters retain the raw owner inside their existing
admission transactions. No frame is adopted from an address or reusable ASID.

`PreparingTable::drop` retains its original private/shared admission and frame.
Private retention only updates the exclusively borrowed account. The shared
charge retains its original pool reservation and records quarantine through an
atomic counter, replacing the pool-locking destructor. Reservation-only
abandonment consumes admission even with no physical frame. Original private
root destruction excludes quarantined charges from refund; successful shared
preparation cannot refund an earlier abandoned charge.

Normal allocation rejection explicitly cancels unused admission. Consuming
`cancel_unpublished` returns physical errors and consumes physical ownership
before allocator entry; rejected release is terminal and Drop does not retry.
Publication still disarms the frame before the architecture callback. No
fallible work is moved after publication, and no retry registry is added.

The production caller review covers x86 root/intermediate publication and Arm
root/intermediate publication. Arm lazy-root hardware-tag exhaustion occurs
after table preparation; it now explicitly cancels before returning. That
fixture checks the original account and physical free count as well as the
uninstalled root/tag. Constructor refunds and explicit cancellation still hold
their captured account/table context. This does not finish the outer caller/IRQ
inventory or separate ordinary rollback from masking table serialization.

Reviewed preparation boundaries:

| Source | Normal path after preparing backing | Exceptional path |
| --- | --- | --- |
| [x86 root construction](../../../crates/catten/src/cpu/isa/x86_64/memory/paging/mod.rs) | Initialize shared upper links, consume `publish`, then move the embedded account into the root. | No ordinary fallible branch after preparation; interrupted preparation retains frame/account. |
| [x86 intermediate construction](../../../crates/catten/src/cpu/isa/x86_64/memory/paging/pth_walker.rs) | Initialize and publish one parent entry; return its table pointer. | Admission/gate failure precedes allocation; interrupted publication retains admission. |
| [Arm root/intermediate construction](../../../crates/catten/src/cpu/isa/aarch64/memory/paging/walker.rs) | Publish initialized descriptor/root under the captured account borrow. | Lazy-root hardware-tag rejection explicitly cancels; unexpected interruption retains backing/admission. |
| Raw foreign-backing fixtures in translation, vmem, root release, stack growth and all three IOMMU walkers | Consume raw frame release after inactive-root/never-published domain destruction or explicit detach/invalidation. | Rejected release consumes ownership once; standalone fallback never frees. |

This reviews immediate preparation continuations, not every upstream caller's
outer IRQ/mask, account-refund interruption or implicit enclosing destructor.
The [cross-category ledger](../../reference/cleanup-recovery.md) retains those
G1/G2 qualifications.

Six atomic observations were added to existing user-stack retirement: started,
user backing released, identity rejection, leaf-detach rejection, invalidation
rejection and physical rejection. Only the fixture's existing deadline expiry
logs snapshots and the exact root identity, after guards leave. There is no
extra lock, allocation, callback or logging in the counter updates. Observations
are global, nontransactional and include injected failures; they cannot identify
an operation, authorize recovery or establish kernel/user pair completion.

## Fixtures and retention

Private and shared table probes each exercise physical rejection, interrupted
publication, ordinary unpublished abandonment and actual reservation-only
abandonment. During Drop the fixture holds both address-space guards, the
physical allocator and both private/shared admission pools. No release occurs.
Each category retains three frames and four original charges. The original
private root is destroyed and its retained charge remains consumed; the shared
fixture explicitly cancels a later preparation without refunding old quarantine.

Two raw-frame probes cover Drop under the same guards and rejected explicit
release. They retain two frames, with one attempt on physical rejection.
Together the new/current table/raw probes retain eight frames and eight table
charges; two table charges have no frame. Interrupted states are modeled,
not actual panic unwinding or hardware race injection.

Existing sparse table-prefix, cached reuse, private/shared alias, public memory
mapping and physical teardown fixtures still run. Normal raw fixture cleanup
and table cancellation preserve their previous free-count checks.

The stack fixtures now include explicit invalidation rejection. Its physical
release callback must never run; its exact root/slot and whole 17-page admission
remain held. Invalidation and physical rejection counters each advance once.
Six retained stack owners consume 102 reservation pages and 20 data frames,
including one live sixteen-page kernel stack, plus private root tables.

## Validation

Final source passed:

| Check | Result | Local log |
| --- | --- | --- |
| `scripts/run-host-tests.sh` | Complete host harness passed, including 29 slot/lease probes and 13 signer tests. | `/private/tmp/charlotte-table-drop-host.log` |
| Kernel Clippy, both custom targets, `--locked -- -D warnings` | Passed. | `/private/tmp/charlotte-table-drop-clippy-{x86,arm}-final.log` |
| Intel VT-d, fresh storage | 15 passed, zero failed/pending. | `/private/tmp/charlotte-table-drop-intel.log` |
| AMD-Vi, fresh storage | 15 passed, zero failed/pending. | `/private/tmp/charlotte-table-drop-amd.log` |
| Arm SMMUv3 security suite, fresh storage | 19 passed, zero failed/pending; security probe `0xffff`, publication generations 1/2, 4,720 concurrent cancellation requests retired. | `/private/tmp/charlotte-table-drop-arm.log` |
| Formatting, whitespace and documentation | `cargo fmt --all -- --check`, `git diff --check`; relative link targets and unchanged 18-family/seven-gap map checked. | Local checks. |

All three runs include the guarded raw/private/shared probes and the stack
invalidation/physical-rejection diagnostic marker. Arm additionally records
explicit hardware-tag-rejection cancellation. Kernel assembly checks verified
249 native x86 entries and one Arm trampoline. The QEMU executions ran
sequentially after host/Clippy work completed; the earlier stack-lease timeout
did not recur, which does not establish its cause or resolution.

Commands (existing signed service bundles were reused; no service/ABI changes):

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance table-drop-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance table-drop-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18549 CATTEN_DEPLOY_HOST_PORT=17849 scripts/run-aarch64.sh --security-test --instance table-drop-arm-20261009 --fresh-storage --timeout 180
```

Kernel SHA-256 after execution:

- x86: `95c10c3a08a186f89cc81c1debf4a965f0ff26c01af7d6e770652ccd6c783ea1`.
- Arm security: `e8a322d6b7719fbc993fb23dcdf4336b737fb6a385d87c35acb1572b4580d55d`.

The first Clippy invocation caught three fixture equality assertions against a
physical error type without `PartialEq`; these were changed to variant matching
before the final passing Clippy and QEMU runs. There were no failed QEMU runs in
this batch. The preceding report's failed Intel execution remains preserved.

## Remaining work

The initial Intel user-stack lease timeout in the preceding report remains
unresolved. Phase diagnostics improve evidence if it recurs; neither later
success nor these synthetic failures explain its cause. The deadline and root
counts are unchanged, and no fence is force-cleared.

C16 stack preparation/reaping still contains physical cleanup in destructors;
Arm's enclosing reaper IRQ context remains unqualified. C15 IOMMU preparation
and C18 publication/metadata destructor contexts remain separate work. Ordinary
CPU backing/table rollback still borrows its original table context. This patch
does not add shared recovery custody, operator policy, supervisor reconciliation,
active-I/O reset qualification or physical-platform evidence. SEC-18 remains
partial under its original deliverables.
