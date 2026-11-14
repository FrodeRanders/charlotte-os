# Application SDK and owned service operations

Use `charlotte-app` for ordinary application setup and service clients.
`catten-rt` remains the resource-owning runtime, while protocol crates own wire data.
Platform service code can use the same clients through `catten-services`, which
reexports their single implementation. The application package has no dependency
on the platform service implementations.

## Application structure

The [reconnecting example](../../crates/charlotte-app/examples/reconnecting_client.rs)
is a complete, compiling client with explicit business recovery policy. It uses:

- `Application::from_context(&ctx)` to borrow the launch-owned controller and
  signed descriptor, and `acquire` returns an owned connection after controller
  authorization. `publish` borrows an endpoint and a caller-owned deadline.
- `charlotte_app::entry!` with `fn(&Context) -> Result<AppExit, E>`. The returning
  scope drops its owners before the wrapper exits, acknowledges an authenticated
  shutdown request, or aborts on an application error. Join workers before
  returning, because exiting one thread does not drain a domain.
- `catten_rt::deadline::Deadline` for a monotonic business-operation budget.
  Reconnection and admission do not reset it.
- `PendingCall::observe_until` for `Reply`, `Deadline`, `WaitRejected` or `Failed`.
  Deadline and wait-rejection variants retain the original pending owner and its
  memory loans. Explicit `close` can return that owner on rejection.
- `CallObservation::cancel_on_failure` when the caller deliberately chooses the
  existing fail-closed cancellation policy. Cleanup can outlive the observation
  budget or abort on rejected loan cleanup, but it does not roll back remote effects.

`Admission::new(&deadline)` offers scalar, copied and moved submissions.
`Admission::with_lifecycle` also checks the borrowed lifecycle between attempts.
Only a rejected `QUEUE_FULL` submission is retried. Moved submission errors
return `(OwnedMemory, AdmissionError)` with the exact original request.
Queue capacity currently has no producer readiness subscription, so this
specific unstarted condition uses bounded one-millisecond timer backoff.

`OwnedPayload::encode(capacity, limit, encoder)` supplies a bounded mutable slice
to an existing wire encoder, validates its initialized length, unmaps and returns
one owner. `copy_from` handles byte payloads. `into_parts` returns unmapped memory
and exact length for `call_move`, while `memory()` supports copying. An empty payload
still has an owned attachment. Errors identify allocation, mapping, encoding or
unmapping, and private owners follow their existing release policy.

Optional `charlotte-app` features are `socket`, `kafka`, `s3` and `test-support`.
Connector credentials and broker-facing transports remain in platform services.
The synchronous connector methods retain their documented blocking/remote-close
behavior, because extraction alone does not turn them into futures or impose one budget
on every protocol session.

## Serving and workers

`service::Server` owns one endpoint, binds it to CQ zero and drains ready messages
before parking. It checks lifecycle between requests and after wakes. The handler
receives the complete `IncomingMessage`, including authenticated sender identity,
attachments and one-shot reply authority. Publication is explicit after startup, and
the scope permits one publication attempt, preventing accidental replay after an
uncertain registration. Handler and wait errors leave the endpoint in the server.
This is a synchronous request/reply helper, not a general multiplexing executor.
The [echo-service example](../../crates/charlotte-app/examples/echo_service.rs)
shows signed publication and resource release before shutdown acknowledgement.

`service::Workers` reserves a bounded vector before accepting native
`ThreadHandle`s. `join_all` confirms exit, whereas registration/observation failure
restores the exact generation-bound handle. The caller first signals workers
through its application lifecycle. Dropping a nonempty group aborts rather than
claiming that dropping a join observation ended a worker. Group insertion
rejection returns the actual handle, which still needs to be joined or retained.
It does not supply scoped borrowed closure spawning or custody of Sitas join
handles. Owned captures such as `Arc<Connection>` can share one actual connection
owner between workers without duplicating raw capability ownership.
An already-created `OwnedSocket<'_>` still borrows that connection, so capturing an
`Arc` does not make the borrowed socket transferable to a `'static` worker.
Concurrent network applications can retain their borrowed sockets in one bounded
CQ-driven scope, as the broker demo now does. Shared socket custody or scoped
Sitas spawning remains an extension for applications requiring thread handoff. See
the [broker review](../reports/audits/2026-11-14-broker-sdk-review.md).

## Sitas call futures

`charlotte-app-sitas` uses the existing Sitas `ShardExecutor` and reactor backend.
Construct `WakeReactor::new(backend, cq, capacity)` before the executor and retain
its `handle()`. The selected `cq` must be the queue actually waited by `backend`, and
for a default `CharlotteReactor::new(lp)` use queue zero. The backend is the sole
ring consumer. Register an accepted call with
`calls.call(pending, Some(deadline))`, then await the returned future.
Registration or binding failure returns `(PendingCall, RegistrationError)`.

The wrapper reserves its fixed interest storage fallibly. Task-waker callbacks
and destruction leave its lock. Each backend wake rechecks registered owners,
because an IPC reply may produce a queue wake without a cookie record. The
wrapper clips the backend wait to the earliest operation deadline. Future polling
only polls the owned call, and it never performs a blocking reply wait. Registration
precedes that readiness recheck. At expiry the future returns the pending owner,
not a fabricated terminal result.

A rejected backend wait logs and aborts the domain with the executor's owners
retained. The pinned Sitas backend captures spawn generations and owns its exit
observations. The wrapper's terminal wait policy also prevents older executors
from treating rejected admission as an unbounded retry hint.

Future Drop removes its registration and uses the existing pending-call
cancellation policy, which may block or abort. Use `into_pending` for explicitly
managed cancellation. The returned owner remains CQ-bound, so observe or close it
rather than binding it a second time. Endpoint/completion futures and Sitas
worker-group custody remain extensions. Lifecycle wakes CQ zero, so multi-shard applications must
explicitly coordinate shutdown across their other queues.

## Export and create a project

```sh
scripts/export-app-sdk.sh --output /tmp/charlotte-app-sdk.tar.gz
mkdir -p /tmp/charlotte-sdk
# Extract into an application-owned directory.
tar -xzf /tmp/charlotte-app-sdk.tar.gz -C /tmp/charlotte-sdk
/tmp/charlotte-sdk/charlotte-app-sdk/charlotte doctor
/tmp/charlotte-sdk/charlotte-app-sdk/charlotte new /tmp/orders
/tmp/charlotte-sdk/charlotte-app-sdk/charlotte test /tmp/orders
/tmp/charlotte-sdk/charlotte-app-sdk/charlotte build /tmp/orders \
  --arch aarch64 --output /tmp/orders.elf
```

Use `--arch x86_64` for the other target. `new` creates an independent Cargo
workspace with a host-testable business library and a small EL0 binary. It
refuses to overwrite an existing directory. The generated application borrows
its launch authority, and it is intended for the normal signed deployment path.

The CLI's `test` and `build` commands currently select this starter's
`<name>-logic` package and `app/Cargo.toml` binary. Importing an existing
multi-crate workspace is not implemented. Such applications can use the
lower-level `build-external-elf.sh --manifest PATH --bin NAME --output PATH`
interface with explicitly reviewed Cargo pins, and the builder itself does not
replace that dependency review.

The SDK contains runtime/client source, the owned async adapter, CI-pinned
`sitas-core`, `sitas-charlotte` and `sitas-unix`, the `charlotte-kafka` client
codec for host conformance tests, platform definitions, a standalone signer and
public development keys. `sdk.json` records runtime ABI, toolchain, OS and Sitas
revisions, source dirty state and hashes of the exact distributed files. The CLI
checks those hashes and the project's SDK fingerprint before running commands.
These consistency checks are not signed provenance or a replacement for the
artifact/deployment trust model. External Cargo dependencies and Rust toolchain
components are resolved normally, so a fully offline distribution is not provided.

`charlotte.json` is a local development plan and SDK binding. It is **not** a new
kernel/deployment format. Stack pages, thread quota, shutdown grace and grants
still require the existing reviewed, signed `CDEPLOY6` descriptor.

To sign a built ELF, pass an explicit restricted key file:

```sh
/tmp/charlotte-sdk/charlotte-app-sdk/charlotte package /tmp/orders \
  --elf /tmp/orders.elf --key-file /path/to/private-key \
  --version 1 --rollback 1
```

The command stages signer inputs/build output under the application, signs the
ELF in place and stops there. It does not choose production credentials, upload
an artifact or submit a deployment. Use the existing
[signing](signing-and-trust.md) and [deployment](../reference/deployment-ingress.md)
workflows for the remaining handoff.

## Host tests and qualification

`client::ScalarService` is a narrow service boundary for business code.
`ScalarClient` implements it using a borrowed connection, the caller's deadline
and checked scalar reply shape. With `test-support`, `ScriptedScalar` checks the
ordered opcode/argument sequence and supplies typed success/failure outcomes.
A mismatched request is rejected without consuming its expected script step.
Scripts never fabricate kernel capability owners or execute the native backend.

Run package tests from outside the repository's freestanding Cargo configuration:

```sh
cd /tmp
cargo +nightly-2026-07-27 test --manifest-path /path/to/charlotte-os/Cargo.toml \
  -p catten-rt -p charlotte-app -p charlotte-app-sitas --all-features --lib
```

The repository host suite also selects the application's optional-feature tests.
Runtime tests inject allocation, mapping, unmapping, admission and observation
failures, and check retained loans and returned-capability cleanup. Async tests
exercise registration capacity, drop and readiness/deadline ordering. The Sitas
EL0 probe adds a moved-memory IPC round trip through the real executor, because host
fakes alone cannot qualify native CQ wake delivery or memory revocation.
The [14 November qualification checkpoint](../reports/audits/2026-11-14-application-sdk.md)
records the host, target, SDK and native results, including the initial HVF
build failure and subsequent build correction. HVF execution remains unqualified.

The [broker SDK adoption checkpoint](../reports/audits/2026-11-14-broker-sdk-adoption.md)
records an existing-workspace consumer using the public manifest/binary builder,
single-snapshot source dependencies, CQ-driven sessions and explicit worker joins.
