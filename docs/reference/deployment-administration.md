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
It adopts uniquely received request/reply capabilities at its raw receive ABI
boundary and uses `call_move`, `PendingCall::wait` and `ReplyToken::reply_move`.
It validates returned record bytes before forwarding. Failure, malformed input,
unknown operations and absent reply tokens release their owned transient
attachments; returned memory is never detached into scalar cleanup logic.
These handlers do not introduce timer polling or replay accepted RPCs.

Controller deploy opcode 2 and DNS packed deployment/query opcodes 8/9 are
retired. Unknown operations reject and consume their received attachments.
The optional `DEP1` descriptor suffix parser and stored-ELF hashing adapter
are removed with those operations. Current named signed admission (DNS 15)
and follower relay retain their existing name/object/node/digest fields until
the next coordinated replicated-record simplification; those diagnostics do
not replace signed variants or committed pins.

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
