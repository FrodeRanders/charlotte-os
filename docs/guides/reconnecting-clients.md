# A client that can replace its service connection

The [copyable Rust example](../../crates/catten-services/examples/reconnecting_client.rs)
uses the current userspace APIs. The manual includes this file directly in
Chapter 5, “Service identity and discovery.” It performs three read-only greeting
requests and can replace its connection before continuing the unfinished read.

The example requires a signed deployment descriptor granting `CALL` on `greet`,
an allowed principal/service policy and a running publication of that service.
For another protocol, replace the service name, opcode and reply decoder together.
The bootstrap connection is the grant controller for this application; it is
borrowed through `Context`, along with the immutable descriptor.

## Recovery scopes

| Scope | What stays live | What happens on failure |
| --- | --- | --- |
| Application | Launch authority, completed-read cursor and current operation deadline | Owns the decision to stop, acknowledge shutdown or restart a session. |
| Discovery/session | One owned `Connection` acquired through signed grants | Endpoint loss returns to discovery. The old connection drops before reacquisition. |
| Business operation | The unfinished read and its original deadline | Only a validated reply advances the cursor. A new operation then receives a new budget. |
| Accepted call | `PendingCall`, including any Rust memory loan | Reply observation ends it. Timeout or rejected wait admission cancels explicitly; rejected close retains the owner through terminal abort. |

The example allows at most three sessions across the run. Lookup, reconnection
and the current read share one five-second monotonic deadline via
`grant_client::acquire_until`. Reconnection never restarts that deadline.
Shutdown is checked between acquisitions and calls. This is a dedicated client;
a reactor that must process inbound work uses [one CQ for all interests](../reference/cq-wait.md).

## Error and replay policy

`ServiceRestarted` is the example's own policy classification. Current runtime
calls report `IpcError::Status(ipc_status::ENDPOINT_CLOSED)` before acceptance,
or `IPC_REPLY_ENDPOINT_CLOSED` in a terminal reply. Neither proves that a
replacement exists. A name's new generation does not retarget or revoke a
still-live old connection. The client must obtain another capability through
its original authority.

Only endpoint loss enters the reconnect branch. Authorization, admission,
invalid capability and protocol errors stop the application. Timeout, a
cancelled reply and rejected wait admission are also terminal here. The latter
retains a pending owner until cancellation; it is distinct from a timeout.
Unexpected returned memory or connections remain owned by `CallResult` during
validation and are disposed on rejection.

Repeating this greeting read after endpoint loss is safe because it has no
business side effects. A mutation needs a protocol-specific recovery contract,
such as durable request IDs, deduplication and outcome queries. Cancellation
does not undo an effect already performed. Keep durable business progress and
reconciliation outside the connection scope. This example's in-memory cursor
survives connection replacement within one process only.

An explicit close can outlive the observation budget while loans are revoked.
If close rejects, the returned owner and enclosing connection stay live through
`domain_abort`; the example does not drop a borrow and continue. That is the
runtime's fail-closed policy, with the wider cleanup limits recorded in
[resource ownership](resource-ownership.md).

## Current discovery limitation

Applications have no publication-readiness subscription. Grant acquisition uses
bounded availability backoff, capped by the caller's remaining deadline. This is
timer-driven backoff; accepted RPC replies use the owned readiness wait. Timer
admission failure follows the existing fail-closed runtime policy. A supervisor
can later restart the client, but it must preserve any unresolved business
operation's recovery policy. The example establishes no general overload,
teardown-latency or production-readiness guarantee.

## Validation

From the repository root, build the example as a freestanding ELF for each
architecture:

```sh
cargo build --locked -p catten-services --example reconnecting_client \
  --target crates/catten-services/aarch64-unknown-none.json
cargo build --locked -p catten-services --example reconnecting_client \
  --target crates/catten-services/x86_64-unknown-none.json
bash scripts/run-host-tests.sh
```

These builds produce unsigned ELFs. Use the normal
[signing and deployment workflow](signing-and-trust.md) before launching one.

The [example's host tests](../../crates/catten-services/examples/reconnecting_client/tests.rs)
check endpoint-loss classification, refusal to replay cancellation and exact
business-reply validation. Cross-target compilation checks the actual SDK
types and consuming operations. These checks do not execute a deployed
lookup/restart/cancellation episode or inject native cleanup failure.
