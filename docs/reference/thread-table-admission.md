# Master thread-table backing admission

The runtime master table uses `IdTable<Thread, thread_table::Charge>`. Its
allocating `try_add_element` API is available only to uncharged generic tables;
runtime thread publication must use `try_insert_from` and complete admitted
replacement backing. [ASID-table backing](root-table-admission.md) uses the same
prepared-growth machinery with separate sponsorship and capacity admission;
wider caller contexts remain separate qualification.

## Capacity and sponsorship

The compile-time [resource policy](resource-policy.md) supplies two independent
dimensions, covering all retained and candidate vector sets together:

| Dimension | Node | Ordinary work | Each signed principal |
| --- | --- | --- | --- |
| Requested bytes | 32 MiB | 24 MiB | 16 MiB |
| Backing slots | 32768 | 24576 | 16384 |

One set owns `Vec<Option<Thread>>`, generation/lease/closing slot metadata and
the eventual return-ID vector. Its requested bytes are the checked sum of all
three array layouts; its slot charge is their common capacity. Exact reservation
avoids geometric allocator slack; an unexpected capacity rejects before
publication. Allocator bookkeeping and physical heap backing remain separate.
The inline run-queue link is included in each `Option<Thread>` slot; a thread's
future retirement-node payload has its own complete metadata admission.

Capacity starts at four slots and grows to the next power of two. The full new
set is admitted while old and competing sets remain charged. The exact creating
root supplies its authenticated signed principal and trusted ordinary/platform
classification under the root/ledger hold. Existing boot-lifetime principal
history records carry the new counters; there is no per-instance registry or
manifest classification. Kernel work consumes trusted node capacity.

The caller that triggers replacement sponsors the complete new allocation,
including its empty slots. The old allocation keeps its original sponsor until
its actual deallocation. Thread exit, logical root retirement, platform promotion
and ASID reuse cannot refund or reclassify retained capacity. Reusing a prepared
free slot requires no table allocation or new capacity charge. A different
principal can therefore use existing slots without taking ownership of their
backing charge. This is shared infrastructure accounting, separate from each
thread's context, retirement-node and stack limits.

## Publication and disposal

1. Retain the complete unstarted thread. Capture an opaque table-identity and
   capacity request under the master table, then release it.
2. Admit the complete replacement and allocate all three empty buffers outside
   lifecycle, publication-gate and master-table guards. Partial allocation
   rejection frees its prefix before refunding the reservation.
3. Under lifecycle, publication-gate and master-table serialization, recheck the
   exact root, abort/closing fences and domain thread limit. Prefer a newly freed
   slot or a competitor's usable capacity over installing an unused candidate.
4. Validate the candidate's exact table and every destination capacity before
   mutation. Move complete payloads and slot state once, preserving generations,
   leases, closing/retiring fences, queue identities and stable boxed contexts.
   No allocator, payload destructor, charge refund or callback enters this move.
   Rejection returns only a scalar error and leaves the same `Option<Thread>`
   in the containing preparation; publication consumes it only after every
   fallible check. This avoids large error/cleanup copies on bounded boot stacks.
5. Carry the empty old buffers or unused candidate out of the guards. Consuming
   `PreparedGrowth::finish` deallocates all three buffers before dropping their
   original charge. Rejected thread cleanup follows outside those guards too.

`CapacityChanged` proves that insertion changed no payload or generation. Only
this rejection permits bounded optimistic re-preparation: at most eight
publication attempts, with complete candidate disposal and a fresh root/fence
check each time. Exhaustion returns `ThreadPreparationFailed`. Quota, allocation,
root, generation and domain-thread-limit rejection never enter that loop. No
accepted thread, physical cleanup or uncertain operation is replayed.

Abandoning a candidate or replaced-buffer receipt retains its complete backing
and original admission without any allocator, counter lock or callback. Losing
the containing preparation also retains the complete thread through its existing
inert fallback. Neither terminal retention supplies a recovery owner or retry
authority. The global master table retains its installed set for the boot
lifetime; empty private fixture tables can deallocate explicitly after guards.

## Qualification and limits

The [qualification report](../reports/audits/2026-11-01-security-thread-table.md)
records 39 host controls and four-LP ARM/Intel/AMD security campaigns. The
standalone production `IdTable` host suite traces allocation/deallocation
and checks partial construction failure, overlapping charges, concurrent
candidates, foreign/stale/undersized rejection, free-slot preference, exact
lease/closing retirement across replacement and inert abandonment under a held
account lock. A control using the old allocating insertion must trigger the
same allocator tracer. It observes deallocation before charge refund.

Native controls use actual three-vector `Thread` layouts, never-dispatched
threads, signed sibling roots, root/ASID reuse, independent byte/slot denial,
sticky classification, and publication with real heap/lifecycle/master-table/
principal-history guards held. Separate counter-only ordinary saturation checks
platform headroom; it is not a real backing-pressure or fairness result.

The captured ARM boot campaign also exercises the original fixtures with a
128 KiB Limine boot stack. This increase costs 64 KiB per LP over the loader's
64 KiB minimum and avoids reshaping fixture ownership around that smaller
stack; runtime stack admission is unchanged. [The investigation](../reports/audits/2026-11-01-security-thread-table.md)
retains the smaller-stack failures, alternative fixture experiment and final
matrix separately.

The ownership move copies up to the admitted table ceiling while IRQ-masking
serialization is held. Its cost is bounded by capacity, not proven to meet a
latency target. Wider caller masks, observer/other metadata admission,
sustained concurrent EL0 pressure and essential management/timer/teardown latency
remain SEC-07 R7-1–5 and SEC-18 G1/G4/G7 work. No general heap, physical recovery
or production-platform qualification follows from this table pool.
