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

The owning charge remains attached through removal and post-guard destruction. Backing is
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

Successful replacement returns a `RetiredCq` containing the complete admitted
map node and old `CqState` out of registry serialization, retaining its
ring/backlog, subscriptions and original charges. Explicit finish frees the
metadata node before destroying the payload and refunding its charge.
Whole-namespace close/replacement does the same with the complete namespace
after serialized authority removal. Physical rings retain their original mapping owner
and quiescent-consumer requirement. See the
[callback/source audit](../reports/audits/2026-10-15-completion-callback-preparation.md).

CQ payload staging now captures the exact namespace root, CQ account,
submission capacity and trusted platform classification under a short registry
borrow. Count/byte admission, fallible map-node preparation, waiter-list
preparation, backlog reservation and heap-ring allocation run outside
`COMPLETIONS`. Partial failures release their
original storage and charges there. Publication borrows the preparation,
rechecks root/account/capacity and retirement, then checks physical aliases
under the same registry hold as ring initialization and replacement. A stale
namespace rejects without crediting its successor or touching its queue.

Physical preparation validates capacity/alignment but leaves the borrowed frame
untouched. Only successful serialized publication initializes it; rejection
retains the complete preparation until unlock. Whole-namespace preparation owns
the complete private namespace and its fallibly prepared CQ map node before
entering the registry. Fresh CQ publication relinks that node without allocation;
replacement detaches the old node without freeing it. Both unpublished and
retired queue fallback retain every field without cleanup; ordinary rejection
explicitly finishes after local guards. One queue registration covers its one
prepared node, but the byte counter still measures ring/backlog storage, not
aggregate metadata bytes. Namespace replacement still revokes current authority
under serialization. See the
[CQ payload-preparation follow-up](../reports/audits/2026-10-16-cq-payload-preparation.md).

The [CQ metadata follow-up](../reports/audits/2026-10-16-cq-metadata.md) covers
fresh queue nodes, complete replacement and explicit namespace draining.
The [global namespace metadata follow-up](../reports/audits/2026-10-16-completion-namespace-metadata.md)
prepares its global node and four original account allocations fallibly outside
the registry, together with the complete private namespace and any default CQ.
Publication relinks without allocation after validation and physical alias
checks; there is no later admission rejection after physical initialization.
Close/replacement carries the global node and complete payload in
`RetiredNamespace`; explicit finish frees that node before payload/account
disposal. Unpublished/retired fallback retains every field. Bare kernel/demo
and mandatory fixture setup uses the same preparation through a panic wrapper;
runtime CQ setup returns typed failure. Wider timer/observer disposal and
arbitrary outer caller/IRQ contexts remain separate. This does not establish
that every registry operation is allocation-free or that a captured root
identity pins physical backing.
The caller retains the original frame/mapping and must
quiesce its consumer through publication and use.

The service loader owns preparation in one rollback guard. If a CQ fails,
teardown closes already-installed queues and releases the partially mapped
address space before returning a typed `DomainLoadError`. Trusted ambient
supervisor paths designate platform domains before CQ preparation; scoped
and syscall preparation use ordinary admission. The boot convenience wrapper
still panics on a failed mandatory load. ELF/runtime-page allocation itself
is not yet fully fallible; this guard does not recover from allocator panic.

Payload admission runs outside the registry and takes domain then node budget
guards, neither of which allocates or enters another subsystem. Platform
identity is captured before the registry borrow and matched to its stored
generation; retirement is checked at capture and again before publication. Loader rollback enters
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
rounding. New native probes inject list/ring preparation rejection, reenter the
registry at actual allocation boundaries, and hold the heap during replacement
or stale/retired/alias publication. They check original charges, private
namespace replacement and unchanged physical bytes containing a pending result.
Metadata controls inject node rejection before list/ring allocation, reject a
33rd queue before allocation, and publish/remove all 32 fresh queue nodes with
the heap locked. Replacement retains an old ring, backlog record and waiter
through namespace reuse; explicit disposal credits only the original accounts.
Guarded preparation/retired-owner abandonment deliberately retains two queues
and one backlog record until guest shutdown, without resetting their counters.
Global metadata controls reject each of five allocation stages without changing
the live namespace, publish/remove 64 fresh namespaces with the heap held, and
exercise complete replacement with retained timer, buffer, backlog, watch and
authority. Disposal reenters the registry after unlock; numeric capability and
actual root-generation reuse leave successor accounts unchanged. Existing
guarded namespace abandonment controls now retain the global node too.
This is injected rejection rather than physical allocator exhaustion; exhaustive
physical-frame leak measurement remains open.

Other than separately bounded [endpoint-close watches](close-watch-budgets.md),
observer queues, weak-only Arc/control-block storage, registry/capability
metadata, worker stacks, other timers, loader/page tables and general kernel
heap allocation still need bounds and fallible lifetimes. In particular, global
namespace nodes and their four account allocations have no separate aggregate
count/byte budget here; queue/record admission does not cover empty namespaces.
Sorted global map lookup is linear in retained live namespaces, with no new
namespace-count ceiling claimed. Per-principal totals
across domains, typed deployment limits and userspace budget counters also
remain open. SEC-07 remains partially implemented; these checks are not a
complete hostile-workload containment guarantee.
