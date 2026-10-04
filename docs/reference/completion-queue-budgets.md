# Completion-queue admission

CQ registration reserves both a queue count and kernel-owned backing bytes.
These resources have different lifetimes from [completion records](completion-record-budgets.md).

| Limit | Registered queues | Kernel-owned backing bytes |
| --- | ---: | ---: |
| Per completion namespace | 32 | 256 KiB |
| Per node | 2,048 | 4 MiB |
| Ordinary-domain share | 1,536 | 3 MiB |

The remainder is available to kernel-designated platform domains. It is a
shared reserve, not a per-service progress guarantee. Artifact roles, names
and application configuration do not select this designation. There is no
application-facing CQ creation API or signed deployment override yet.

## What is charged

Each queue reserves one registration and its preallocated backlog's bytes.
Backlog capacity is rounded up to a power of two, with a minimum of four for
nonempty queues; checked arithmetic rejects overflow. A namespace's configured
submission capacity is clamped to 1,024 before allocation. A heap-backed ring
adds 4 KiB, allocated as `Vec<u64>` to guarantee the ring's alignment. Allocation
uses `try_reserve_exact`; unexpectedly larger capacity is rejected rather than
retained without a charge.

A physical ring belongs to its address-space mapping, not to the CQ registry.
Its queue count and kernel backlog are charged here, but its mapped frame is
not included in the heap-byte counter. General physical-page and loader
admission remain separate work. Replacing a physical queue does not unmap its
old frame; do not interpret a registration count as a bound on all mapped pages.

The owning charge remains attached to the queue until removal. Backing is
released before admission is returned. Each namespace has a reference-counted
budget owner tied to its captured generation, so old releases cannot credit a
new namespace using the same ASID.

## Fallible preparation and replacement

Kernel CQ creation returns `CqOpenError`: unknown or retiring namespace,
resource limit, allocation failure, invalid ring capacity, misaligned physical
frame, or a frame already registered to another queue. All admission and
backlog allocation checks precede physical-ring initialization. Another
namespace or queue cannot register the same physical ring; replacement of the
same queue may reuse its frame after the caller quiesces the consumer.

Replacement stages the new backing while the old queue remains registered.
Peak old-plus-new admission must fit; being at a limit can therefore prevent
replacement. Failure preserves the old queue, capabilities, pending results
and counters. Successful queue replacement discards old undelivered results
and returns detached submission slots, as described in the record reference.
Whole-namespace replacement publishes only after CQ preparation succeeds and
then revokes the old completion capabilities. This is kernel-controlled
teardown, not a lossless live-migration API.

The service loader owns preparation in one rollback guard. If a CQ fails,
teardown closes already-installed queues and releases the partially mapped
address space before returning a typed `DomainLoadError`. Trusted ambient
supervisor paths designate platform domains before CQ preparation; scoped
and syscall preparation use ordinary admission. The boot convenience wrapper
still panics on a failed mandatory load. ELF/runtime-page allocation itself
is not yet fully fallible; this guard does not recover from allocator panic.

Lock order is completion registry, domain budget, then node budget. Budget
guards neither allocate nor enter another subsystem. Platform identity is
captured before taking the registry lock and matched against its stored
generation; retirement is checked during admission. Loader rollback enters
lifecycle teardown only after CQ setup has returned and released the registry.

## Verification and remaining work

Synchronous guest tests cover domain count and byte exhaustion, both ordinary
and total node dimensions, rollback, invalid capacity, failed replacement
preserving pending data and capabilities, aligned heap backing, physical-frame
admission without writes, alias rejection, same-queue frame reuse, retirement
and ASID reuse. Loading a real signed bootstrap image with only two ordinary
registration slots available exercises failure partway through its five CQs;
the test checks returned charges and ASID reuse, then verifies trusted platform
preparation can still succeed. Pool exhaustion uses reservations, not allocation
of the maximum corresponding backing footprint. Host tests exercise checked
rounding. No allocator-failure injection or exhaustive physical-frame leak
measurement is claimed.

Observer queues, weak-only Arc/control-block storage, registry/capability
metadata, worker stacks, other timers, loader/page tables and general kernel
heap allocation still need bounds and fallible lifetimes. Per-principal totals
across domains, typed deployment limits and userspace budget counters also
remain open. SEC-07 remains partially implemented; these checks are not a
complete hostile-workload containment guarantee.
