# IPC capability payload storage

Every IPC capability has a prepared payload node alongside its shared authority
reservation. The endpoint, pending-call, direct/delegated connection, returned
connection and receiver reply-capability paths all use the same preparation.
There is no allocating `BTreeMap` insertion after authority publication.

`PreparingCapability` retains the captured namespace, prepared shared-authority
metadata, admitted authority and payload storage in one inert owner. Shared
count admission precedes payload-node allocation. Allocation rejection after
admission leaves the complete prefix in that owner.
No payload or attachment mutation precedes this admission. Ordinary failure
explicitly finishes unused storage and authority after local IPC guards leave;
abandonment retains the complete prefix. Existing containing call, endpoint and claimed-reply
owners keep their exact roots through this disposal and finish those roots last.

Call and endpoint factories now run outside IPC through the exact-root parents
described below. Direct/delegated grants, receiver reply authority and returned
connections still use the guarded adapter to the same preparation machinery.
Their factories and all wider IRQ/guard contexts remain separate qualification.
The memory/authority publication helper also still allocates its batch vector
under IPC, including scalar-only calls. The existing shared capability and connection count limits
are unchanged; private payload bytes and allocator overhead have no separate
comprehensive byte/principal admission.

## Call and endpoint preparation

The existing root-owning operations carry one captured namespace through three
phases:

1. Prepare shared-authority storage outside IPC, retaining it in the containing
   operation. No capability identity is admitted yet.
2. Reacquire IPC, revalidate that exact namespace and its retirement fence, and
   reserve shared count/identity by relinking prepared authority metadata.
3. Leave IPC before payload-node allocation. The call also prepares its pending
   waiter list here, using the original call-record classification. A full shared
   namespace rejects before either factory. Revalidate again before publication.

Endpoint publication uses its existing root check and borrowed reservation;
call staging now only revalidates already-prepared metadata before attachment
work. Unused preparation, including rejected shared-admission metadata, finishes
after local guards; roots finish last. Complete fallback retains every field.
Kernel roots remain immortal and synthetic fixtures retain their explicit
namespace boundary. No lifecycle-under-IPC edge, IRQ enable or new root mechanism
is added. Wider caller masks and unrelated guards still need qualification.
See [execution evidence](../reports/audits/2026-10-24-security-ipc-unlocked-preparation.md).

## Publication and observation

Publication borrows the retained authority reservation. Once joint memory and
authority publication succeeds, installation only relinks the prepared payload
node under the same IPC hold. Its capability identity must already be live in
the target's shared namespace. Call, connection and memory attachments retain
their existing atomic publication and hidden-delivery rules.

Connection payload nodes own the original connection-count charge independently
of the payload. A queued or unobserved returned connection stays hidden from
ordinary lookup until dequeue or first reply observation. A direct grant becomes
visible under the same IPC hold as installation. Readiness alone observes no
returned resources.

Receive keeps its preparation and speculative retirement in one containing
owner. Result-page rejection detaches only the speculative receiver reply
capability; the queued call, token, loans and output bytes remain intact. Payload
and authority storage dispose after IPC unlock. Both scalar and vector receive
use this boundary; one-way receive prepares no reply capability.

Scalar reply also retains unused returned-connection preparation and detached
capability metadata with its existing completion owner. Claimed replies retain
them with their existing root-owning operation through post-IPC disposal. Failed
publication or loan cleanup leaves the live token and uncertain backing fenced;
metadata preparation does not supply replay authority.

## Retirement

Removing an IPC capability detaches both its payload node and the shared
`RetiredRecord` under IPC. That authority receipt moves into the detached payload
before the node joins the existing temporary cleanup list. Public close, queued
and unobserved-result cancellation, speculative receive rollback, scalar/claimed
reply, endpoint close and namespace drain all carry that complete metadata pair.

The existing close/cancellation owners reuse the detached nodes as list storage;
there is no new registry or teardown snapshot allocation. Explicit disposal leaves
IPC, releases shared authority metadata, then frees the payload and its node
before returning the original connection charge. Abandonment retains the whole
pair and its original charges without allocation, locks or callbacks. Detached
records can retain their original accounts after root retirement and ASID reuse;
late disposal cannot credit the successor.

These receipts own metadata. Existing root/pin/loan proofs still govern physical
cleanup. Logical absence, timeout, a wake or a detached node grants no physical
release, retry or recovery custody.

## Verification and limits

The [storage audit](../reports/audits/2026-10-24-security-ipc-capability-storage.md)
records preparation rejection, heap-held publication/detachment, post-guard
refund, containing-call failure, guarded abandonment and exact ASID reuse.
Existing quota, transfer/mapping failure, loan cancellation, returned-resource,
readiness and scheduled retirement controls remain enabled. Stage injections
exercise real admitted prefixes without exhausting the global allocator.

All IPC registry maps now use the shared admitted-node storage. Lookup and
sorted insertion are linear, and reference/close scans can repeat them. Count
limits bound registry size; they do not establish worst-case guard/IRQ hold time,
hostile-pressure fairness or management progress. Other factories/destructors,
weak-only retention, aggregate byte/principal accounting and historical mapping
or root-close failures remain open. SEC-07/18 remain partial.
