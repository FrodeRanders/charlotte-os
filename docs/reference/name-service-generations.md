# Full-name publication generations

NS interface **2** uses one mandatory `CNSGEN01` moved-memory record for
publication observation (opcode 18) and exact-generation unregistration (opcode
10). `arg0` is the exact frame length. `charlotte-protocol-ns` defines the shared
encoder/borrowed decoder; `catten_services::ns_generation` reexports it. The
kernel's node NS endpoint advertises the same shared interface version.

| Field | Encoding |
| --- | --- |
| Magic | `CNSGEN01`, eight bytes |
| Total length | u32 little-endian, exact frame termination |
| Name length | u32 little-endian, 1..=256 bytes |
| Generation | u64 little-endian |
| Name | Complete bytes, including trailing zeros |

The header is 24 bytes and the whole frame is at most 280 bytes. Observation
requires generation zero; retirement requires 1..=i64::MAX. Both action decoders
validate magic, bounds, exact lengths and generation semantics before lending a
name. Extra allocation page padding is outside the frame. Retired scalar
observation, bare-name observation and packed-name/eight-byte retirement reject;
there is no compatible parser or missing-field default. Other NS operations
retain their existing grammar; DNS interface 4 and register runtime ABI 3 stay
unchanged. All signed service variants rebuild together with the kernel endpoint.

## Authorization and unique ownership

NS adopts each generation request's complete `IncomingMessage` once at the
receive boundary. Unexpected connections reject and are disposed with the other
owned attachments. Memory size, frame decoding, fallible complete-name copy and
explicit unmap precede registry/policy mutation. Invalid memory/framing or
mapping/copy/unmap rejection returns `ERR_INVALID`; every attachment and reply
authority stays in its Rust owner on early return.

Observation exposes naming metadata without minting invocation authority. A
confirmed missing or unpublished endpoint returns `ERR_NOT_FOUND`. The reactor
consumes its persistent endpoint-death watch before request intake; watch
admission rejection prevents publication, and uncertain watch observation
follows the fail-closed domain policy. The owned client
preserves `Result<Option<generation>, Error>`: only a confirmed absence is `None`.
The agent's existing convenience API still returns `Option` for readiness only;
DNS explicit tombstoning uses the checked result and submits no command on error.

Retirement requires the exact current generation and a kernel-authenticated,
nonzero stable principal equal to the recorded registrar, or a service manager.
A missing principal/registrar grants no authority. Provisioning still verifies
the captured address-space generation, principal and roles through PolicyStore;
its own manager identity performs the existing policy unpublish. A stale or
unauthorized request returns `ERR_NOT_FOUND` and leaves the publication intact.
On success the registry keeps its generation tombstone and takes its complete
`ns_publication::Publication`: an owned connection and endpoint-death completion.
The watch is canceled/disposed before its source closes. Existing best-effort
local connection close establishes no physical backing quiescence or refund.
Other raw receive/lookup/reply adapters remain separate ownership work.
DNS does not acquire retirement authority by looking up an independently
published endpoint. Caller cleanup returns no-match unless its original
registrar/manager and generation checks succeed. Neither a catalog tombstone
nor endpoint death grants DNS manager rights.

Service publication now stops at `i64::MAX`, matching both positive NS return
values and retirement records. The policy store checks that bound before
changing its generation, ceiling or active state; exhaustion preserves the live
binding or tombstone. The [18 October review](../reports/audits/2026-10-18-name-publication-model.md)
records the reproduced boundary failure and correction.

## NS-owned endpoint-death producer

At registration's unique receive boundary, NS adopts the incoming connection
once and prepares a persistent endpoint-close watch before policy or registry
mutation. A failed watch submission disposes only the unused candidate and
leaves the existing publication untouched. Each live registry entry owns its
connection and watch together; replacement disposes the previous complete owner
after the new generation is authoritative.

NS binds its launch-owned request endpoint to the default CQ. Kernel endpoint
watches already post capability-backed completions and signal that same CQ, so
the reactor waits event-only on requests, watch completions and lifecycle wakes.
It drains ready CQ entries before each request, with no timer, unrelated wake
source or fresh per-instance registry. `Completion::matches_cq` only selects a
readiness hint; NS still polls that owned completion once to confirm the exact
`IPC_REPLY_ENDPOINT_CLOSED` result. Pending, stale and unrelated hints cannot
unpublish a binding. Unexpected terminal results or observation/wait admission
failure abort with the registry retained; they are not endpoint death, retry
permission or successful cleanup. Lifecycle requests are rechecked on every turn
and use terminal domain abort without a cooperative/physical-cleanup acknowledgement.

A confirmed endpoint-death result unpublishes the current entry's complete name
and captured generation through NS's own policy authority, then takes its
publication owner. Old/replaced watches cannot act on a replacement: hints are
resolved through its own retained completion and exact underlying producer,
never the name or an old CQ result alone. This internal producer clears bindings
created by any registrar, while caller retirement keeps its original principal
checks. It is logical namespace retirement, not physical endpoint reclamation.

Live publications are bounded to 64. The trusted supervisor's NS constructor
prepares a 65-operation completion/record/watch budget before thread publication
or event capture, retaining one slot for replacement preparation. Ordinary
launches keep their 16-operation budget. No live sponsor is replaced; captured
platform classification and normal node/aggregate admission remain. Tombstones
retain generations within the existing bounded policy store and consume no live
watch. A new binding at the live limit rejects before publication; replacement
may prepare against its reserved slot. Wider policy/tree allocation qualification
remains separate. See [execution evidence](../reports/audits/2026-10-14-ns-endpoint-death.md).

## Cleanup readiness and failure policy

`publication_generation::observe` stages/moves the record and waits on its one
accepted RPC with the remaining original five-second monotonic budget. It does
not mint a connection. Timeout and wait-admission rejection remain errors;
runtime pending-call cleanup preserves its existing cancellation/abort policy.
The NS process is independent, but this bounded synchronous metadata RPC can
still occupy the DNS reactor until completion; broader asynchronous registration
and metadata observation remain separate work.

`Unregistration` owns one accepted RPC. Its record is moved, not borrowed, and
every returned resource is disposed by `CallResult`. The operation binds producer
reply readiness to the shared DNS CQ before publication; its remaining deadline
participates in the reactor's earliest wait. Repeated unrelated turns do not
renew it. Exact generation success, no-match, explicit rejection and uncertainty
are separate terminal outcomes. No-match also covers authorization rejection;
it is not proof that the namespace was already removed. Timeout, unexpected
positive results and poll failure are uncertain, not permission to replay.
Drop uses the existing pending close/cancel policy, not a second retirement RPC.

The existing pending vector reserves fallibly before submission and is bounded
to 64 cleanup RPCs. Storage/preparation/submission rejection has no admitted
recovery owner: DNS follows its fail-closed domain policy while the containing
transaction remains live. Rejected CQ binding returns the accepted owner and
retains it through nonreturning domain abort. This is a deliberate fail-closed
availability limit, not owner-preserving retry, a new custody registry or cleanup
success. Raft-commit cleanup and the publication scan share the existing
`local_cleanup_submitted` claim for the same captured publication, preventing
both paths from accepting separate retirement RPCs. There is no timer
polling/retry fallback for accepted cleanup. The
reactor retains its separate clock/transport watchdog for other sources; the
whole service is not event-only.

## Evidence and remaining boundaries

Three host codec controls cover exact/golden bytes, 255/256-byte and trailing-zero
names, action/generation semantics, invalid writers, every truncated prefix,
overflow, trailers, bad magic and retired records. Four controls exercise the
production authorization predicate and generic pending owner with fake calls:
manager/registrar and stale denial, CQ binding, exact terminal results, retained
binding failure, unchanged deadlines and Drop. They do not inject live allocator,
mapping or physical-release failure.

The linked ARM/x86 fixture delegates a separate long echo alias to DNS, which
becomes its NS registrar. It observes that publication, rejects
retired and stale generation requests without changing it, publishes/invokes
both catalog names, retracts both after endpoint death, and observes the node-local
DNS-owned long-name tombstone with read-only named lookup. That bounded fixture observation
does not submit a removal to make cleanup pass. It does not qualify native
wrong-principal rejection or all 256-byte publication paths. See
[batch evidence](../reports/audits/2026-10-14-protocol-simplification-name-generations.md)
and the existing [C18 family](cleanup-recovery.md).

Other packed NS/DNS entry points, other raw endpoint watches, transport/receive
adapters, metadata admission, outer IRQ/Drop contexts and physical completion
remain separate. Historical IPC exact-exit and later persistence intermittence
remain unresolved.

The reference echo service accepts an optional byte-valued
`cluster` (`charlotte_launch::echo_config::CLUSTER_NAME`) manifest entry. It delegates its endpoint to DNS for
that distinct name before serving; DNS then owns its NS publication. Lookup and
publication use one original five-second budget with owned calls and no retry.
Wrong manifest types reject. The native leader fixture supplies this entry;
ordinary echo launches keep only their existing local registrations.

## Formal publication projection

[`CharlotteNamePublication`](../tla/CharlotteNamePublication.tla) covers public
issuance/replacement gates, watch-before-mutation admission, serialized policy
and registry publication, current-watch confirmation, logical retirement and
watch-before-source disposal. A matching stale CQ hint cannot substitute for the
current producer's result. Rejected waits and unexpected observations retain the
registry through abort. Two finite instances and thirteen negative controls are
recorded in the [model review](../reports/audits/2026-10-18-name-publication-model.md).
Kernel authentication, actual watch admission, trusted 65-slot preparation,
allocation/destructor contexts and physical completion remain assumptions;
this supplies no native capacity-saturation or liveness qualification.
