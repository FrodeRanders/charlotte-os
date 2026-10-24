# Borrowed authority publication batches

Memory transfers and additional IPC authority publish through one borrowed
batch. `PublicationBatch` exposes a reborrowed iterator over existing owners;
`publish_batch` validates the entire set before changing any authority, holding
the same `CAPABILITIES` guard across both passes.

Current providers are a publication slice/array and `TransferPublications`, which
chains the existing additional reservations and prepared memory-transfer slice.
Each pass must enumerate the same destinations, sources and dispositions without
changing membership, allocating, invoking callbacks or entering another
subsystem. The current providers only reborrow disjoint fields. They do not own,
clone or detach reservations, escrow, roots, pins or charges.

The memory helper no longer constructs a temporary `Vec<Publication>`. Iteration
uses constant auxiliary space, with no heap buffer or stack array proportional
to batch length. Existing checked length arithmetic and IPC descriptor limits
remain. One packed request can still contain 255 memory entries plus the call
and delegated-connection authorities. No ABI, wire grammar, version or count
policy changes.

## Validation and mutation

Memory publication first validates its prepared payloads under `MEMORY_OBJECTS`.
The borrowed authority gate then checks every destination and source against its
captured account, kind and staged/escrow state. A retired or invalid member
rejects the whole authority set before any member becomes live. Successful
publication applies the existing source actions: moves revoke source authority
into its retained metadata owner; loans restore their source authority.

Scalar-only calls need no memory registry. A prepared scalar call can now publish
its call authority, install its admitted payload nodes and enqueue without heap
allocation. This does not include initial preparation, receive or reply handling.

Mixed memory payload installation follows authority publication under the same
memory registry hold, with IPC held by RPC callers. The global memory-object map now relinks [prepared records](memory-object-storage.md),
including copies. Target memory-capability maps, namespace creation and
read-loan bookkeeping still use allocating `BTreeMap` updates. This batch removes the temporary authority
vector; memory publication is not yet fully fallible or fully allocation-free.
The heap-held mixed test brackets only authority iteration/publication, then
releases the heap before those payload updates.

## Ownership and rejection

The containing operations keep their existing exact roots, transfer pins,
private copied frames, escrow and original charges. Borrowed batch views add no
cleanup receipt or recovery registry. Ordinary unstarted rejection leaves
prepared owners available for their existing cancellation; it does not publish
or observe returned resources. Confirmed physical cleanup still requires its
own typed proof. Timeout, logical absence and readiness grant no replay or
physical release authority.

See the [audit evidence](../reports/audits/2026-10-24-security-ipc-borrowed-publication.md)
for complete heap-held scalar commits, the 257-member mixed authority control,
late extra/transfer rejection and ordinary rollback. These serialized checks
preserve IRQ state and original accounting; they do not prove arbitrary races,
allocator exhaustion or sustained fairness. Other guarded factories, memory
payload storage, wider caller contexts, aggregate byte/principal admission and
historical failures remain SEC-07/18 work.
