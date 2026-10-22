# Deployment administration

Applications launch through signed `CDEPLOY6` descriptors and the scoped
owning runtime API. Descriptor-less syscall 64, `spawn_artifact` and
`launch_artifact` are removed. Number 64 is unassigned and is not an alias for
scoped launch 76. Both ISA entry points validate exact syscall membership; a
retired number follows the existing unknown-syscall domain-abort policy rather
than reaching the kernel dispatcher's trusted-call assertion. The independently authenticated encrypted connector launch
and live-upgrade contracts remain separate.

## Controller and catalog operations

`clusterctl` interface version 3 exposes `OP_NOTIFY` (6) for signed descriptor
admission and `OP_STATUS` (3) / `OP_ROLLOUT` (8) for inspection. Both queries
move the full artifact name in memory; `arg0` is its exact byte length. Names
must satisfy the signed descriptor grammar and fit its 48-byte bound. Packed
scalar status requests reject. Status returns the complete current
[deployment query record](catalog-snapshots.md), including the signed descriptor
and aligned replica node/variant pins. Rollout returns the canonical 40-byte
`RolloutStatus`; exact-generation readiness remains distinct from admission.

The controller forwards through the sole named DNS query operations (16/18).
It adopts each complete incoming transaction once and retains source mapping,
reply authority and accepted DNS call in `clusterctl_relay::Relays`. Eight fixed
positions share a 2,098,176-byte original/copy request reservation. Each DNS call
binds to the default CQ, alongside endpoint and lifecycle readiness. One original
five-second server budget spans descriptor preflight and submission; the caller's
separate deadline is unchanged. Failed, malformed or foreign preflight records
cannot authorize mutation. A checked absent result or older revision can; the
leader still performs authoritative admission.

Returned records validate before consuming `ReplyToken::reply_move`. Timeout
cancels DNS work and ends the source mapping before replying upstream; a delivered
DNS copy may remain receiver-owned and charged. Binding/wait rejection aborts with
the pool live. Shutdown drops pending calls before mappings and incoming authority,
without remote rollback proof. Accepted mutations never replay. Legacy upload,
join and key handlers remain blocking contexts on the same endpoint. See the
[relay evidence](../reports/audits/2026-10-22-security-controller-relay.md).

Controller deploy opcode 2 and DNS packed deploy/query opcodes 8/9 are
retired. DNS 15's redundant named envelope is also retired. Current single-
component admission uses DNS `OP_DEPLOY_SUBMIT` (33), with only the exact
signed descriptor and its byte length. Name, requested placement, object keys
and content digests are read from its signature. Follower relay 0x22/0x23
carries that same descriptor plus session/correlation/caller identity.

Deployment records contain generation, descriptor and exact replica pins.
`CDEPQR01` tags their complete query representation; `CATALOGI` version 18 tags
snapshots. There is no object ID, primary node or variant-zero digest in either.
Rollout status's representative node is computed from the first assignment for
inspection; it is not stored launch authority. Unknown operations reject and
consume their received attachments. The optional `DEP1` parser and stored-ELF
hashing adapter are removed.

## Local tooling

The serial fixture console offers `upload <short-name> [payload]` and
`status <full-name>`. Upload remains a local signed-ELF staging adapter with a
1–8-byte name, rejecting longer names rather than truncating them. Upload
neither admits placement nor grants launch authority. Use the host's
`cluster-sign deployment-notify` or `release-notify` for signed deployment.
The programmatic fixture tests retired opcode rejection and malformed/long
full-name queries; the deployment fixture queries each ready generation through
the public controller and checks its signed descriptor and replica variant pin.

See [batch evidence](../reports/audits/2026-10-13-protocol-simplification-administration.md)
and the [remaining simplification ledger](api-simplification.md).
