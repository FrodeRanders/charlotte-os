# Security audit: principal limits for private tables and thread stacks

Date: 2026-10-23. Reviewed baseline: `66f81585`.

The shared signed-principal allowance now includes **private translation pages
and maximum user-thread stack capacity**, alongside object, heap and image
backing. The ceiling stays at **16,384 pages (64 MiB) and 1,024 memory objects**.
A new instance cannot obtain a separate allowance for sparse page tables or its
user/kernel stack pairs. This extends SEC-07 R7-3 enforcement through two more
resource families; it does not finish general kernel-heap admission or essential
service fairness.

The [tracked evidence](evidence/2026-10-23-security-principal-structure.json)
records source/model/artifact hashes and the native results. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production
trust remains disabled. The preceding
[backing audit](2026-10-23-security-principal-backing.md) retains its original
three-family scope, failed runs and verification limitations.

## Enforcement

Verified signed launch prepares the existing principal-history record and a typed
sponsor before constructing an architecture root. x86 binds that sponsor before
its first PML4 reservation; AArch64 binds it before lazy allocation. Trusted
ordinary/platform classification still comes from kernel construction policy.
This covers private-root backing even when construction or namespace preparation
fails before an ASID or authority is published.

Authority installation checks any prebound table sponsor against the verified
principal. Its existing atomic reservation pays only for previously unbound
backing; an initial table page is not billed twice. Trusted raw fixtures still
bind pre-existing backing explicitly. Another principal cannot inherit a retained
charge through ASID reuse.

Each private table reservation consumes the same principal counter before backing
allocation. Empty and partially prepared branches remain charged to their root.
Confirmed unpublished cancellation and fully confirmed root destruction refund
both node and principal shares. Quarantined provisional pages and incomplete
physical walks retain their original shares. Platform promotion does not refund
the principal. Fresh shared kernel tables remain in their existing separate pool.

A user-thread `StackSlot` borrows the root's immutable sponsor and retains a typed
share for its configured maximum user pages plus sixteen kernel data pages. This
happens before physical stack allocation. Node rejection cancels the unused share
after the pool guard leaves. The existing root lease, bitmap slot, classification
and one-shot cleanup fences stay with that reservation. Explicit unused
cancellation or confirmed release of both halves refunds it. Failed cleanup,
partial kernel-half retirement and abandonment retain the complete share behind
an inert field fallback.

Data growth within reserved capacity does not pay again. New private table
branches still need separate admission, and cached branches remain charged until
root destruction. Reserving data capacity does not promise frames or successful
mapping. Pure kernel-only threads and inherited stacks keep their existing node
policy; the change does not invent an application principal for them.

These are the existing C03/C16/C17 owner families. No registry, cleanup custody,
physical retry owner, wire format, persistent record or runtime ABI is added.
Current contracts are in [principal admission](../../reference/principal-backing-admission.md),
[private tables](../../reference/translation-admission.md),
[stack admission](../../reference/stack-admission.md),
[lock ordering](../../reference/locking.md) and the living manual.

## Verification

New serialized kernel controls use real backing and never-dispatched roots:

- A zero principal allowance rejects x86 initial-root construction before a frame
  is allocated; AArch64's lazy root cannot allocate its first table either.
- Two roots share a sparse-table ceiling. A rejecting mapping retains its admitted
  prefix; 64 further attempts allocate nothing. The other root can reuse cached
  branches at the ceiling. Trusted promotion grants no principal headroom.
- Two user-thread roots share enough remaining capacity for one four-page user
  stack plus its sixteen-page kernel half. The first reserves all twenty pages
  while only one user page is committed. Sixty-four attempts on the second reject
  without frame allocation while a different principal creates and retires stacks.
- Growth commits the remaining user pages without a second data charge. Confirmed
  pair release refunds capacity. Missing-frame and invalid-address preparation
  cancel their complete reservations, with exact counter/frame reconciliation.

Existing mixed data, retained-transfer, object-count and atomic-binding controls
now include the roots' actual private-table charges. Their deliberately small
ceilings are set after warming the mappings needed by the test, so rejection
still isolates the intended resource. Two real signed-image instances account
for their image and table pages together. The failed pre-launch callback still
retains its moved object after the original root closes.

Existing private-table, stack and data/root release fault suites now bind their
real owners to the same fresh fixture principal and reconcile aggregate changes
against the original family/node charges. They retain real backing on deliberate
physical failure. Table and stack abandonment also run beneath the principal
lock and the original lifecycle/table/allocator/pool guards, without refund or
new cleanup work. These are serialized fixtures, not concurrent EL0 exhaustion
or a complete outer-context proof.

Both first combined native runs pass: AArch64 **20 passed / 0 failed / 0 pending**,
x86 **18 / 0 / 0**, two scoped security generations and all **14 host liveness
controls** each. Healthy replies beside stalled peers complete in **0.529–0.558 s**
on AArch64 and **0.376–0.440 s** on x86. Trickled deployment bodies reject in
**30.23 s** on each. Cancellation-stress retirement completes after **14,592**
and **15,452** requests respectively. Budgets were not widened and no guest
repeat was needed for this batch. Strict base-kernel Clippy passes on both
architectures; workspace formatting and whitespace checks pass. No userspace
service source changed, and the checked-in manual PDF is preserved.

The updated `CharlottePrincipalBacking` model distinguishes one-page
object/table footprints from two-page reserved stack capacity. A growth event
consumes prepaid capacity. Its safe graph has **72,344 distinct / 290,961 generated
states**, depth **14**. The two new expected counterexamples refund an abandoned
reservation or charge data growth again. The previous four controls still cover
generation-only admission, retirement refund, sponsorship changes on move and
late release against another principal. Object counts, node pools, hardware,
metadata, pressure fairness and executable refinement remain outside that model.
The complete harness passes **45 safe configurations and 133 expected
counterexamples across 42 modules: 178 cases and 221 checked inputs**.

## Remaining work

The aggregate boundary now covers these data/private-table/stack footprints;
it does not bound all kernel resource consumption by principal. General kernel
heap and metadata, principal-record storage, shared kernel tables, IOMMU/device
metadata, capability/IPC/completion families and kernel-only work still need
qualification or aggregate admission. Principal-record lookup and IRQ hold times,
concurrent application restart pressure and management/timer/teardown progress
under sustained load remain SEC-07 R7-3/R7-4/R7-5 work. The fixed principal limit
can also be above a smaller node/family ceiling; it is not a per-principal
fair-share guarantee.

No later pass resolves the earlier publication-lookup failure, cancellation-stress
root reclamation timeout, shared-quota restoration failures or lost ingress
response. Their original captures and causal limitations remain in their dated
reports. The finite controls here provide no permission to replay uncertain
cleanup or reclaim backing from a counter snapshot.
