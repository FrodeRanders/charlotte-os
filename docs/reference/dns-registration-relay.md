# Correlated DNS registration and exact-generation retraction

Only the current network grammar is accepted. The reliable transport prepends
the tag to each encoder body; integers are little-endian.

| Direction | Complete frame |
| --- | --- |
| Register request, `0x28` | `tag:u8 \| session:u64 \| request_id:u64 \| owner_len:u32 \| owner \| name_len:u32 \| name \| deployment_generation:u64` |
| Register reply, `0x29` | `tag:u8 \| session:u64 \| request_id:u64 \| generation:u64` (exactly 25 bytes) |
| Automatic retraction, `0x2a` | `tag:u8 \| owner_len:u32 \| owner \| name_len:u32 \| name \| generation:u64` |

Owners must be nonempty and fit 4,096 bytes; names must be nonempty and fit the
current 256-byte local publication bound. Writers reject oversized identities
instead of shortening them. Decoders borrow identity slices only after exact
frame/bounds validation, without allocating on malformed network records.
Session and request ID must be nonzero. Deployment generation is mandatory;
zero explicitly identifies a system service. Retraction generation must be
nonzero. A reply generation of zero explicitly reports failed registration or
activation; it is never an absent-field default.

Retired `0x14/0x15/0x16` tags reject. Relabelling their old u8 bodies does not
make them current frames. Catalog-v18, Raft pair-v3 and runtime ABI 3 remain
unchanged; DNS interface 4 changes only client invocation. Both architectures rebuild/sign DNS together.

## Admission and ownership

A follower prepares its full request, captured leader string and pending-vector
slot before local publication resolution or transport submission. An independent
registration counter never wraps/reuses IDs within its DNS session; exhaustion
rejects before admission. The complete RemoteRegister owner is published before
sending. It retains the captured peer/session/request, original name, caller
ReplyToken and optional Connection. Immediate preparation rejection drops the
unique attached connection while leaving reply authority with the receive caller.
Deferred rejection consumes the same pending owners. Registration retains the
existing queue-cap uncertain-abort policy; this batch adds no new remote
registration deadline or replay authority.

Service registration requires an attached connection or a live local publication.
Missing publication rejects before a leader Raft proposal, instead of inventing
a catalog-only service. Catalog-only markers/events use the explicit memory-carried
`OP_EVENT_FIRE` operation. The DNS and HTTP kernel fixtures use that operation
for their alpha marker. Service echo registration still uses actual local
publication; the follower deployment barrier retains its real echo owner.

Scoped deployment registration observes its live publication generation without
acquiring invocation authority; the deployment agent retains the complete
DeployedArtifact. Both local resolution and scoped generation observation use memory-carried
full-name calls, including short names, without a packed-name fallback. Prepare/Activate move Connection and ReplyToken owners between phases;
local registration uses a staged full-name memory copy and delegated connection,
retaining returned resources in CallResult. The connection moves into the existing
LocalPublication record without integer adoption or a new cleanup ladder.

The leader binds request owner bytes to the registered transport source peer,
then fallibly prepares both identities, the pending slot and a PreparedReply
before its first Raft command. That reply allocation survives register/activate,
term invalidation and queue-cap abort. Completion patches only generation and
consumes the same allocation. Preparation rejection submits no command and
supplies no fabricated completion; existing uncertainty policy remains.

A follower consumes a reply only when peer, session and request ID match its
captured operation. It uses that operation's retained name/local generation;
reply-supplied identity overrides are gone. The two-phase generation checks
remain required before catalog visibility. MAC-to-peer equality is an existing
transport binding, not newly qualified cryptographic authentication.

## Retraction and readiness limits

Retraction still requires the exact hosting owner and distributed generation.
The leader checks that active catalog generation and prepares the pending name
and slot before submission. Stale retraction cannot remove a replacement. Failed
sender encoding leaves the existing publication record intact; its existing
protocol retry interval and exact-generation termination policy remain. No new
polling, wait source, registry, custody or permission to replay an accepted
non-idempotent operation is introduced.

Generation observation/retirement now uses [mandatory full-name records and
owned cleanup](name-service-generations.md). Raw endpoint-close watch completions,
watch admission retry, transport/catalog metadata and broader IRQ/Drop contexts remain
separate qualification work. Local lookup/registration uses PendingCall::wait;
this does not make every phase of the DNS reactor event-driven. Existing vector
publication outside the specifically prepared pending slots also remains separate.
The owner-family row and evidence are [C18](cleanup-recovery.md).

## Verification

Host controls cover complete golden layouts, full 256-byte names with distinct
final bytes, 4 KiB owners, explicit deployment generation zero, empty/oversized
writers and complete invalid frames, every truncated prefix, length overflow,
missing suffixes, trailers, retired/relabelled records, zero correlation fields,
prepared reply allocation reuse and wrong-peer/session/request rejection.
Native mixed ARM/x86 deployment checks exercise real follower publication,
automatic endpoint retraction and signed architecture pins. Large identities
remain codec evidence, not a long-identity native cluster qualification.
Allocator exhaustion and every reactor cleanup context were not injected.
See [batch evidence](../reports/audits/2026-10-14-protocol-simplification-dns-registration-relay.md).
