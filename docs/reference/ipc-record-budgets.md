# IPC record admission

Connections, retained pending calls and outstanding reply tokens have separate
count dimensions, independent of endpoint queues and scheduler waiter entries.

| Record dimension | Per sponsoring namespace generation | Node | Ordinary-domain node share |
| --- | --- | --- | --- |
| Connection capabilities | 512 | 8,192 | 6,144 |
| Retained pending calls | 512 | 8,192 | 6,144 |
| Outstanding reply tokens | 512 | 8,192 | 6,144 |

These are initial kernel policies, not signed deployment settings. Platform
classification comes from the kernel's exact live generation designation,
never a name, application role or supplied manifest field. The remaining node
share is a shared reserve, not an allowance or progress guarantee per service.

## Who sponsors a record?

Direct minting/delegation charges the **grantor**; a call's delegated connection
charges its caller. Each re-delegation creates and charges a new capability
against its own grantor. Unsolicited grants therefore cannot make a recipient
pay for connection metadata. A connection returned in a solicited reply instead
charges the **requesting caller**, preventing repeated lookups from consuming
the serving grantor's connection allowance. A receiver may hold connections
sponsored by several namespaces: 512 is not a per-recipient table-size limit,
and sponsorship does not confer authority to close the recipient's capability.

Pending calls and their reply tokens are sponsored by the **requesting caller**.
A client that creates outstanding requests consumes its own record budget,
rather than the target service's. A reply token is admitted at call submission,
before queuing: dequeue merely publishes its already-admitted receiver capability.

The caller retains its pending-call charge after completion or result observation,
until that record is closed. The reply-token charge returns on reply or cancellation.
Connection charges remain until actual capability removal, including queued
delegation cancellation and unobserved returned-result cleanup. An observed
returned connection is independent of its former pending call.

## Preparation and retirement

Every call path reserves both record dimensions atomically and then prepares
its fallible waiter list before moving, copying, lending or vector-transferring
attachments. A connection-bearing call also admits its delegated connection
before copying memory. Staged Rust owners return all reservations on rejection
or attachment failure; record admission does not consume the attachment.

A connection-bearing reply reserves its connection before consuming the token,
moving memory or revoking a loan. Kernel admission rejection leaves the token
and loan live. The current consuming `ReplyToken::reply_connection*` runtime
methods instead close/cancel the token on any failed syscall; they do not return
a retryable token. The borrowed grant source survives this error. Applications
should handle the returned error rather than construct raw-handle retry logic.

Namespace teardown fences record admission before collecting capabilities to
drain. Receive and connection-publication paths reject a retiring recipient,
preventing a late receiver token or delegated connection from escaping that
snapshot. Generation-qualified memory retirement is checked too. Existing
record charges retain the original reference-counted account after its namespace
is removed; a late remote close cannot credit a replacement using the same ASID.

Counter locking is IPC registry → domain → node. Identity checks may take the
address-space table and memory ledger under IPC, but release them before record
counters are acquired. Counter guards allocate nothing, invoke no callbacks and
enter no other subsystem. Notifications still run after releasing IPC.

## Errors and verification

Kernel exhaustion returns `IpcError::ResourceLimit`. The existing create/mint/call
submission ABIs return zero on failure, so owned clients retain their existing
creation/submission errors without a quota-specific status. Reply APIs with a
status result report `ipc_status::RESOURCE_LIMIT` (10). No ABI, launch descriptor
parameter or application-facing accounting endpoint was added.

Kernel boot tests exercise actual connection, retained completed-call and
outstanding-call ceilings; every call variant's pre-transfer rejection; staged
attachment-failure rollback; rejected reply with a live loan; observed/unobserved
returned authority; queued delegation cancellation; IPC retirement fencing;
and forced ASID reuse. Ordinary and total node saturation is counter-only,
not maximum-footprint registry allocation. The scoped EL0 probe saturates
connections and 512 outstanding calls using owning batches, drops them, and
verifies recovery. Its fifteen-bit mask is unchanged. A host owner test covers
resource-limited replies cancelling only their token, not their grant source.

These count budgets do not bound attachment bytes, all capability kinds,
observer/list control blocks, weak-only retention, loader/page tables, or the
entire kernel heap. Registry/capability insertion and namespace-account allocation
remain infallible. There is no per-principal aggregate across several domains,
signed override, allocator-failure injection, exhaustive race proof or hostile
pressure-soak claim. SEC-07 remains partial; the audit's deployment restrictions
still apply.
