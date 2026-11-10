# Completion registry-node backing

Capability-backed completions, detached operations and CQs use independently
admitted registry nodes. Their existing complete preparation, published map,
backlog and retirement owners carry the same typed
[`entry_budget::Charge`](../../crates/catten/src/completion/entry_budget.rs).
Logical record/CQ admission and allocation backing measure different lifetimes.

| `completion_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 6 MiB | 8 MiB |
| Prepared/published/retained nodes | 15360 | 20480 |

Count defaults are twice the sum of the existing completion and CQ ceilings:
`2 * (6144 + 1536)` ordinary and `2 * (8192 + 2048)` total. Capability-backed
and detached operations share completion admission; they are not counted as
two independent full populations. The extra capacity permits unused candidates,
queue replacement and detached retirement to overlap without reducing existing
completion or CQ limits. A quarter remains shared trusted reserve. No RAM is
preallocated, and this planning envelope does not guarantee heap availability
or progress when retained work consumes the headroom.

The funded node charge occupies 32 bytes on supported 64-bit targets. Actual
node layouts are 80 bytes for capability records, 152 for detached operations
and 240 for CQs. At the count ceilings, even a population of the largest
layout requests 3686400/4915200 bytes ordinary/total, within the 6/8 MiB ceilings.
The layouts include the inline payload and charge. Allocator excess, completion
record Arcs and private holders, user buffers and observer lists have their own
owners and remain separate from these layout charges.

## Captured sponsorship and publication

The existing target capture takes the root generation, verified principal and
explicit platform resource class under the root-table hold. The completion
registry must match that exact generation before preparing work. Existing
account, capacity and source checks still qualify publication. Permanent kernel
work uses the trusted class. Raw fixtures with no live root use ordinary class
and family admission. Trusted root backing alone does not grant access to the
completion resource reserve.

The three layouts draw on the existing 8 MiB principal metadata allowance.
The shared funded node factory reserves principal and family bytes before
allocation. Promotion, replacement, retirement and ASID reuse preserve the
original charges. The captured principal supplies funding and does not confer
a root lease, publication authority or physical completion proof.

A CQ also reserves the bounded backlog capacity and its kernel heap ring, when
present, against the same principal before either allocation. The existing
CQ charge owns those bytes and the domain and node CQ limits. Physical ring
frames remain charged through their mapping owners. The CQ state disposes
detached entries and frees the backlog and heap ring before refunding its
storage charge. Unused storage refunds follow the same order.

Node admission reserves actual bytes and one slot before its own heap allocation.
Quota rejection never calls the allocator; allocator rejection releases the
unstarted reservation. Capability-backed and detached submission preserve
`SubmitError::WouldBlock`. CQ storage admission returns `CqOpenError::ResourceLimit`
and node preparation returns `CqOpenError::AllocationFailed` on rejection.
Earlier independent record/CQ/list preparation can already have allocated, so a
rejected complete operation still needs its existing post-guard rollback.
No new operation or authority is published on failure, and failed CQ replacement
keeps the old queue and source identity.

Published maps and detached CQ backlog entries only relink prepared storage.
The shared retirement collection frees each node before refunding its original
backing charge. Capability-node release remains independent of a completion Arc
held by a strong/weak reference. Detached ring delivery carries its node in the
existing refund batch until post-registry deallocation; an undelivered backlog
retains that node and its charge. Retired CQs retain their node with the ring,
backlog and source until explicit disposal. Root close cannot refund a node
still held by an existing retirement owner.

Complete preparation and retirement fallbacks remain inert under unknown guards.
They retain the original nodes and all existing authority, buffer, timer and
source owners; they neither refund backing nor create cleanup/retry authority.
Explicit unused cancellation and confirmed retirement leave local publication
guards before disposal. The pool itself only changes counters, without heap
allocation, callbacks, namespace lookup or nested subsystem acquisition.

## Qualification and remaining scope

[Current principal qualification](../reports/audits/2026-11-11-q4-completion-principals.md)
records the funded layouts, native artifacts and source identities.
[Original family evidence](../reports/audits/evidence/2026-11-02-completion-node-admission.json)
covers the earlier layouts and class-only scope.

Native controls fill ordinary and total ceilings with actual mixed-layout empty
candidates. They exercise heap-held family denials, funded allocator rejection,
ordinary and trusted classes, and the three production adapters. Sibling roots
share the original principal allowance. A rejected CQ replacement preserves
the existing source and rolls back its private storage prefix. An independent
principal still replaces its queue and completes an operation.

The retained capability and queue control crosses root close and ASID reuse
with a detached operation backlogged in the retained queue. Explicit disposal
reconciles each original refund independently of the successor and surviving
completion reference. Physical queue controls charge the kernel backlog and
verify both alias rejections preserve the frame. The existing guarded namespace
abandonment retains its full charge while the principal history is locked.

The pressure controls use serialized kernel fixtures and injected allocator
errors. Allocator excess, other record/control allocations, combined heap
capacity, sustained client progress, broader cleanup contexts and physical
completion qualification remain open under SEC-07 and C18/G1/G4.
