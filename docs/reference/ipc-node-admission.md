# IPC payload-node backing

IPC capability, endpoint, pending-call and reply-token payload nodes use the
shared admitted collection with `ipc_nodes` requested-byte/count admission.
This covers unused candidates, published nodes and detached cleanup storage.
It is separate from logical endpoint/connection/call/reply counts, unified
authority backing, namespace accounts and queues. Staging/loan vectors and
attachment retirement catalogs share the byte dimension as described below.

`ipc::node_budget::Charge<R>` keeps backing admission and an optional later-bound
logical resource reservation in the same allocation. The common collection's
`BindCharge` contract binds the latter without replacing backing admission.
Capability, pending and reply nodes use record charges; endpoint nodes use their
endpoint charge. Non-connection capability payloads still own backing admission
without an additional logical record charge. Requested layouts include payload,
linkage and both charge fields. No holder allocation or cleanup registry is added.
Actual node deallocation precedes both refunds through existing post-IPC disposal.

## Captured classification

| Node | Backing classification | Independent logical sponsorship |
| --- | --- | --- |
| Capability payload | `PreparedReservation` captures the exact recipient authority namespace's immutable storage class before preparation; IPC borrows it without another lookup | Connections retain their original grantor/requester record charge; other kinds use shared authority admission |
| Endpoint payload | Original endpoint reservation's class, captured against the accepted exact root before allocation | Original endpoint owner and its account |
| Pending call / reply token | Original call-record reservation's class, captured against the accepted exact caller root | Requesting caller and its original record account |

Trusted backing grants no logical resource privilege. An ordinary grantor may
create a trusted kernel-recipient payload while retaining an ordinary connection
charge. Promoting a root cannot reclassify a namespace's captured storage class,
an endpoint reservation or an already prepared node. Retirement and ASID reuse
cannot redirect their eventual refunds.

## Preparation and retirement

Every factory admits its actual layout before allocation. `PreparedCall` retains
its exact root, both logical reservations and the pending/reply prefix across
unlocked preparation. Capability authority/count preparation precedes its private
payload factory; the same captured storage class accompanies both. Endpoint
preparation retains its original endpoint/queue reservations before allocating
its node. Quota or allocator denial returns `IpcError::ResourceLimit`. Ordinary
rejection explicitly cancels the complete prefix after local guards, root last.
A failed capability factory leaves authority with its containing preparation
until finish; no attachment or payload is published early.

Publication and detachment only relink complete nodes. Scalar reply, cancellation,
endpoint/namespace close and hidden/unobserved connection cleanup retain these
same nodes in existing owning operations and retirement lists. Failed loan cleanup
retains its fenced record. Closed endpoints referenced by queued or returned
connections keep backing and endpoint admission until their actual disposal.
No metadata ticket reconstructs ownership or supplies physical completion/replay
permission. Complete call, capability, endpoint/growth and detached-list fallback
remains inert under IPC, heap and metadata-pool guards. Wider active containing
destructors, caller IRQ masks and copy latency require separate qualification.

## Staging and loan vectors

`klib::admission::ByteAllocator<IpcNodes>` attaches requested-byte admission to
standard `Vec` allocations. It reserves bytes and zero node slots before calling
`Global`, rolls back an allocation rejection, and deallocates before refund.
Allocator clones/moves preserve one immutable captured class and reserve nothing
by themselves. Reported blocks have the requested size, keeping deallocation
layout accounting exact. Default growth reserves the full candidate while the
old allocation remains charged; rejection preserves pointer, capacity and data.
This conservative growth can require more temporary headroom and a copy;
protocol paths normally reserve their known count once before publication.

| Buffer | Entry bytes | Maximum entries | Maximum requested bytes |
| --- | --- | --- | --- |
| Descriptor byte snapshot | 1 | 4082 | 4082 |
| Parsed descriptor entries | 16 | 255 | 4080 |
| Capability IDs | 8 | 255 | 2040 |
| Prepared transfers | 328 | 255 | 83640 |
| Reply-token loan origins | 32 | 255 | 8160 |
| Reply/cancellation revocation work | 200 | 255 | 51000 |

Call staging captures allocator class from the original record reservation
before any vector allocation. Sends use the captured exact sender class. The
capability vector moves into `MemoryAttachments` with its allocator; retirement
catalog admission borrows that class, without a new caller-supplied class or ASID
lookup. Loan vectors move into the reply token unchanged. Pop/clear do not refund
capacity: even an empty token loan vector stays charged until actual node/payload
disposal. The allocator stores one class byte without a holder allocation.

`PreparedVectorSend` owns the submission/root leases, cap and transfer buffers,
retirement catalog and delivery. The parser fills this owner (or `PreparedCall`)
as each transfer is prepared; partial rejection cannot leave an active prefix
in a temporary returned vector. Ordinary finish leaves IPC before child cleanup,
storage disposal and notification, completing roots last. Abandonment retains
all fields. A guarded real copied-vector fixture checks this parent after caller
promotion. The allocator grants no element-cleanup or physical-completion proof;
other active transfer parents and outer masks remain separate qualification.

The descriptor boundary uses `memory::object::snapshot_bytes_in` with the same
captured IPC allocator. Source rights, revocation state and length are validated
under the memory-object registry before allocation/copy. A complete 4082-byte
copy precedes descriptor parsing; only those copied bytes are subsequently
validated and used. Later source mutation cannot change the parsed snapshot.
Quota or backend allocation failure returns `ResourceLimit`; invalid sources
retain their `UnknownCapability` result and malformed entries retain
`MemoryTransferFailed`. Snapshot rejection preserves the caller's descriptor and source;
successful submission consumes the descriptor under the existing contract.

This temporary byte vector contributes zero node slots and stays charged until
actual disposal on every parse/transfer outcome, before the containing
submission completes its exact roots. It has no active capability elements or
new holder allocation. Default `snapshot_bytes` callers use `Global` through
the same validation/copy implementation; their image/package/store byte
admission remains separate work. Snapshot allocation/copy still holds the memory
registry, and ordinary descriptor disposal requires its existing post-IPC caller
context. This change does not qualify unknown outer heap/IRQ guards or latency.

## Revocation work buffers

`PreparedReply` and `PreparedCancellation` clone the original token's loan-vector
allocator while capturing token/call/root identities. Their work vectors hold
`(MemoryBorrow, LoanRevocation)` entries and share `ipc_nodes` requested bytes
with zero node slots. Promotion, a different closing actor or namespace retirement
cannot select a new class. Prepare the full capacity before preparing loans or
publishing the claim; quota denial returns `ResourceLimit` with the accepted
token and call intact. Empty work reserves nothing, even at the byte ceiling.

The complete parents retain work capacity, exact root leases or borrowed closing
owners, claims, returned-output preparations, detached metadata/attachments and
pending notifications in inert storage. Drop invokes no child destruction,
refund, source restoration, notification or root completion. In particular,
abandoned returned memory retains its actual transfer owner, source escrow/pin
and hidden destination; it must not be closed by reconstructing scalar owners.
Ordinary known-unstarted rejection explicitly restores that preparation.

Each child consumes its typed revocation receipt outside IPC. Successful pops,
cancelled unstarted prefixes and empty length do not refund capacity. Once child
cleanup and metadata/attachment disposal are complete, explicitly free the work
buffer before releasing exact roots, then dispatch pending notifications. Failed
physical cleanup keeps the child's existing terminal fence/pin and original
backing charges; freeing an empty parent work buffer grants no retry authority.

A work entry measures 200 bytes; maximum capacity requests 51000 bytes (about
49.8 KiB). The complete reply/cancellation parents measure 944/472 inline bytes
on supported targets; these are stack owners, with no new holder allocation or
payload-node layout change. Current numerical limits remain in place. Capacity
is requested during cleanup, rather than reserved when the call is accepted:
byte saturation can reject reply/cancellation and postpone root retirement while
original authority and backing remain retained. Raising limits adds headroom but
does not guarantee saturated teardown progress. A stronger guarantee would need
earlier cleanup-capacity reservation and integrated pressure qualification.

## Attachment retirement capacity

`MemoryAttachments` reserves `caps.len() * size_of(RetiredMemory)` before
allocating its exact-capacity retirement catalog. It contributes bytes and
**zero node slots** to `ipc_nodes`. Calls retain their original call-record
class; sends capture trusted state against the exact sender generation before
root leasing and revalidation. Retirement admission borrows the incoming cap
vector allocator's original class. Receiver identity, promotion and ASID reuse
never reclassify either allocation. Retirement and cap-ID storage have separate
allocation-lifetime charges, retained by the same attachment owner.

One inert owner retains the caps, full allocated retirement capacity and original
charge. Retirement can append at most the admitted cap count without growth.
Unused preparation explicitly finishes after local guards. Call publication
failure leaves the catalog in `PreparedCall` until post-IPC finish. Successful
receive carries delivered storage in `ReceiveResources` across guard exit before
freeing it; result-page rejection leaves queued storage untouched. Ordinary
vector-send rejection also disposes its catalog after IPC unlock.

Retired release moves the complete catalog into an inert FIFO iterator owner.
The full capacity and charge remain until every callback returns and the actual
iterator buffer is freed. Interrupted callback dispatch retains all remaining
owners and the allocation/charge. A child consumes its own typed retirement
owner, including terminal retention on physical failure; this metadata handoff
supplies no physical proof or replay permission. Explicit disposal frees the
buffer before refund. Empty length or successful delivery alone cannot invoke
implicit cleanup under an unknown outer guard.

## Capacity and administration

The expanded pool uses ordinary/total limits of **12/16 MiB** and
**73728/98304 nodes**. The earlier pending/reply-only defaults were 6/8 MiB and
18432/24576 nodes. Logical resource ceilings remain unchanged. The increase
accommodates capability and endpoint payloads alongside calls, avoiding an early
shared backing ceiling merely because the covered family grew.

A conservative count plan permits every ordinary/total unified capability slot
(49152/65536) to hold an IPC payload, plus all pending and reply nodes
(6144/8192 each) and endpoints (768/1024): **62208/82944 nodes**. This leaves
candidate/layout headroom while preserving a quarter of each pool for trusted
admission. Native capability/endpoint/pending/reply layouts are 120/272/200/184
bytes. This upper-bound mix requests 8466432/11288576 bytes (about 8.07/10.77 MiB),
within the 12/16 MiB budgets before retirement catalogs, other nested
allocations and allocator overhead. Added backing fields cost 16 bytes per
capability/endpoint node
(780/1040 KiB at those capability/endpoint ceilings), without a holder allocation.
Independent ceilings do not promise simultaneous RAM fit or progress.

Retirement entries measure 136 bytes on supported 64-bit targets. A maximum
255-entry catalog requests 34680 bytes (about 33.9 KiB). Its inline reservation
adds 24 bytes to each queued-message payload slot; its capability-vector allocator
adds another 8 inline bytes after alignment. Together, at the existing
ordinary/total queue backing-slot ceilings these fields represent 768/1024 KiB
before allocator overhead. The reply-token layout includes its 8-byte vector
allocator growth to 184 bytes.
These slot fields retain the existing endpoint queue accounting. Catalogs now
compete with node layouts for the same byte budget. Native real-capacity and
public-path controls require no numerical limit increase; measured mixed workloads
may justify adjusting central limits. The node-only plan above excludes catalogs.

Sharing the existing pool avoids another reserve and administration category,
but these layouts now compete for same-class bytes and slots. Raising limits
permits larger retained heap consumption; lowering them can restrict otherwise
valid logical resources. Review actual mixed workloads and adjust central byte
and count limits together. General heap/principal admission remains open.

The 23-pool catalog, statistics version 10, header/record widths, SDK and signed
runtime ABI remain unchanged. Services consume current limits from snapshots;
no service source or bundle rebuild is needed for this kernel-only extension.

## Qualification and remaining scope

The [payload evidence](../reports/audits/evidence/2026-11-02-ipc-payload-node-admission.json)
identifies source/artifacts and diagnostics. Earlier [pending/reply evidence](../reports/audits/evidence/2026-11-02-ipc-node-admission.json)
retains its original limits and version-10 HTTP qualification separately.
The full host suite includes actual deallocation tracing for backing plus a
later-bound logical resource charge. Native controls inject 512 allocator
rejections across all four layouts, exhaust real empty-node admission, and prove
heap-held pre-allocator quota rejection with exact recovery. Independent byte
saturation also uses an explicit counter adapter; it is not general heap OOM.

Public capability controls cover 64 quota denials after authority reservation,
namespace classification through promotion, and an ordinary grantor using trusted
recipient backing without trusted logical admission. Endpoint controls cover
64 quota rejections after endpoint/queue admission and promotion between reserve
and node allocation. Existing heap-held publication/detachment, root reuse,
hidden connection cleanup, scalar/vector receive, cancellation, returned authority
and guarded complete-owner abandonment exercise the same admitted nodes.

The [retirement-capacity evidence](../reports/audits/evidence/2026-11-02-ipc-attachment-capacity.json)
records actual deallocation-before-refund tracing, FIFO/interrupted callback
retention, constructor/size/quota rejection and native real-buffer pressure.
Public copy-call controls cover quota rollback, queued cancellation and
receive/reply/close with exact physical-frame and admission recovery; promotion
preserves the original buffer class. Complete copied-call and single-send
abandonment retain catalog capacity under the pool guard. A storage-only
delivered-receive fixture retains one empty catalog under IPC/heap/pool guards;
it adopts no physical backing or authority. Host physical retirement is an
adapter, not a native physical-completion proof.

Current IPC buffer qualification passed the full host suite
(including the 47-test byte-allocator and 48-test attachment harnesses), strict
test/minimal checks on both architectures,
and fresh four-CPU security campaigns: ARM 19/19, Intel VT-d 17/17 and AMD-Vi
17/17, with zero failures or pending tests. Numerical limits and wire formats
remain unchanged.

The [vector-capacity evidence](../reports/audits/evidence/2026-11-02-ipc-vector-capacity.json)
records actual old/candidate allocation overlap and deallocation tracing,
backend/quota rejection, pop/clear retention and immutable allocator class. Native
controls use real ordinary/total capacity, heap-held growth denial, a mapped
read-loan lifecycle and transfer-prefix/cap-ID quota rollback with exact counters
and frames. Existing partial vector and scalar-send rollback controls use the
same admitted buffers; a new guarded complete vector-send owner retains two data
pages and its original roots/authority.

The [descriptor evidence](../reports/audits/evidence/2026-11-04-ipc-descriptor-capacity.json)
records ordinary/total quota rejection before allocation while holding the
heap guard, using explicit counter-pressure reservations, plus trusted headroom,
invalid source/length and malformed rollback, public send rejection/retry and
source/descriptor/frame preservation. A copied
snapshot retains its original ordinary charge after source mutation, caller
promotion and root retirement, until actual buffer disposal.

The [revocation-work evidence](../reports/audits/evidence/2026-11-04-ipc-revocation-work.json)
records ordinary/trusted queued/delivered lifecycle controls, counter-pressure
quota denial under the heap guard before claim, original class after promotion,
full capacity through final pop/publication/attachment release, zero-work
admission at quota and exact recovery. Existing physical-failure controls verify
work-buffer disposal while uncertain backing stays fenced. Complete reply,
standalone cancellation and borrowed endpoint/namespace cancellation parents
are abandoned under lifecycle/table/IPC/heap/physical/registry/pool guards.

Other control storage, remaining allocations, general heap and aggregate
principal admission remain open. Cleanup headroom is not pre-reserved.
Serialized controls do not prove
sustained hostile concurrent EL0 fairness or all caller/IRQ/destructor contexts.
SEC-07/18 remain partial.
