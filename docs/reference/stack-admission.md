# Stack backing admission

Thread preparation reserves its maximum data footprint before physical backing
allocation. This is independent of stack-usage telemetry, heap/image/object
pools and translation-table admission. User pages remain demand-backed; reserving
capacity does not eagerly allocate or promise physical frames.

| Scope | Reservation or ceiling |
| --- | --- |
| User thread | Configured maximum user pages plus 16 kernel-stack pages. |
| Owning user root | At most 64 stack slots; each user range admits at most 64 pages. Maximum combined footprint is 5,120 pages (20 MiB). Signed/adaptive limits can reduce this. |
| Kernel-only thread | 16 pages from trusted kernel admission. |
| Node stack pool | One eighth of usable RAM, rounded down to pages, minimum one. |
| Ordinary user domains | Three quarters of that node pool; the final quarter is trusted-platform headroom. |

This is an initial kernel policy, not a configurable application entitlement or
a measured worst-case service requirement. Trusted platform classification uses
the captured root identity and existing kernel policy, never a caller-supplied
name, role or flag. The reservation captures its ordinary/platform class for
refund; later ASID reuse or policy promotion cannot reinterpret it. Inherited
boot/CPU/interrupt stacks predate runtime thread preparation and remain outside
this pool. General kernel heap and thread/control-block metadata have separate
admission needs. The independent pools are not a complete RAM ledger.

## Ownership and physical completion

`StackSlot` owns the original root lease, exclusive bitmap slot, captured user
capacity and the complete user-plus-kernel reservation. `Stacks` owns both
physical ranges for user threads, or the kernel range and reservation for
kernel-only threads. Both architecture contexts retain that owner through
construction, publication, execution and off-CPU retirement. There is no scalar
stack cleanup ladder when kernel preparation fails after user mapping.

Ordinary unused admission is cancelled explicitly: `StackSlot::cancel_unpublished`
checks that no backing was published, then consumes the exact slot/lease/charge.
`PreparingStackPage::cancel_unpublished` first consumes its frame before physical
release and reports physical rejection separately from slot completion rejection.
Constructor allocation rejection, invalid layout and confirmed mapper rejection
use explicit cancellation; the mapper's local table guard leaves before release.
Unused reservations refund without backing release. Otherwise, refund requires
confirmed removal, invalidation and physical release of both ranges. User
cleanup detaches under the original table/generation guard and invalidates after
releasing it. Kernel cleanup uses `RetiredKernelRange` after arena/table guards
are gone. Unpublished kernel frames rejected by mapping also enter that receipt;
their release error cannot be hidden by a frame destructor. Kernel allocation
reports incomplete rollback or rejected post-publication invalidation to the
owning stack transaction, which retains its reservation.

Any failed or abandoned physical step retains the whole maximum reservation,
even if other pages were released. User threads also retain their bitmap slot
and exact root lease. The domain ceiling is therefore still consumed, and root
teardown/reusable-ASID lookup cannot refund the charge. Kernel-only failure
retains the node reservation and uncertain backing. There is no force-clear,
administrative reclamation or retry of consumed physical release.

## Provisional abandonment

Initial page preparation, growth preparation and implicit `StackSlot` field
destruction acquire no allocator, table, lifecycle or admission-pool guard and
enter no callback/logger. They retain original backing/admission even when
abandoned before allocation. An unpublished slot is not refunded by Drop; only
ordinary explicit cancellation can release it. Dropping the operation and charge
tokens retains the original root count and complete maximum reservation.

Growth fallback marks its exclusively borrowed parent slot uncertain and
quarantines any provisional frame. A later successful committed-prefix release
cannot discharge that uncertainty, root lease or reservation. A rejected
explicit provisional release consumes frame ownership once and leaves the same
fence. Only confirmed explicit rollback can clear the fence that it armed;
already uncertain stacks reject new growth. Normal growth allocation rejection
completes its unused preparation explicitly and leaves the parent usable.

This is terminal retention, not a deferred retry owner or reclamation API.
Published pair release is now explicit and arms a one-shot phase fence before
any physical work. Stack/context field destruction only retains backing and
admission; it never enters allocator/table/pool guards or logging. Both
architectures use pinned IRQ-enabled reapers. Failed pairs retain their entire
thread/context in the original retirement node and cannot be retried by later
scans. Ordinary constructor/publication/submission rejection explicitly releases
a never-admitted pair after local serialization leaves.
Growth's production `grow_current_user_stack` keeps the master thread-table
write guard through the operation, including ordinary rollback. Outer constructor/syscall masks and general thread metadata fallback
remain [C16/C17/G1/G2](cleanup-recovery.md).

## Demand growth

Growth borrows the existing `UserStack` and its `StackSlot`, whose reservation
already covers the full configured capacity. Each provisional page has a
physical owner through zeroing and mapping. Every mapping revalidates the exact
root generation; publication records the committed count before relinquishing
the provisional owner. The address must remain within that slot's captured
capacity, independently of later launch-limit changes.

The existing one-sixteenth physical growth reserve is checked atomically with
each allocation. The initial user page retains the existing one-eighth physical
floor. Translation tables have their own budgets and can still reject mapping.
An ordinary rejected leaf releases unpublished backing explicitly; failed
release or interrupted publication permanently marks the slot uncertain.
Subsequent successful cleanup of the ordinary committed prefix cannot return
that slot, root lease or reservation. Growth failure preserves exact partial
progress for the fatal path to retire.

## Verification and limits

Serialized boot fixtures exercise real stack pairs, initial rejection before
allocation, growth with a foreign-leaf collision, partial progress and retry,
cached slot reuse, exact reservation/free-count restoration, and platform/kernel
progress under ordinary admission pressure. Existing thread-publication,
128-round preparation/explicit-release churn and launch rollback fixtures still run.

Six failed/abandoned fixtures retain six original roots/slots and 102 reservation
pages. They retain four provisional/user data frames and one sixteen-page live
kernel stack, plus their roots' private table hierarchies. The kernel cleanup
rejection, incomplete preparation marker and interrupted growth are injected
owner states, not physical hardware failures or actual panic unwinding. Their
backing remains unavailable for the lifetime of the guest. The additional
invalidation-rejection fixture never invokes physical release and retains its
exact slot, root lease and complete reservation.

Sixteen additional preparation fixtures cover ordinary and platform admission:
bare slot, reservation-only initial page, unpublished page, interrupted initial
publication, reservation-only growth, unpublished growth, interrupted growth
publication and rejected growth physical rollback. Drop holds lifecycle, both
address-space guards, physical allocator and the original stack admission pool.
They retain 272 reservation pages (136 ordinary) and ten provisional frames,
plus sixteen exact roots and their private hierarchies. Successful committed-page
cleanup occurs outside those probe guards and cannot refund abandoned growth
admission. Root close stays busy and quota admission stays rejected.

Normal cancellation, invalid-layout/allocator rejection, growth cancellation and
growth allocation rejection restore the expected free counts and leave usable
parents. Existing actual foreign-leaf collision, 64-slot cancellation, successor
identity and 128-round thread preparation fixtures still run. Interruption is
modeled, not real panic unwinding. See the
[preparation report](../reports/audits/2026-10-09-security-stack-preparation-abandonment.md).

Six atomic user-retirement observations track starts, successful user release,
identity rejection, detach rejection, invalidation rejection and rejected
physical releases. They take no lock, allocate no storage and enter no logger.
Snapshots are independent counter reads, include injected boot failures and
cannot identify a particular operation or authorize retry/refund. They describe
the user half, not completion of the kernel/user pair. The user-isolation
fixture logs before/after snapshots and its exact root identity only when its
existing retirement deadline expires, after subsystem guards leave. The prior
Intel timeout remains unresolved; this adds evidence, not a recovery bypass.
See the [follow-up report](../reports/audits/2026-10-09-security-table-abandonment.md).

See the [audit record](../reports/audits/2026-10-07-security-stack-admission.md)
for QEMU evidence. This establishes runtime stack capacity admission, not
complete node exhaustion isolation, metadata budgeting, boot-stack accounting,
physical-platform quiescence or abandoned-owner recovery.

Two additional ordinary/platform published-pair abandonment fixtures drop while
holding lifecycle, both table guards, allocator and stack admission pool. They
retain two exact roots/slots, 34 reservation/data pages and both mapped ranges,
without incrementing retirement phase counters. A kernel-only failed pair retains
sixteen mapped pages in its existing thread retirement node through repeated
scans. Successful pair release also rejects a second attempt before callbacks
and prevents further growth. See [published-pair evidence](../reports/audits/2026-10-09-security-published-stack-retirement.md).

The Intel timeout reproduced in the initial IOMMU preparation follow-up. A staged
node now retains its reported pair error alongside its complete owner. Timeout
logging copies the exact thread/root/LP, started fence and error outside staging
serialization; kernel outcomes distinguish invalid stack, detach, physical and
unconfirmed retirement. Independent global x86 shootdown counters report success,
busy, masked, exhausted, delivery rejection and timeout. They have no operation
identity. No diagnostic value or missing staged snapshot grants retry/completion,
and the original deadline remains unchanged. See the
[IOMMU follow-up](../reports/audits/2026-10-09-security-iommu-preparation-abandonment.md).
