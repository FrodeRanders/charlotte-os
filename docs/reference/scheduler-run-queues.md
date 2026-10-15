# Inline scheduler run queues

Ready membership lives in each complete `ThreadPayload`, in a fixed
`QueueLink`. LP schedulers retain only a queue identity, head, tail and count;
the current handle is a separate borrowed exact-generation identity. There is
no growable ring, per-enqueue allocation, detached queue node or new registry.

## Admission and lifetime

The link is part of the existing thread payload/retirement-node and thread-table
storage. Thread preparation and ID-table capacity remain fallible before
publication. Once published, every thread already has its queue storage, so
wakeup, requeue and migration cannot encounter queue-capacity allocation.
Failure of existing thread/node/context preparation still precedes publication;
no queue reservation or cleanup ladder is added.

This is fixed metadata per admitted thread, not a complete byte/principal
budget. Master-table vector preparation still has its existing outer guards;
general thread-node/heap admission and quantum-timer metadata remain separate
SEC-07/18 work. [Waiter preparation and cancellation](scheduler-waiter-budgets.md)
now leave the scheduler's local LP/table guards; wider source/outer contexts
remain separate. No domain account is reclassified and
stack/root/retirement charges retain their original owners.
Byte accounting must include both live table capacity and the prepared future
retirement node's payload storage, even though only one link is active in the
unique thread owner. Fixed link size alone is not the total metadata cost.

Queue heads and neighbors carry TID plus the thread's nonreused generation.
Lookups resolve the original inline link under the borrowed table, never a
pointer into its moving vectors. A monotonic, nonreused queue identity in each
link distinguishes even two queue instances on one LP. Queue identities convey
membership only, never cleanup or restart authority.

## Publication and removal

All operations retain system/LP/table serialization in its established order.
Push, pop and exact removal inspect a constant number of endpoints/neighbors.
Validation rejects a missing generation, duplicate/foreign membership or
inconsistent local linkage before mutation. Candidate traversal retains a
captured count bound and reports stale/corrupt linkage without adopting a
successor or treating it as a cleanup result.

`RoundRobin::next` borrows the table through outgoing requeue and selection.
Popping an aborted candidate clears readiness membership while leaving its
complete thread in the table for owner-LP retirement. An early wake may make
the outgoing context Ready and queue it before its save; queue removal retains
that borrowed current handle until the architecture handoff completes.

Ready blocking and explicit abort pass their existing table borrow into queue
removal instead of reacquiring it. Owner-LP requested abort takes its LP guard
before the table and unlinks an exact queued owner before extraction/staging.
Remote abort revalidates its notification LP under the captured generation's
request-publication guard, so an older pre-migration LP observation cannot
authorize the wrong queue's retirement.

## Migration transaction

`migrate_ready` validates the exact source generation, Ready LP, certification,
pin/wait/abort/execution fences and architecture policy. The source then checks
its original neighbors and the destination's endpoints before unlinking.
Ordinary rejection leaves both queues, state and affinity unchanged.

After validation, removal, destination append and Ready/affinity publication
are scalar updates beneath the same exclusive table and both LP guards. No
allocation, callback, destructor or guard release enters that splice. The LP
load observations update before local guards leave; the scheduler sends the
destination IPI afterward. Arm still needs its CPU-ownership handoff; x86 still
rejects every already dispatched context. No new migration authority is added.

## Cleanup and abandonment

Queue membership is not thread quiescence. Stack retirement returns
`RunQueueLinked` before metadata notification or physical teardown; unstarted
release returns its complete owner while linked. Reaper readiness also rejects
linked owners. Only explicit unlink permits the existing retirement policy to
consider its independent executing-stack and CPU-ownership proofs.

Dropping a queue clears no parent link and performs no cleanup. A replacement
queue cannot adopt that membership. The complete thread's existing fallback
retains context, roots, stack backing, original charges and inline membership.
Corrupt or abandoned linkage is terminal retention, not an ID-based retry
owner. This adds no custody registry, force-clear, physical replay or recovery
permission.

## Evidence and scope

The [implementation audit](../reports/audits/2026-10-15-inline-scheduler-run-queues.md)
records allocation-traced FIFO/model tests, stale/foreign and destination
rejection, actual heap-held kernel transaction probes, linked-owner cleanup
rejection, and the bounded native matrix. These qualify queue storage and the
listed caller ordering; they do not complete C16/G1/G4/G7, prove essential
progress under broad pressure or attribute historical timeouts.
