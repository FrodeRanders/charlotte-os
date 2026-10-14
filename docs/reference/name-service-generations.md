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
confirmed missing, unpublished or dead endpoint returns `ERR_NOT_FOUND`; inability
to prepare/check the endpoint watch returns rejection instead. The owned client
preserves `Result<Option<generation>, Error>`: only a confirmed absence is `None`.
The agent's existing convenience API still returns `Option` for readiness only;
DNS explicit tombstoning uses the checked result and submits no command on error.

Retirement requires the exact current generation and a kernel-authenticated,
nonzero stable principal equal to the recorded registrar, or a service manager.
A missing principal/registrar grants no authority. Provisioning still verifies
the captured address-space generation, principal and roles through PolicyStore;
its own manager identity performs the existing policy unpublish. A stale or
unauthorized request returns `ERR_NOT_FOUND` and leaves the publication intact.
On success the registry keeps its generation tombstone and detaches its one raw
connection into `Connection` at a documented unique adapter boundary. Existing
best-effort local close does not establish physical backing quiescence or refund.
This does not convert the rest of NS's raw registry/lookup adapters to Rust owners.
DNS does not acquire retirement authority by looking up an independently
published endpoint. Such cleanup returns no-match and cannot clear another
registrar's binding. Retiring dead third-party namespace entries needs a separate
NS-owned endpoint-death or explicit retirement-delegation contract; this remains
open. Neither a catalog tombstone nor endpoint death grants DNS manager rights.

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

Other packed NS/DNS entry points, raw endpoint watches, transport/receive
adapters, metadata admission, outer IRQ/Drop contexts and physical completion
remain separate. Historical IPC exact-exit and later persistence intermittence
remain unresolved.

The reference echo service accepts an optional byte-valued
`cluster` (`charlotte_launch::echo_config::CLUSTER_NAME`) manifest entry. It delegates its endpoint to DNS for
that distinct name before serving; DNS then owns its NS publication. Lookup and
publication use one original five-second budget with owned calls and no retry.
Wrong manifest types reject. The native leader fixture supplies this entry;
ordinary echo launches keep only their existing local registrations.
