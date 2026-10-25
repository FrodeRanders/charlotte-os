# Thread context and retirement-node admission

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

Creating a thread reserves one complete metadata bundle before either heap
allocation, generation claim, stack preparation or scheduler publication. The
bundle covers the exact requested layouts of its stable `Box<ThreadContext>`
and preallocated retirement node, including the node's charge field.

| Dimension | Whole node | Ordinary work | Each verified signed principal |
| --- | --- | --- | --- |
| Requested heap bytes | 8 MiB | 6 MiB | 1 MiB |
| Complete bundles | 4096 | 3072 | 128 |

Every dimension must fit. These fixed kernel ceilings have no userspace setter
or deployment override. They are independent of stack/frame admission and the
[principal backing limit](principal-backing-admission.md). A prepaid context
consumes its full layout even while uninitialized. This reserves allowance;
it does not promise allocation success or available physical stacks.

## Sponsorship and lifetime

Preparation validates the captured address-space generation and live memory
account under the same table/ledger hold. It captures the signed principal and
trusted ordinary/platform classification there. Signed launch installs identity
before constructing threads. Kernel-only and trusted raw unsigned fixture work
still consume node admission without inventing a signed application principal.

The existing boot-lifetime principal-history record holds the new byte/count
counter. Instances and restarted generations share it. No per-instance registry
or separately allocated sponsor is added. Admission and final refund allocate
nothing and do not look up the current occupant of a numeric ASID. Later trusted
promotion cannot change an existing bundle's classification or refund its
principal charge.

One typed charge travels in the existing retirement node. The context retains
its ordinary stable-address Box, preserving assembly's saved-context contract.
Live threads keep their empty node; staging moves that node around the complete
thread. Reaping does not allocate replacement storage.

| Outcome | Disposition |
| --- | --- |
| Byte/count denial | `ThreadPreparationFailed`, before either allocator, generation or physical stack work. |
| Node/context allocation rejection | Dispose unused storage, then refund its complete reservation. |
| Stack/construction/publication rejection | Existing explicit unstarted cleanup must confirm pair release before destroying context/node and refunding. |
| Staged, deferred, failed or abandoned thread | Retain the same node, context, root/stack owners and original charge. |
| Confirmed pair retirement | Dispose the typed payload and context, deallocate the node, then refund the original byte/count charge. |
| Root retirement or ASID reuse | No metadata refund. A late release still addresses the original principal. |

The shared retirement collection accepts an optional typed charge parameter.
Its zero-sized default preserves uncharged node layouts. Empty preparation
releases normally; published-node/list fallback retains storage and charge.
Explicit disposal keeps admission through the callback and actual node
free. Interruption retains it. For a charge covering payload allocations, the
typed callback must also confirm their disposal; the thread family uses
`Thread::dispose_retired` after successful pair release.

These remain C16 owners in [cleanup/recovery](cleanup-recovery.md). No new physical
completion proof, recovery custody, retry permission or observer readiness is
introduced. Production reapers remain owner-LP and IRQ-qualified. Ordinary
constructor failure and explicit metadata release retain their existing outer
context requirements; arbitrary caller guards/destructors are not qualified by
this counter.

## Evidence and remaining scope

The [audit follow-up](../reports/audits/2026-10-23-security-thread-metadata.md)
records real root/context/node controls, ordinary-pool exhaustion, kernel reserve
admission, late release after ASID reuse, independent byte/count denial and
abandonment under accounting/root/allocator guards. Host tracing checks refund
after actual node deallocation. The finite `CharlotteThreadMetadata` projection
checks admission and disposal ordering, with four expected unsafe controls.

This pool covers two requested layouts per thread. It excludes the master
thread table's retained vector capacity, exit-observer lists and weak references,
waiter/timer/control-block allocations, root/principal-history storage, other
kernel metadata, allocator bookkeeping/rounding and the physical heap arena.
No claim is made that thread-table capacity is refunded at thread exit.
Principal-history nodes have their own
[bounded pool](principal-history-admission.md).

A single kernel-thread admission beside serialized ordinary exhaustion supplies
narrow reserve evidence. It does not demonstrate sustained concurrent application
fairness, bounded IRQ holds or management/timer/teardown latency under combined
pressure. SEC-07 R7-1 through R7-5 and the wider SEC-18 context audit remain open
in their documented scopes.
