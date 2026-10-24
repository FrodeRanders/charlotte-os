# IPC capability payload storage

Every IPC capability has a prepared payload node alongside its shared authority
reservation. The endpoint, pending-call, direct/delegated connection, returned
connection and receiver reply-capability paths all use the same preparation.
There is no allocating `BTreeMap` insertion after authority publication.

`PreparingCapability` first reserves shared authority against the namespace's
captured generation, then allocates its payload node fallibly. Both fields stay
in one inert owner, including when allocation rejects after authority admission.
No payload or attachment mutation precedes this admission. Ordinary failure
explicitly finishes unused storage and authority after local IPC guards leave;
abandonment retains both. Existing containing call, endpoint and claimed-reply
owners keep their exact roots through this disposal and finish those roots last.

These factories **still run under IPC**. Fallible preparation before mutation
removes the later infallible allocation, but does not qualify its allocator or
all outer IRQ/guard contexts. Shared authority's own factory remains a separate
guarded boundary. The existing shared capability and connection count limits
are unchanged; private payload bytes and allocator overhead have no separate
comprehensive byte/principal admission.

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
