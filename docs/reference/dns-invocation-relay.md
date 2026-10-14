# Generation-fenced DNS invocation relay

DNS relays scalar opcode/argument invocations to the node selected by its
linearizable catalog query. Current frames use tags `0x26/0x27`; the reliable
transport prepends the tag to the encoder's body. All integers are little-endian.

| Direction | Complete frame |
| --- | --- |
| Request, `0x26` | `tag:u8 \| session:u64 \| call_id:u64 \| caller_len:u32 \| caller \| name_len:u32 \| name \| target_generation:u64 \| opcode:u32 \| arg:i64` |
| Reply, `0x27` | `tag:u8 \| session:u64 \| call_id:u64 \| target_generation:u64 \| result:i64` (exactly 33 bytes) |

Caller and name must be nonempty and each fit the 4,096-byte transport bound.
Writers retain the complete bytes; decoders validate bounds, tag and exact frame
termination before fallible identity copies. Retired `0x10/0x11` frames reject.
The old request u8 grammar cannot be upgraded by relabelling; the reply retains
its explicit fixed fields under the new tag. No absent field gets a default.
Persistent catalog-v18, Raft pair-v3 and runtime ABI 3 remain unchanged. DNS
interface 4 changes client invocation to mandatory full-name moved memory; both
architectures rebuild/sign DNS together.

## Preparation and admission

The direct invocation path captures its monotonic deadline before request
encoding. The follower carries its original query deadline through the local
or remote invocation handoff; expired routing results reject before admission. Only successful encoding enters
the existing in-flight record and sends a frame. Invalid/oversized identities
or allocation rejection return the existing error status without publishing
an invocation. Preparation and routing do not restart the caller's budget.
The receiving node uses its own local progress budget; clocks across nodes are
not assumed interchangeable.

After binding the caller to the registered peer identified by the transport
source MAC, the target
fallibly prepares a `PreparedReply` before dedup admission, local lookup or
invocation. Failure admits no operation and sends no fabricated result; the
requester's existing deadline remains. The existing remote destination owns
that buffer through asynchronous local progress. Ordinary local admission
rejection returns the entire destination, so the same prepared buffer encodes
the rejection. Terminal progress consumes that buffer into the reply, changing
only its result field, without allocating another codec buffer.

The asynchronous local lookup now uses `LocalInvocation`, moves the complete
1..=256-byte name to the named NS operation, and retains its pending call and
returned connection. Both lookup and invoke phases bind readiness to the DNS CQ.
No packed-name fallback remains in this operation. Client calls use the mandatory
CDNSCL01 grammar described in [local invocation](dns-local-invocation.md).
The 4 KiB relay bound does not claim 4 KiB local publication/invocation support.

## Correlation, deduplication and uncertainty

Source/caller equality, DNS session, call ID, exact target generation and
expected reply peer remain required. The target checks its catalog ownership
and generation before first execution. Pending duplicates do not execute
again; completed outcomes remain in the existing bounded dedup window with
per-peer reply ACK fencing. The prepared buffer replaces the destination's
redundant generation field; its immutable correlation bytes survive to reply.

Once a remote invocation is sent, loss or deadline expiry can mean the target
already executed. The caller returns `ERR_UNCERTAIN`; neither readiness nor a
timeout permits replay. Preparation failure at the target supplies no remote
completion proof. This adds no delivery negotiation, new registry, watchdog,
polling path or capability custody.

The existing MAC-to-peer equality check is not a new cryptographic identity
qualification. Only codec byte allocation is qualified here. Existing transport queue and
dedup metadata allocation, raw transport/receive and close-watch adapters,
wider guard/Drop contexts and physical quiescence remain separate.
The containing owner-family row is [C18](cleanup-recovery.md).

## Verification

Host controls check the complete layout, signed scalar argument round trip,
full identities differing beyond the former 255-byte prefix, exact 4 KiB and
oversized frames/writers, empty identities, wrong/retired tags, complete old
records, every truncated prefix, overflowing lengths and trailers. Prepared
reply completion preserves its original allocation and correlation fields.
The linked ARM/x86 fixture exercises real follower routing and remote
invocation, endpoint retraction and signed architecture-pinned rollout.
Results and limits are in [batch evidence](../reports/audits/2026-10-14-protocol-simplification-dns-invocation-relay.md).

Node-local generation cleanup now follows the [NS interface 2 contract](name-service-generations.md).
