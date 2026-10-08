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
128-round preparation/Drop churn and launch rollback fixtures still run.

Six failed/abandoned fixtures retain six original roots/slots and 102 reservation
pages. They retain four provisional/user data frames and one sixteen-page live
kernel stack, plus their roots' private table hierarchies. The kernel cleanup
rejection, incomplete preparation marker and interrupted growth are injected
owner states, not physical hardware failures or actual panic unwinding. Their
backing remains unavailable for the lifetime of the guest. The additional
invalidation-rejection fixture never invokes physical release and retains its
exact slot, root lease and complete reservation.

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
