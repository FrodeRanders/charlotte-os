# Cooperative shutdown and forced retirement

CharlotteOS treats shutdown as a bounded lifecycle transition, not as an
assumption that destructors run when a thread is killed. Deployed applications
receive requests through their kernel-owned, userspace-read-only launch page.
They acknowledge the request through the domain status page, release owned
resources, and exit. The kernel forcibly retires all remaining domain threads
on the deployment agent's first retirement poll at or after the signed
deadline. While a retirement is active, the agent polls at most 10 milliseconds
apart.

## Application pattern

Put the serving loop and all of its owned resources in a function that returns
`ShutdownRequest`. Returning normally drops local owners and lets explicit
remote teardown report or log errors before `complete()` performs the
divergent thread exit:

```rust
use catten_rt::{Context, ShutdownRequest, owned::Endpoint};

fn serve(ctx: &Context) -> ShutdownRequest {
    let endpoint = Endpoint::create(INTERFACE, VERSION, 16).unwrap();
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            // Perform any fallible remote close here. Local owners are
            // dropped when this function returns.
            return request;
        }

        match endpoint.try_receive() {
            Ok(Some(message)) => handle(message),
            Ok(None) => sleep_or_wait_with_a_bound(),
            Err(_) => catten_rt::domain_abort(),
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}
```

Do not call `thread_exit()` from inside the resource-owning scope: it does not
unwind the Rust stack. Do not block forever on `Endpoint::receive()` or another
unbounded wait in a lifecycle-aware application. Use non-blocking operations
plus a bounded timer, or a completion-queue wait with a timeout, so the loop
checks `shutdown_requested()` regularly.

Transactional work needs protocol cleanup before returning. A Kafka step
drops its `Transaction` to invoke the abort fallback and releases its delivery
token. The Kafka connector explicitly aborts an open broker transaction and
leaves its consumer group. S3 and ordinary endpoint services close their
owned connections, sockets, mappings, and endpoints through their existing
RAII owners.

## Deployment and kernel behavior

Current tooling emits `CDEPLOY4`. Its `shutdown_grace_ms` field is signed with
the artifact digest, placement, stack pages, thread quota, and grants. Valid
values are zero through 300,000 milliseconds; zero requests immediate forced
retirement. `CDEPLOY1` through `CDEPLOY3` remain readable and receive the
5,000-millisecond compatibility default.

On the first `DeployedArtifact::poll_retire()` call, the kernel:

1. publishes `DrainRequested`, the reason, and an absolute monotonic deadline
   into the read-only lifecycle record;
2. leaves the domain runnable while the agent continues polling;
3. reclaims the address space immediately if every domain thread exits; or
4. publishes `ForceTerminating` and aborts all remaining threads on the first
   retirement poll at or after the deadline.

The agent retains the `DeployedArtifact` owner until reclamation completes.
Dropping an unfinished owner sends an immediate best-effort force request; it
does not claim that asynchronous reaping has completed. Ordinary
reconciliation must retain the owner and keep polling until the kernel reports
completion, both to provide the cooperative window and to observe final
reclamation.

The lifecycle request is authenticated by memory protection: only the kernel
can mutate the launch page. The application can acknowledge a request but
cannot clear it, extend its deadline, or request shutdown of another domain.

## Scope

This contract currently covers agent-owned deployed applications and
operational connectors. Service-specific `OP_SHUTDOWN` messages remain useful
for tests and targeted live upgrade, but are not the deployment lifecycle
authority. Coordinated whole-node poweroff still needs a replicated drain
intent, reverse dependency ordering for platform services and drivers, durable
flush, DMA/device quiescence, and a final architecture poweroff operation.
