# Correlated DNS catalog-query relay

Followers resolve lookups and remote-call destinations through the leader's
fresh per-read Graft log fence. They do not promote their local applied catalog
into routing authority. The query record retains caller DNS session, query ID
and exact expected leader; replies retain status, service generation and owner.

## Sole current grammar

The reliable-message transport prepends the one-byte tag. Writers return the
body without that tag. All integers are little-endian.

| Direction | Complete frame |
| --- | --- |
| Request, tag `0x24` | `tag:u8 \| session:u64 \| query_id:u64 \| caller_len:u32 \| caller \| name_len:u32 \| name` |
| Reply, tag `0x25` | `tag:u8 \| session:u64 \| query_id:u64 \| status:i64 \| generation:u64 \| owner_len:u32 \| owner` |

Each caller, name or returned owner has a **4,096-byte transport bound**.
This retains complete identifiers, including node identities above the former
255-byte prefix. It does not widen local name-service publication or invocation
policy. Request caller/name must be nonempty. Replies may explicitly carry an
empty owner and generation zero for a missing lookup or read-barrier rejection;
those are current outcomes, not omitted fields or compatibility defaults.

Encoders validate lengths before fallible buffer allocation; no identity is
shortened. Decoders check tag, full declared lengths, bounds and exact end
before fallible identity copies. There is no suffix/padding acceptance. Old
`0x12/0x13` frames reject even with complete formerly valid payloads; relabelling
an old u8 body cannot make it current. The generic Raft transport recognizes
only tags 1--6, so these retired frames do not become a different control RPC.

The follower captures its monotonic deadline before preparation. Only a
successfully encoded body enters the existing pending-query record and
transport. Invocation routing retains the original name in that record; a
placement-only query needs its correlation and deadline after encoding.
Preparation rejection returns the existing invalid/oversized or
resource-limit status without pending publication or an outgoing query.
If reply preparation fails, the leader sends no partial response or synthetic
owner. It records that rejection once; the caller retains its original query,
expected leader and deadline. No accepted invocation is replayed.

## Correlation and ownership

The receiver still binds the claimed caller to its authenticated transport
peer. A requester accepts a reply only from the captured expected leader with
its own DNS session and query ID. Generation and owner remain required before
forwarding a call, whose separate existing generation fence is unchanged.
Neither a received frame nor readiness alone transfers capability ownership,
proves invocation completion or authorizes non-idempotent replay.

The codec owns ordinary bounded byte vectors, not capabilities. It uses the
same reactor pending-query and transport records, with no new registry or
per-instance wait source. Reply/read-barrier processing remains outside any
new kernel cleanup context. Existing raw reactor reply/attachment adapters,
metadata admission/destruction, timeout and physical-quiescence qualifications
remain separate; C18 documents the containing publication family.

Persistent catalog-v19, Raft pair-v4 and runtime ABI 3 are unchanged; DNS
interface 4 changes only client invocation. Both architectures must rebuild/sign
their DNS service for these new wire tags; there is no old-tag negotiation. Remote invocation subsequently
uses the [current 0x26/0x27 contract](dns-invocation-relay.md).
Registration/retraction now use the separate [current full-name and correlated
contract](dns-registration-relay.md) on `0x28/0x29/0x2a`.

## Verification

Host tests include a golden frame, explicit missing/read-barrier replies,
identities sharing a 255-byte prefix through the exact 4 KiB limit, rejection
above it on both writer and decoder, wrong/retired tags, complete old grammars,
relabelling, every truncated prefix, declared-length overflow and trailing data.
They do not simulate every allocator or live transport-admission failure.
The linked mixed-architecture fixture checks real follower queries, expected
peer/session/generation fences, remote invocation, exact variant pins and
endpoint retraction. Actual results and limits are recorded in
[batch evidence](../reports/audits/2026-10-14-protocol-simplification-dns-query-relay.md).

The [per-read fence correction](../reports/audits/2026-10-19-raft-read-fences.md)
keeps the leader's request and original deadline live until its fresh no-op
commits and applies. Local DNS OP_LOOKUP returns placement metadata without a
capability; remote placement returns node identity in moved memory. Ordinary NS
lookup grants are non-delegable SEND/CALL authority and are used directly by
owned invocation, rather than re-minted for a different caller. The native
fixture checks the local reply has no attachments. Application grant policy
remains at its existing authority boundary.
