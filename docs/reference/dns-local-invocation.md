# Owned full-name DNS client invocation

DNS interface **4**, opcode **5**, accepts one moved memory object. `arg0` is
its exact frame byte length. `charlotte_protocol_dns::encode` is the shared
service/kernel-fixture encoder; `catten_services::dns_call` reexports it.

| Field | Encoding |
| --- | --- |
| Magic | `CDNSCL01`, eight bytes |
| Total length | u32 little-endian, exact frame termination |
| Name length | u32 little-endian, 1..=256 |
| Target opcode | u32 little-endian |
| Scalar argument | i64 little-endian |
| Name | Complete bytes, including any trailing zero |

The header is 28 bytes; the complete frame is at most 284 bytes. Decoder bounds,
magic and lengths must all match. An allocation's unused page tail is outside
the frame. Retired packed-name/12-byte records reject; missing memory or invalid
framing returns `ERR_BAD_OPCODE`, boundary/length rejection `ERR_TOO_LARGE`, and
mapping/copy admission rejection `ERR_RESOURCE_LIMIT`. No absent opcode, argument
or name gets a default. The unique incoming memory, connection and reply token
are adopted together at the receive ABI boundary and disposed through their
runtime owners. No register layout changes: runtime ABI remains 3; catalog-v19,
Raft pair-v4 and remote relay tags `0x24` through `0x2a` remain current.

## Operation ownership and readiness

`LocalInvocation` moves the complete name into `ns::OP_TRY_LOOKUP_NAMED`, including
short names. Its lookup phase owns a `PendingCall`; its invoke phase owns both
the accepted pending call and the returned `Connection`. Unused returned memory
and connections are disposed once. Pending-call teardown precedes connection
teardown. The existing destination retains typed client reply authority or the
already prepared remote reply bytes; ordinary begin rejection returns that
whole destination. The existing pending vector reserves a slot fallibly before
submission, within its 64-operation bound. There is no new live-resource registry.

Each RPC phase binds producer reply readiness to the shared DNS CQ before the
reactor parks. Later reactor turns observe replies without blocking on one
operation needed to produce other work. The reactor uses the earliest remaining
local-operation, query or remote-call deadline, capped by its existing independent
clock/transport watchdog. Other raw receive/transport sources are not all bound
to this CQ; this is not a claim that the entire reactor is event-only.

One kernel-monotonic deadline starts before decoding/preparation. Follower
routing, lookup and invocation retain it; delayed preparation and unrelated wakes
do not renew it. Encoding is followed by an expiry check before publication.
The remote receiver uses its own original local deadline rather than comparing
clocks across machines. Lookup expiry admits no invocation and returns lookup
failure. After an invocation is accepted, timeout, poll failure or CQ-binding
rejection is `ERR_UNCERTAIN` and grants no replay authority. Drop uses the existing
runtime cancellation/close policy, including domain abort on rejected cleanup;
a wake is not completion or physical quiescence.

## Evidence and remaining work

The production state machine runs against fake owning resources in seven host
controls covering complete names, both CQ bindings, successful returned-resource
disposal, preparation/submission/binding rejection, cancellation/drop, uncertain
accepted operations, unrelated turns and one original deadline. Three codec tests
cover golden bytes, signed arguments, full 255/256-byte and trailing-zero names,
invalid writers, every truncated prefix, trailers, overflowing lengths and the
retired record. These controls do not inject live allocator exhaustion or prove
kernel physical cleanup.

The native linked fixture uses the shared encoder, rejects retired/trailing
frames, invokes short echo and a distinct delegated long alias through the named-memory
NS operation, and preserves
protected greet denial, endpoint retraction and exact signed variant pins. It
does **not** demonstrate native 256-byte publication/retraction. Current [NS generation operations](name-service-generations.md) now use mandatory
CNSGEN01 records and owned CQ-bound cleanup. Raw receive/transport, close watches,
transport queue/dedup admission and broader guard/Drop contexts remain separate
gaps. The containing family is [C18](cleanup-recovery.md), with
[batch evidence](../reports/audits/2026-10-14-protocol-simplification-dns-local-invocation.md).
