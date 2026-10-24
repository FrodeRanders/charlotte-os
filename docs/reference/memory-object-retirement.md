# Memory-object mapping retirement

Memory-object backing and its creating generation's quota charge now survive
mapping removal until invalidation completes, independently of DMA/copy pins.
This repairs a teardown race: the final external unpin could previously destroy
a deferred object after the registry was unlocked but before its shootdown.
Failed bulk unmaps could also lead to physical release with live leaves.

## Sponsor metadata preparation

Memory-budget accounts now have fallibly prepared registry nodes and waiter/timer
sponsors. `PreparingAccount` retains all partial fields before any root-publication
guard. `PreparingRegistration` retains the unpublished root, capability namespace
and account preparation together. Ordinary rejection cancels metadata and disposes
the never-published root after local lifecycle/table guards leave; abandonment
retains every field without allocator, registry, callback or physical cleanup.

Publication links the prepared account to the root's exact assigned generation
under the existing table/lifecycle serialization. Runtime lookup, reservation,
retirement and sponsor capture never create an account. Missing account admission
rejects; thread sponsor capture treats a missing published account as an invariant
failure. BSP initialization prepares the kernel's trusted account after heap setup
and before any scheduler/thread or AP can capture a sponsor. User preparation is
ordinary; existing trusted platform promotion remains separate from manifest input.

The existing memory-budget ledger uses the shared `AdmittedMap` nodes instead of
an allocating `BTreeMap::entry`. A closed generation retains its node until its
last memory-object charge ends. Node removal does not refund an outstanding
waiter or timer: each retains its original sponsor allocation independently.
Final detached account metadata disposes after the local ledger guard, including
last-charge Drop and final-root bookkeeping. Other enclosing guards/masks remain
separate caller-context qualification; this is not physical completion or recovery.

The [sponsor metadata audit](../reports/audits/2026-10-18-memory-sponsor-preparation.md)
records rejection, heap-held first-use/publication, exact generation reuse and
guarded retention controls. These qualify preparation and local lifetime ordering.
The account/sponsor allocations do not yet have aggregate logical-principal or
general heap byte admission. Principal-history node admission is described below;
wider destructors remain SEC-07/18 work; this is not a fully fallible root-publication
path or hostile-pressure progress qualification.

## IPC namespace preparation

The same root-registration owner now prepares the IPC namespace node and both
endpoint/record budget accounts fallibly before publication guards. It captures
an exact-root publication lease before lifecycle/table unlock, publishes the
prepared node under IPC alone and releases that lease after IPC unlock. Missing
namespaces reject at runtime. Fenced cleanup detaches the drained node and
releases it outside IPC, while outstanding charges retain their original account
references. Per-capability payload nodes now prepare fallibly before authority
mutation and retire together with shared authority after IPC unlock; see
[capability storage](ipc-capability-storage.md). Call/endpoint factories now leave IPC with their existing exact roots retained;
other capability factories, publication vectors and wider caller destructor
contexts remain unqualified. Endpoint storage now prepares unlocked
and retires through the [existing cleanup owners](endpoint-budgets.md). See
[namespace publication](ipc-namespace-publication.md).

## Root policy and telemetry metadata

`PreparingMetadata` now prepares the launch-limit, domain-usage and signed-authority
slot nodes fallibly alongside the existing capability namespace and sponsor
account, before local
lifecycle/table publication guards. `PreparingRegistration` retains all partial
fields and the never-published root. Ordinary preparation or table rejection
explicitly disposes unused metadata and then the root after those guards leave;
abandonment retains the complete owner without field destruction.

Publication relinks these nodes in their existing registries using the assigned
generation. No allocating insertion or lazy fallback remains for those records.
After cleanup admission seals, all three exact-generation nodes detach into the
existing `ClosingAddressSpace` and transfer with the root to `RetiredAddressSpace`.
Failed invalidation retains them in the same receipt; only confirmed invalidation
permits explicit post-guard node release before root backing and slot completion.
Abandonment is terminal retention. The existing bounded root-invalidation retry
policy is unchanged; these nodes confer no new cleanup or replay authority.

The [root policy preparation audit](../reports/audits/2026-10-18-root-policy-preparation.md)
records rejection, heap-held publication, generation reuse and retirement controls.
Shared `AdmittedMap` lookup is linear and its hold time under hostile pressure is
unqualified. The later root-record pool below bounds those three node layouts;
other metadata/principal admission and wider caller contexts remain open.

The [signed-authority slot follow-up](../reports/audits/2026-10-18-root-authority-preparation.md)
extends that same owner and retirement receipt with an empty slot tagged by the
root's assigned generation. Empty slots grant no authority. Only the trusted signed
loader installs the principal/roles and descriptor digest, without allocation;
stale generation, zero principal and duplicate installation reject before mutation.
The slot detaches with the other root metadata; invalidation rejection retains
even a populated identity/digest, and confirmed invalidation releases its node
outside local guards before backing/slot completion. No separate registry or
identity cleanup ticket is introduced. History candidates now prepare before
publication guards; record linking and high-water updates reuse admitted storage.

Principal-history candidates and published records now carry their own bounded
[byte/count admission](principal-history-admission.md). The existing root
metadata owner retains a losing candidate through failed invalidation and
post-guard disposal; the published record keeps its original charge for the boot
lifetime. Policy/usage/authority node layouts now have their own
[root metadata pool](root-metadata-admission.md): 4 MiB/12,288 nodes total,
3 MiB/9,216 for ordinary preparation. Typed charges follow partial preparation,
publication and detached-root retirement until actual node deallocation.
One/two remaining slots reject complete root publication and return unused
partial nodes. Other namespace/account/control-block metadata remains separate.

## Owning the gap

`MappingRetirementPin` acquires a registry pin before mapping work or detachment.
Only explicit consuming release after invalidation decrements it. Its Drop
does nothing: an abandoned operation retains the pin, backing and original
charge in the registry. It neither takes a lock nor initiates invalidation.
Object IDs are monotonic and are not restored through reusable ASID lookup.

`RetiredObjectMappings` owns detached mapping records plus that pin. Domain
cleanup processes one object at a time, moving its existing mapping tree rather
than allocating temporary vectors of objects, invalidations or backing. A
monotonic cursor avoids rescanning processed objects. Final frame release
requires owner retirement, no DMA/copy/retirement pins, and no mapped aliases.
It runs after releasing the object registry, with the sponsorship charge still
owned through physical release.

## Mapping rejection and maintenance

`map` and `map_any` share one completion path. `map_locked` installs mapping
metadata and acquires the existing `MappingRetirementPin` under one registry
hold, before entering the page-table walker. Its ordinary pre-pin rejection
paths precede that metadata insertion. A returned rejection with no pin therefore made no
mapping change in this call. That branch skips TLB maintenance, returns the
original admission error and cancels any unused scratch reservation. An
already-mapped object or copy/DMA/loan restriction cannot make a denied call
consume cross-LP shootdown admission.

An admitted call retains its pin through invalidation even when installation
rejected and its prefix was rolled back. An error result alone does not prove
that maintenance is unnecessary. Only confirmed invalidation and, where needed,
scratch completion permit consuming the original pin. A failed rollback keeps
its installed-prefix record; uncertain maintenance retains scratch, metadata,
backing pin and original charge under the existing policy. No retry owner is
created.

Kernel map/map-any/unmap errors now preserve `InvalidationFailed(cause)` per
call. x86 distinguishes busy admission, masked context, epoch exhaustion,
delivery rejection and timeout; timeout includes the owned epoch and a sampled
first missing target. A later sample can be empty after a racing acknowledgement.
AArch64 preserves missing-root rejection. These observations authorize no
backing release or replay. Invalidation failure takes precedence if installation
or detachment also rejected; it does not imply those earlier steps succeeded.
The syscall boundary still reports status 9 for uncertain invalidation, without
exporting the detailed cause to userspace.

The [mapping admission follow-up](../reports/audits/2026-10-23-security-map-admission.md)
records denial/prefix controls and retained-failure tests. Its historical queued
loan panic remains causally unresolved; removing maintenance from pin-free
rejections does not identify the cause of that admitted mapping failure.

## Lock-separated detachment

Retirement preparation moves the mapping records and acquires the backing pin
under the registry guard. The receipt initially records that physical leaf
detachment is incomplete. Preparation consumes and drops that guard before
returning, so a chained detach call cannot accidentally extend a borrowed
temporary guard's lifetime. After release, the pin copies borrowed
frame identities into a fixed stack batch of 16 entries. Each batch's table walk
runs without the registry held. The batch is not a frame owner, a new object-size
limit, or a teardown heap allocation; larger objects use successive batches.

Ordinary unmap, mapped-loan revocation (including its IPC adapter), and domain
cleanup share this path. It removes the previous registry-to-address-space-table
nesting in unmap and bulk cleanup, matching mapping's existing separation. The
pin keeps the frame list immutable and backing charged across every registry
unlock, including a concurrent final DMA/copy unpin. Prefix lengths are checked
against that pinned list before invoking the walker; each leaf still must match
its expected physical frame.

All batches are attempted, retaining the first error. Any failure leaves the pin
undischarged, even when some leaves have been removed. Domain cleanup moves
records before detachment; ordinary unmap removes its record only after the full
detach succeeds. Invalidations and scratch/authority completion retain their
existing ordering. Whole-domain memory now borrows closing ownership and retains
mapped peer leases before releasing lifecycle. Published IPC move/copy/result
attachment cleanup now carries detached backing outside IPC, as described below.
Namespace loan revocation now borrows closing ownership and retains peer cleanup leases. Public mapping operations, all existing
borrowed-memory replies, explicit call/reply cancellation and direct loan revocation
now own live-generation leases across these phases, as described below.

Scratch now records live extents with fallible admission before publication;
release removes an exact existing reservation without allocation. Bulk finish
retains its pin and loan restrictions on any scratch-completion error, rather
than ignoring that result. The outer window-registry insertion and general
metadata admission remain SEC-07 work. See
[scratch admission](scratch-admission.md) for policy, costs and verification.

The pin also protects ordinary unmap and failed-map rollback: a concurrent
capability close cannot recycle backing just because its mapping record has
already been removed. Mapping frame-list preparation reports allocation failure
before publication. Mapping, copy/snapshot, write, DMA, transfer and loan-grant
paths reject new access while retirement is pending. This is an operation fence,
not revocation of an already executing CPU or a previously authorized DMA job;
those still require their own teardown and invalidation.

Borrower cleanup retains loan restrictions until invalidation finishes. Failed
detachment, failed invalidation, failed scratch release during ordinary unmap,
or abandonment keeps the retention pin. Quarantine preserves the original
generation's charge even after its namespace and translation tree disappear.
It has no recovery API. It is bounded by the existing charged backing-object
pool, but can reduce available capacity; it is not a successful cleanup or a
substitute for fixing an underlying failure. A diagnostic records failed bulk
detach/invalidation; there is no external quarantine telemetry field yet.

## Partial installation and foreign leaves

Each mapping records the prefix actually installed. If rollback fails, that
record remains instead of reporting the object as unmapped. Cleanup may only
visit the installed prefix, never the foreign leaf where installation failed.
Leaf detachment verifies its physical identity before removal. Missing or
mismatched leaves conservatively fail detachment; they do not authorize backing
reuse. Incomplete rollback therefore quarantines backing rather than attempting
an unsafe reverse cleanup. Scratch ranges are not returned before all associated
detach/invalidation checks succeed.

## Remaining boundary

The backing receipts themselves do not own an address-space generation lease.
Their caller must retain one or retain lifecycle/IPC serialization across the
complete operation. Numeric ASIDs and scratch identities must not be reused
between detach and finish. Published unmapped IPC attachments instead detach
all authority under IPC and retain original backing charges outside it; no ASID
lookup or mapping invalidation remains in their physical release. Whole-domain
memory retains exact mapped roots in its completion owner. Namespace loan
cleanup owns the closing generation and leases its peer before releasing IPC.

SEC-18 remains partial. Copy/vector preparation and unpublished copy rollback
now run outside IPC under exact root leases; publication revalidates captured
authority. General metadata allocation/destruction still uses allocator work
under serialization. Raw kernel fixtures retain serialized loan adapters.
Bounded x86 retry and QEMU NVMe reset now have hardware-path regressions;
abandoned-root recovery and wider platform/device quiescence remain open. See
[hardware quiescence](hardware-quiescence.md),
[kernel frame retirement](kernel-frame-retirement.md) and
[page-table lifetime](page-table-lifetime.md).

The [final address-space root](address-space-retirement.md) now has a detached
slot-leasing owner that finishes after lifecycle/table guards are released.
That separate close boundary does not lease an arbitrary live mapping's ASID;
live operations must establish their own retention before releasing serialization.

A [live-generation lease foundation](live-address-space-operations.md) now
retains roots through explicit completion and rejects busy close before mutation.
Public memory-object map/map-any/unmap calls hold a lease through page-table
changes and TLB invalidation, and release it on ordinary error as well as
success. Their mapping pins still independently retain backing. MMIO
map/map-any/unmap also holds a generation lease and a capability in-flight claim
through invalidation; concurrent device close rejects while that claim is held.
Explicit device close also leases its root and detaches its device object before
releasing lifecycle for invalidation. Whole-domain device cleanup now borrows
its closing root through an owning receipt outside lifecycle. It retains unfinished
device records/authority and scratch on failure; DMA teardown must complete
before its IPC drain. All existing borrowed-memory reply variants now compose
a reply claim and both namespace leases before releasing IPC. Explicit pending-call/reply
close now composes the same ownership in a separate cancellation owner;
explicit endpoint close borrows a server owner for each queued call.
Whole-domain loans now borrow closing ownership and retain exact peer leases.
The staged fence does not cover non-lease paths or revoke their authority while older operations drain.

## Owned loan revocation

Direct `revoke_lend` now owns both live namespace leases and a `LoanRevocation`
transaction. Preparation validates the owner and exact borrower capability, then
moves the existing borrower list into the transaction, publishes `Revoking` and
takes a backing pin under the memory registry. Detachment and invalidation run
after releasing lifecycle, registry and table guards. The borrower mapping and
capability remain recorded until successful invalidation and scratch release.
Completion removes that borrow and restores the other read borrowers using
their existing metadata, then releases the pin and root leases. No fallible
admission or allocation follows successful scratch release.

Failed detachment, invalidation, scratch release or transaction abandonment keeps
the backing pin and `Revoking` fence. The borrower capability cannot be closed or
used to regain access, and the original backing charge stays consumed after
namespace teardown. Failure also fences any other loans of that object. There
is no retry that restores authority from an uncertain state. Ordinary errors
complete the root leases; abandoning the complete leased operation retains its
roots too. Preparation errors publish no object fence and explicitly return all
already acquired leases.

Borrowed-memory replies, including returned connections or memory, use
`ipc::reply::PreparedReply` to retain both exact namespaces,
every prepared loan and an exclusive claim on the reply record. Namespace
admission precedes IPC; identities and capability authority are revalidated
under IPC before preparing loans. A bounded fallible vector is reserved before
publishing any loan fence. Failed preparation explicitly restores only receipts
that have not started physical cleanup, then completes acquired leases.

Once claimed, loan detachment, invalidation and scratch release run outside IPC.
Each successful loan is removed from the reply record before processing the
next. Competing replies reject the claim. Close of the pending call or reply
capability waits cooperatively outside IPC without first consuming authority,
keeping borrowed memory live until cleanup finishes. Success publishes the
result and removes reply authority before returning the leases. A later cleanup
failure restores unstarted receipts, clears the reply claim and completes leases,
but leaves the failed loan fenced and pinned. Dropping the whole operation
retains both root leases and the reply claim; close cannot force reclamation.
There is no recovery API for an abandoned claim.

Returned connections additionally own an unpublished `PreparedConnection`,
sponsored by the requester, and borrow the source endpoint/connection capability
through the reply claim. Explicit source-close waits outside IPC until that
claim ends. Queued connections and unobserved returned connections are rejected
by ordinary IPC lookup with `UnknownCapability` before grant or loan preparation:
their earlier queue/call still owns them. There is no queue-scan qualification
fallback. Delivery/observation is monotonic, so a
qualified source stays under explicit-close ownership. Its existing payload
keeps the endpoint record alive even if an unrelated endpoint-owner domain
closes. Such closure still makes the endpoint unavailable; a retained grant
does not resurrect it.

After loan cleanup, destination authority is published and installed under IPC
before result visibility. Ordinary preparation/cleanup/publication failure
refunds the hidden grant and its sponsorship before releasing leases. The
source is borrowed, never escrowed or re-admitted. Destination connections remain
inaccessible in their pending result until first observation, including against
minting, calls, watches and explicit close. Operation abandonment retains
the source claim but refunds the still-unpublished destination. Observed results
belong to the caller; pending-call close reclaims only unobserved returned grants.

Returned memory additionally retains a `PreparedTransfer` containing source
escrow, backing pin and hidden destination reservation. The source must be
owned, transferable and unmapped. Queued and unobserved-result memory is
rejected by ordinary memory lookup before loan mutation: the earlier queue/call
still owns its cleanup. IPC reports `MemoryTransferFailed` from owned transfer
preparation; no global queue/result scan supplies qualification. Delivery or
observation transfers that responsibility to the server. Both root leases prevent source/destination namespace reuse during the
unlocked interval.

Memory close checks the source's transfer fence before authority lookup. It
releases the registry and leases the exact root captured by that transfer,
then parks on an owned interest until the fence completes. Its source list and
disposal node are admitted outside memory/lifecycle/IPC guards and relinked
only after exact object/fence revalidation. Registration enters the independent
list, with a completion recheck rather than reacquiring the memory registry
under the scheduler's thread table. Closed-source late registration is immediate
readiness. Admission failure returns `ResourceLimit`, leaving source authority
and pins untouched; ordinary close completion explicitly releases its lease,
while abandonment retains the whole close owner.

The interest belongs to this transfer, not a namespace-wide or physical pin
drain. Ordinary loan/DMA/copy/mapping pin rejection still returns `LendingActive`.
Readiness proves neither hardware completion nor permission to retry uncertain
cleanup. Nonwaiting close retains its existing path; it acquires no fresh root
lease beneath serialized boot-fixture lifecycle/table guards. Serialized IPC cleanup
uses a nonwaiting retirement check instead; it cannot park beneath IPC.
It carries detached backing to explicit release after unlocking. Ordinary
rollback restores source authority, releases the backing pin and clears the
fence under one registry hold. Rejected close never removes/reinserts payloads.
Late transfer Drop cannot change a successor's reused scalar capability: escrow
captures its namespace, and payload cleanup checks the immutable object identity.

After escrow/pin/fence completion and its original metadata work, transfer
cleanup closes the exact interest. It queues already-admitted notification
nodes and its complete source-allocation disposal node in the shared
producer-LP waiter transport. That LP dispatches after producer/local guards
leave, preserving IRQ state. Another LP cannot run the callbacks while the
producer holds IPC. Source/list guards leave before Waker admission; final
list/node disposal follows notification outside those guards, even when no
waiter survived. The same transport serves retirement-head wakes. It carries
no backing/root owner, recovery ticket or physical retry policy; existing
cleanup receipts remain authoritative. Source fallback retains its list/node
and original allocation charge without field destruction.

Evidence: [exact transfer-close readiness](../reports/audits/2026-10-12-memory-close-readiness.md).

After loan cleanup, memory and any returned connection reservations publish
jointly under IPC before result visibility. A successful move removes source
authority; observed memory remains caller-owned, and pending-call close reclaims
an unobserved result. Preparation, cleanup or publication failure restores the
unstarted output source and refunds hidden admission before releasing leases.
Operation abandonment can also safely restore that unmapped, unstarted output
move; it cannot restore uncertain input loans or clear reply/connection-source
claims. No-loan replies still use atomic IPC serialization without detachment.

## Owned explicit cancellation

Closing a pending call or delivered reply with live loans uses
`ipc::cancellation::PreparedCancellation`. It captures both exact namespaces,
admits their leases before IPC, revalidates the call/token/capability identities
and prepares a bounded loan-receipt vector before publishing an exclusive
`completing` claim. No capability or pending record is consumed at preparation.
Second-namespace or loan preparation failure restores only unstarted receipts
and returns admitted leases without publishing a claim.

Queued cancellation keeps the original message and attachments in their admitted
queue until cleanup finishes. Receive returns `Pending` for a claimed front
message; closing its endpoint waits cooperatively outside IPC. Delivered reply
or call close likewise waits, and competing replies reject the claim. Both roots
remain leased while detachment, invalidation and scratch completion run outside
IPC. Each completed loan is removed from the token before continuing.
Blocking receive readiness excludes claimed or failed queue fronts. Queued
removal drains endpoint waiters and re-signals its bound CQ after IPC/lease
completion, so work behind a formerly held front is not stranded. The runtime's
nonblocking receive treats `Pending` as no currently available message.

Success removes the token and visible reply authority. Caller close removes its
pending record and queued ownership; queued copies/moves are released, while
already delivered ownership remains with the server. Reply-cap close publishes
`REPLY_CANCELLED` only after all loans complete. Notification occurs after IPC
unlock and lease completion. No new destination authority or teardown snapshot
is allocated at this point.

A cleanup error retains the failed pin/fence, restores untouched receipts,
returns root leases and leaves the call/reply capability live. It publishes no
terminal result and fences that failed token from delivery or reply; neither
partial-loan completion nor a released root lease makes the request usable again.
It cannot authorize borrowed-memory reuse. Abandonment retains
the claim, both roots, queued records and loan pins indefinitely; there is no
force-clear, timeout reclamation or recovery API. `PendingCall::close` returns
its Rust owner on rejection; its Drop/consuming-wait fallback aborts the domain
if close cannot confirm safety rather than ending the borrow normally.

Whole-domain loan cleanup now borrows the exact `ClosingAddressSpace` and
retains peers through cleanup leases, including already-closing namespaces.
Ordinary operation admission remains closed. Record sponsorship and every owned
endpoint are fenced before unlock, preventing new calls from entering the drain.
Per-token cancellation claims prepare bounded receipts under IPC, then detach,
invalidate and complete scratch outside IPC/lifecycle. Each confirmed loan is
recorded before capability/result visibility. Queued calls return
`REPLY_ENDPOINT_CLOSED`; delivered calls retain `REPLY_CANCELLED`.

Pending returns the closing owner for later polling. Failure returns an error to
root retirement, preserving original roots, charges, endpoint claims, failed
receipts and uncertain pins without publishing terminal success. Unstarted
receipts may be restored. Abandonment also retains peer leases and token claims.
Namespace removal iterates admitted registry storage without snapshots. Cleanup
admission permanently seals after IPC and owned memory drain and zero leases,
before root backing teardown. Raw boot fixtures alone retain serialized namespace loan
revocation. Move/copy/result attachments and comprehensive failure
recovery/quiescence remain separate work.

Loan preparation rejects any live DMA pin under the memory registry before
claiming `Revoking`; DMA admission also rejects that fence under the same guard.
CPU detach/invalidation cannot revoke device translations. Rejected revocation
leaves the original loan and mappings intact and returns its newly admitted root
leases. IPC reply/cancellation publishes no terminal result until DMA pins finish
and loan cleanup succeeds. Whole-domain device teardown remains ordered before
its IPC drain; a concurrently retiring peer also observes this pin guard.

Coherent DMA and executing CPUs
retain their existing quiescence obligations; a revocation transaction does not
itself stop an already authorized DMA transfer.

## Verification

Single-mutator boot fixtures inject the final copy/DMA unpin between detach and
invalidation, checking actual free-frame counts and exact-generation charges.
They verify loan authority before/after the borrower fence and scratch reuse
only after all mapping invalidations. Real collision leaves exercise clean
rollback, failed rollback, installed-prefix retention and physical-identity
checks. Fault adapters cover partial detach and failed invalidation; an abandoned
receipt exercises the non-releasing Drop policy.

A 35-page fixture exercises three batches (16/16/3), ordinary unmap, mapped-loan
revocation and owner teardown. It checks registry/table guard availability at
each detach callback, rejects an oversized installed prefix before any callback,
injects the final copy unpin between preparation and the first table walk, and
verifies exact data-frame and charge release after invalidation. It adds no
permanently retained frames. These guard checks are serialized fixtures, not a
concurrent lock-order stress test.

The scratch-completion probe checks two borrower mappings, all barriers before
release, last-copy/DMA-unpin retention, successful-range reuse and failed-range
non-reuse. Its rejected completion retains one extra page/charge even after
all involved domains close.

Loan fixtures check both-root close rejection, failed preparation without leaked
leases, completion while a staged borrower close waits, preserved read borrowers,
last-copy-unpin retention and scratch reuse after the barrier. Fault adapters
reject detachment, invalidation and scratch completion, and abandon a transaction.
Each retains one backing page and its charge after both domains close. These
four probes verify guard availability but do not stress concurrent hardware.

Failure probes intentionally quarantine **twelve 4 KiB data pages and ten object
charges** for the test guest's lifetime. They never re-adopt the backing. This is
in addition to the kernel-range fixture's one quarantined page. These fixtures
model the dangerous interleaving, not a concurrent hardware-walk stress test.
Both architecture guests execute the software fault fixtures. They do not
inject actual failed hardware IPI delivery or establish recovery from it.

Bulk-cancellation fixtures additionally exercise queued endpoint close and
queued/delivered caller-root close with mapped loans. A real prepared loan is
abandoned on the second cleanup step to model uncertain physical retirement.
The fixtures verify partial-success receipts, retained queue/call/capability
authority, zero terminal waiter notifications, failed receive/reply admission,
propagated root errors, closing-lease rejection and non-reuse of roots, frames
and original charges. Failed probes intentionally retain their namespaces and
backing; no recovery bypass frees them. See the
[bulk cleanup remediation](../reports/audits/2026-10-06-security-bulk-cleanup.md)
for executed guest results and limits.

## Owned explicit endpoint close

Explicit close of an endpoint containing loans now retains an inline
`PreparedEndpointClose` and its exact server-root lease. Its endpoint claim
rejects enqueue, receive, mint, resize and CQ rebinding with `Pending`, and hides
readiness without reporting terminal closure. A competing endpoint close waits
outside IPC. Root retirement rejects while that lease or claim remains owned.

The owner processes admitted queue storage one call at a time. A queued call's
`PreparedCancellation` borrows the server owner, leases its exact caller before
IPC, revalidates the front token and prepares its bounded loan receipts. It does
not reacquire the server lease: a staged server close may already fence new
admission. Detach/invalidation/scratch completion runs outside IPC/lifecycle and
registry/table guards. Only that call's confirmed cleanup permits attachment
removal and `REPLY_ENDPOINT_CLOSED`; earlier completed calls can finish even if
a later call rejects. Asynchronous sends have no loans and use their existing
copy/move cleanup. No whole-queue/root snapshot is allocated.

Endpoint close watches, readiness and bound CQ closure notification occur after
the queue is empty, endpoint authority is removed, IPC is unlocked and the server
lease completes. The claim stays live through final publication so a new loan
cannot enter that interval. Ordinary preparation/physical cleanup rejection
returns the endpoint capability, clears its endpoint claim and completes the
server lease. Failed call pins/token fences remain; untouched loans are restored
only before starting cleanup. If rejection exposes readable work, its readiness
edge is re-signaled after unlock. Abandoning an owner retains its claim/root;
abandoning a current call also retains that caller's lease, token and loan pins.

Loan-free endpoint authority removal remains atomic under IPC; its detached
backing releases after unlock. Whole-domain cleanup uses borrowed closing
ownership and peer cleanup leases rather than invoking this live-leasing owner
beneath lifecycle. Its loan cleanup and published move/copy/result frame release
run outside IPC. Physical CPU/DMA quiescence and recoverable x86 shootdown
failures remain separate work. See the
[endpoint remediation](../reports/audits/2026-10-06-security-endpoint-close.md)
for the executed regressions.

The [namespace remediation](../reports/audits/2026-10-06-security-namespace-close.md)
records whole-domain loan cleanup and closing-peer validation.

Device ownership and DMA-pinned loan regression evidence is recorded in the
[device retirement remediation](../reports/audits/2026-10-06-security-device-retirement.md).

## Owned whole-domain memory cleanup

Mapping publication captures the exact `AddressSpaceHandle` before registry
access while its operation lease retains that generation. Admitted mapping nodes
carry optional cleanup leases, avoiding a teardown snapshot. For each object,
`PreparingNamespaceObject` acquires a pin while mappings remain visible, then
retains peers outside the registry, including already-closing peers. Competing
cleanup returns Pending while that pin is active and cannot seal an unleased
mapped root. Borrowed closing ownership and the permanent kernel root need no
additional user-root lease. Unstarted admission rejection returns admitted peer
leases and the pin without moving mappings or changing destruction state.

After every affected peer is retained, `PreparedNamespaceObject` owns detached
map records and the backing pin. Fixed batches check leaf identity and detach
outside the registry; invalidation and scratch completion run outside lifecycle
and table guards. Success releases backing/authority, then peer leases. Physical
detach/invalidation/scratch failure or abandonment retains the pin, affected
mapped roots, scratch and original closing root. Rejected physical frame release
retains its original charge and affected roots after quiescence; it does not retry
partially released backing. Earlier completed objects stay completed. Namespace close
returns `MemoryCleanupFailed`, or Pending for another live pin/authority fence.
Only exact object and namespace completion receipts permit subsequent sealing.

Unmapped borrowed caps also wait for live revocation/transfer and DMA/copy fences
before their borrower state and authority are removed. A revocation's owned prior
reader list must return before another reader namespace can disappear; otherwise
completion could restore a stale borrower entry. Destroy-fenced objects cannot
resume authority and retain independent backing pins/charges through their own
lifetime. Root slot/tag/account destruction remains a separate final owner.

The historical serialized `close_address_space_fixture` adapter is used only at
raw boot-fixture boundaries. It is not a production fallback or compatibility path.
See the [memory cleanup remediation](../reports/audits/2026-10-06-security-memory-retirement.md).


## Undelivered owning IPC memory

IPC moves/copies stay inaccessible after authority publication until receive or
first reply observation. Their `MemoryCap` carries `delivery_pending`; ordinary
lookup rejects it even when an application guesses a valid scalar ID. This
covers information/byte access, CPU mapping, DMA pinning, copy/move/loan source
preparation and explicit application close. Scalar/vector calls and sends, plus
both returned-memory reply paths, publish through
`commit_undelivered_transfers[_with_authority]`. Trusted non-IPC transfers retain
immediate delivery through the ordinary commit API.

Receive publishes all owning attachments under IPC only after result-page
writing succeeds. Failed speculative receive leaves them hidden and queued.
The first successful `poll_reply` publishes returned memory before observation
is recorded under the same IPC hold. Subsequent polling does not republish
consumed or moved authority. `wait_reply` observes readiness only. Delivered
moves/copies belong to the receiver and survive call/endpoint cancellation;
unobserved/queued ones remain exclusively reclaimable by IPC.

The private cleanup lookup permits consuming hidden authority; normal explicit
memory close does not. Applications cannot create mappings, loans or pins that
would make its serialized cleanup reject while IPC discards the owning receipt.
This cleanup frees unmapped backing; it does not perform TLB invalidation beneath
IPC. Existing loan receipts keep their distinct owned revocation semantics,
including raw kernel fixtures that map queued loans. No new CPU/DMA quiescence,
recoverable shootdown or allocator-latency guarantee follows from this gate.
See the [delivery remediation](../reports/audits/2026-10-06-security-memory-delivery.md).


## Returned-memory source qualification

Both connection and memory reply sources now qualify through their ordinary
visibility lookup. Split-phase returned-memory preparation no longer visits the
global pending-call/endpoint registries or scans the server's queued attachment
vectors under the IPC write hold. Queued/unobserved owning memory is inaccessible through its admitted payload
state; delivered/observed sources must still pass `PreparedTransfer` ownership,
rights, mapping, pin and exact-generation checks before loan cleanup starts.

Queued and delivered loans retain their existing revocation contract. Their
borrower has map rights, not transfer ownership; copy preparation requires object
ownership too. Rejected borrowed-memory return leaves input loans, mappings,
pending result and destination admission unchanged. A normal reply can still
revoke those loans afterward. DMA-pinned revocation can reject and retain its
receipt/backing; this removes no physical quiescence requirement.
See the [source qualification follow-up](../reports/audits/2026-10-06-security-source-qualification.md).


## Published IPC backing release

Queued move/copy attachments and unobserved returned memory remain unmapped and
inaccessible through ordinary application lookup. Cleanup removes their exact
capability and object under IPC plus the memory registry into `RetiredMemory`.
That owner retains the frame vector and original sponsor charge. It performs no
physical free until explicit consuming `release` after IPC unlock. Abandonment
quarantines backing and charge without registry access, allocator work or a
retry. Any rejected physical release retains the full original charge even if
some frames already released; logical IPC close may complete because all
reachable authority was detached and no mappings/pins remain.

`MemoryAttachments` reserves retirement-owner storage fallibly before attachment
publication. Admission failure rolls back unpublished transfers and fresh IPC
admission. Its capacity is bounded by `CAP_VECTOR_MAX` (255), not a smaller
assumed vector size. Queue cleanup moves the existing admitted queue storage
outside IPC, retaining that storage's sponsorship until its actual destruction.
No teardown snapshot, growth or large fixed-size retirement array is needed.
Single unobserved results use one inline retirement owner.

Prepared cancellation confirms every loan first, then retires undelivered owning
attachments under IPC. Physical release finishes before returning its root
leases. Explicit endpoint close likewise retains its server lease and claim
through asynchronous backing release and forwards each call's cleanup owner.
Whole-domain cleanup keeps its borrowed closing root. Delivered and observed
memory remains application-owned and is not reclaimed by these receipts.

Physical release uses at most sixteen frames per allocator hold, consuming each
frame identity before deallocation. The allocator is unlocked between batches;
original accounting is refunded only after successful completion. This bounds
one hold's frame count, not end-to-end scheduling latency or all outer guards.
Copy/vector submissions now prepare and roll back unpublished backing outside
IPC, retaining both exact roots until staging ends. `commit_retained` returns
its failed `PreparedCall` before frame-owner destruction. General metadata
allocation/destruction under IPC remains separate SEC-18/07 work.

Boot regressions cover preparation-storage rejection with source restoration,
two full vectors (510 owning attachments), unlocked call/result/endpoint cleanup,
loan cleanup before ownership detachment, live root leases during physical
release, mapped/pinned retirement rejection, 35-page release across multiple
batches, original accounting across ASID reuse and Drop quarantine. The latter
adds one intentionally retained page and object charge to the fixture totals.
See the [IPC backing release follow-up](../reports/audits/2026-10-06-security-ipc-backing-release.md).
