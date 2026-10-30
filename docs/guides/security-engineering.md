# Security engineering lessons

This guide captures reusable knowledge from CharlotteOS's security work. Read it
when designing or reviewing a resource operation, a service reactor or a trust
boundary. The [subsystem contributor contracts](../reference/contributor-contracts.md)
carry the detailed implementation requirements. The
[security ledger](../reference/security-remediation.md) tracks unfinished work;
[dated reports](../reports/README.md) retain particular experiments and incidents.

## Own the complete operation

A capability handle, a backing allocation and a root lease each protect different
things. A safe multi-step operation owns all of them together, including temporary
metadata, authority reservations, borrowed memory and its current phase. Keep
that ownership through every fallible call and publication boundary.

For example, cancelling an IPC loan must retain both exact roots, reply/call
authority and the revocation pin until cleanup is confirmed. Saving the memory
object ID and retaining its frames cannot reconstruct that transaction later.
Make transfer and finalization consume the actual Rust owner. Return that owner
on rejection when its contract supports continuation.

Use `catten_rt::owned` and protocol-specific owners in userspace. Local infallible
cleanup can use ordinary RAII. Fallible remote close needs a consuming explicit
operation and its documented fallback. Kernel owners used beneath unknown locks
often retain all fields on abandonment; that policy must be explicit. Merely
forgetting a resource does not provide recovery.

Read [userspace ownership](resource-ownership.md),
[cleanup composition](../reference/cleanup-recovery.md#composition-order) and
the [transfer owner](../../crates/catten/src/memory/object/transfer.rs).

## Admit storage before publication

Count limits do not make an infallible map insertion safe under heap exhaustion.
Prepare fallible payload storage, authority and retirement metadata before
changing public state. Under serialization, revalidate the captured identity and
relink already prepared nodes. Carry losing preparation out of the guard for
explicit disposal.

IPC illustrates the composition requirement: publishing a moved memory capability
before admitting its call record creates an ownership gap on the second failure.
Admit both, commit them together and keep delivery hidden until receive or first
reply observation. A readiness notification does not publish the attachment.

Budget the actual allocation lifetime, including weak references, unused
candidates and detached or quarantined storage. Deallocate before refunding its
charge. Moving an object, retiring its sponsor or reusing an ASID must not make
live backing free in the accounts. Keep the original trusted/ordinary class.

Read [capability admission](../reference/capability-admission.md),
[IPC record budgets](../reference/ipc-record-budgets.md),
[principal backing](../reference/principal-backing-admission.md) and
[resource policy](../reference/resource-policy.md).

## Capture identity and authority together

Reusable ASIDs, TIDs and capability numbers identify slots. Capture the generation
and retain its owner before leaving the registry that established the identity.
A reply/cancellation claim also has an epoch: rollback and a later claim on the
same token are separate episodes.

For example, a late transfer cancellation must touch only its captured source;
looking up the same ASID/capability pair can find a successor. Authorize from the
authenticated IPC envelope or verified signed descriptor. Names, role strings,
discovered MACs and third-party lookups cannot supply retirement authority or
platform admission privileges.

Read [live root operations](../reference/live-address-space-operations.md),
[IPC receive](../reference/ipc-receive.md),
[scoped deployment](../reference/scoped-deployment-launch.md) and
[execution targets](../reference/execution-targets.md).

## Separate waiting from completion and replay

Carry one monotonic deadline from preparation through observation. After preparing
watchdog/subscription storage, recompute the remaining budget. Unrelated wakes,
new reactor phases and routing results must not restart it.

At expiry, recheck the actual producer state through the retained owner before
classifying a timeout. Completion can arrive while the clock is being read.
Keep that final observation and its failure diagnostic together; a separate
later sample can describe a different state.

| Observation | Required treatment |
| --- | --- |
| Submission rejected before acceptance | Follow the typed unstarted-rejection contract; retain or explicitly cancel the original preparation. |
| Accepted operation reaches its deadline | Retain accepted work and every borrow through checked cancellation or completion. Its outcome can remain uncertain. |
| Waiter/watchdog admission rejected | Preserve a distinct error and the pending owner. It establishes neither expiry nor terminal completion. |
| Readiness or CQ notification arrives | Recheck the exact current owner and observe its result before adopting returned resources. |
| Positive output prefix confirmed | Advance that prefix using the same output owner; retain the unsent remainder. |

An accepted mutation may commit after its observer times out or its credential
expires. A second submission can duplicate it. Retry needs an explicit protocol
or an owner that proves the operation never started; a busy-looking diagnostic
after work began is insufficient.

Read [checked CQ waits](../reference/cq-wait.md),
[status observations](../reference/status-observation.md) and
[DMA ownership](resource-ownership.md#dma-unmap-completion-and-rejection).

## Connect readiness to its producer

Register interest in the exact resource that can unblock the operation, then
recheck its state before parking. Notify after producer guards leave. Retain and
cancel competing source/watchdog registrations through the same owning wait.

A reactor serving several sources must observe all of them. Bind accepted replies
and endpoints to its CQ, wait for the earliest remaining deadline and recheck
lifecycle after wakes. Waiting on an unrelated CQ cannot replace reply polling
unless the producer actually signals it.

Deferred notifications need a delivery boundary on every relevant control path.
Delivering only in an older thread's yield continuation misses fresh thread entry;
parking before flushing the producer LP can strand its dependent worker. Incoming
stack delivery must preserve the architecture's IRQ and FP/SIMD handoff contract.
Readiness still grants no physical-release authority.

Read [scheduler waiter budgets](../reference/scheduler-waiter-budgets.md),
[thread retirement](../reference/thread-retirement.md) and
[scheduler state machines](../reference/scheduler-state-machines.md).

## Qualify the caller's cleanup context

Leaving a helper's own lock does not establish that cleanup is safe. Its caller
may still hold IPC, lifecycle, an allocator guard or an IRQ mask. Rust temporary
lifetimes and implicit field destruction can extend that context beyond the
visible scope. Bind a guard-producing result before matching when the scrutinee
would otherwise keep a guard alive through cleanup.

Review the complete call chain, including rejection, panic and abandoned
preparation. Prepare storage before serialization, detach into one owner, leave
the guards and explicitly finalize in the qualified context. Preserve an outer
IRQ mask; never enable interrupts just to make a blocking helper work.

Changing an inner owner's destructor to retain backing does not qualify an active
containing destructor. That parent can still enter physical release or destroy
metadata. Test the relevant parent path with the actual enclosing guards held.

Capture failure diagnostics alongside the original operation before entering
the failing phase. A reporting helper that looks up the registry again can
deadlock beneath a caller's retained guard. Copied identity/phase fields can
explain a failure without granting cleanup or retry authority.

Read [locking](../reference/locking.md),
[cleanup gates G1/G4](../reference/cleanup-recovery.md#named-gaps-and-acceptance-gates)
and [publication boundaries](../reference/cleanup-recovery-details.md#c18-publication-rollback-and-detached-memory).

## Require the right physical completion proof

CPU TLB invalidation, thread quiescence, DMA drain, scratch release and a logical
socket-close reply establish different facts. Preserve each dependency until its
own proof permits release. A worker-return marker, missing registry entry or
timeout cannot substitute for that proof.

Record the destructive phase before hardware or physical release starts. Only a
complete retained owner with an explicitly retryable phase can continue. Once a
physical walk has partly returned frames, replaying its old addresses can free
reallocated memory. Failed reset or maintenance must preserve requester fences,
command-engine state and original charges.

Quarantine preserves safety while consuming capacity. Recovery requires eligible
complete ownership and separately authorized policy. Keep unsupported recovery
paths fenced rather than manufacturing completion by clearing counters.

Read [kernel frame retirement](../reference/kernel-frame-retirement.md),
[hardware quiescence](../reference/hardware-quiescence.md) and
[root recovery](../reference/root-recovery.md).

## Specify shared-memory accesses precisely

An owned mapping does not make a concurrent wire protocol correct. Define field
width, alignment, byte order, who may write each field and the ordering that
transfers a slot between producer and consumer.

The RNG failure demonstrated why a live 16-bit index must not be read or published
as two bytes: transition from `0x00ff` to `0x0100` can expose a torn `0x01ff`.
Checked aligned halfword access addresses that boundary; payload visibility still
needs the separate descriptor/completion ordering contract.

For a CQ, the producer acquires `tail` before reuse and releases `head` after
initializing a record. The sole consumer acquires `head`, copies the record and
then releases `tail`. A fence after publishing `tail` is too late. Independent
readers need external serialization even when one library serializes its own
readers. Kernel bounds remain private; writable user mirrors cannot authorize
kernel accesses.

Read [entropy](../reference/entropy.md) and the current
[CQ ring definitions](../../crates/catten-syscall/src/lib.rs). The focused
[RNG](../reports/audits/2026-10-28-security-rng-ring-access.md) and
[CQ](../reports/audits/2026-10-29-cq-ring-ordering.md) reports retain the examples.

## Reject incomplete protocols and persistent state

Use one complete bounded internal grammar. Validate counts, lengths, full
identities, attachments and tags before mutation. A clamped prefix or missing
field default can change identity or authority while appearing successful.
Update parser, encoder, SDK, fixtures and living documentation together.

Distinguish an absent store from unreadable, corrupt, unsupported or incomplete
metadata. For example, only two successfully read zero-filled object-store
superblock slots authorize first-boot formatting. Restore rejection must leave
live application state unchanged and stop Raft publication. Format changes to
deployed stores require an explicit interruption-tested migration preserving
committed and election state.

Read [protocol simplification](../reference/api-simplification.md),
[object-store format](../reference/object-store-format.md),
[catalog snapshots](../reference/catalog-snapshots.md) and
[Raft persistence](../reference/raft-persistence.md).

## Keep security guarantees and qualification scope explicit

Signatures approve artifacts and mutations. Production trust also needs protected
boot, installed-state rollback resistance and key custody. Peer authentication
needs enrolled identities and role/session/freshness binding. Credential expiry
depends on authenticated security time; a nonzero SNTP sample cannot supply it.
Local zeroizing owners cover named storage when their destructors execute;
forced termination and compiler or physical copies need separate treatment.

Similarly, a bounded pool proves admission limits, while essential-service
progress requires concurrent workload qualification. Combine hostile ordinary
allocation and client traffic with healthy management, timer and teardown work.
Choose deadlines before the run and check charges after both confirmed and
uncertain cleanup. Fixed platform headroom alone does not prove fairness.

Keep tests discriminating: exercise the real production boundary and an unsafe
control where useful. Separate fixture assumptions from production guarantees;
one LP's IRQ mask does not isolate a sponsor shared with other LPs. A passing
repeat does not explain a failed run. Finite models check their stated
abstractions; QEMU process loss does not qualify device power-loss durability or
physical hardware recovery.

Read [production milestones](../reference/security-remediation.md#production-security-milestones),
[secret-state limits](../reference/secret-state-lifetimes.md),
[resource qualification](../reference/security-remediation.md#sec-07-bounded-resource-admission)
and [testing](testing.md).
