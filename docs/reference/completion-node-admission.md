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

Each node adds a fixed byte/count charge (16 bytes on supported 64-bit targets),
a short pool operation at preparation/release, and a possible admission rejection.
Native node layouts are 64 bytes for capability records, 120 for detached
operations and 184 for CQs on both architectures. Even the largest layout at
the count ceilings requests about 2.7/3.6 MiB ordinary/total; the 6/8 MiB byte
ceilings leave layout-growth headroom. Charge overhead at the total count ceiling
is 320 KiB.
The byte dimension uses the actual requested retirement-node layout, including
the charge and inline payload storage. It excludes allocator bookkeeping,
separately allocated completion Arcs, buffers, rings, backlogs and observer lists;
those retain their existing owners and admission. Aggregate principal/general
heap accounting remains separate.

## Captured classification and publication

The three existing target captures retain exact namespace identity, budget-account
identity and current trusted resource classification before fallible node work.
Each node factory receives that captured class explicitly. Permanent kernel work
is trusted; raw fixture namespaces are ordinary. Application roles and names
cannot select reserve, and the allocator never re-resolves a numeric ASID.
Later publication revalidates the captured target through its existing checks;
the metadata charge supplies no root lease, publication authority or resource
reserve grant. Promotion, replacement, retirement and ASID reuse cannot reclassify
an already prepared or retained node.

Node admission reserves actual bytes and one slot before its own heap allocation.
Quota rejection never calls the allocator; allocator rejection releases the
unstarted reservation. Capability-backed and detached submission preserve
`SubmitError::WouldBlock`; CQ preparation preserves `CqOpenError::AllocationFailed`.
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

The [versioned evidence](../reports/audits/evidence/2026-11-02-completion-node-admission.json)
identifies sources and final native artifacts. Fresh four-CPU ARM, Intel VT-d and
AMD-Vi campaigns run the same production node types and shared pool.

Controls fill ordinary and total ceilings with actual mixed-layout empty
candidates. They check 384 heap-held quota denials across all three layouts,
192 injected allocator failures, independent byte rejection before allocation,
and complete recovery. Actual submission, detached submission and CQ replacement
reject without changing original accounts, authority or source identity.
Trusted production adapters still progress at ordinary exhaustion. Promotion
cannot redirect an original ordinary refund, and releasing one real node at total
exhaustion permits exactly one replacement allocation while weak-only completion
backing remains independently charged.

A separate control retains real detached capability/CQ nodes across root close
and exact numeric ASID reuse. Their later disposal refunds only original backing
and leaves the successor untouched. Existing 64-operation detached controls check
backlog retention through ring publication and post-guard refund. Existing complete
preparation, terminal, namespace and CQ abandonment controls also hold the new
pool guard; no additional deliberate-retention instance is introduced. Shared
collection host tests cover payload disposal and node deallocation before charge
release. Minimal/full strict builds, policy controls and formatting remain gates.

Native pressure is serialized kernel-fixture work. Byte denial uses oversized
layout admission without constructing that payload; native heap OOM is not forced.
The allocator failures are injected. These controls do not establish sustained
hostile concurrent EL0 fairness, aggregate principal admission, worst-case latency,
arbitrary containing-destructor/IRQ safety or physical CQ retirement. Other
registry/control storage and general heap accounting remain open. SEC-07 remains
partial; existing C18 and G1/G4 cleanup boundaries remain applicable.
