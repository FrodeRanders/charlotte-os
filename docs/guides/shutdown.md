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

## Node-drain propagation

The deployment agent is itself lifecycle-aware. When it observes a
`NodeShutdown` request, it stops catalog reconciliation before launching any
new work, and marks every ordinary and operational child as retiring. It first
polls ordinary applications until their address spaces have been reclaimed;
only then does it begin retiring their privileged Kafka/S3 operational
connectors. Applications therefore retain the external-service capabilities
needed to finish or abort transactions during their grace window. Child
lifecycle records carry `NodeShutdown`, rather than disguising the transition
as a deployment update.

The enclosing node deadline and the descriptor's signed grace period are both
authoritative upper bounds. For each child, the kernel publishes the earlier
of `now + shutdown_grace_ms` and the node deadline. A node coordinator can
therefore shorten a grace period when little node-drain time remains, but it
cannot extend one. Repeated polls likewise cannot move an established deadline
later. An artifact already retiring because of a deployment change is upgraded
to the node-shutdown reason and its deadline may only become earlier.

Only after ordinary deployments and privileged operational connectors have
both drained does the agent acknowledge its own lifecycle request and exit.
Kernel counters distinguish acknowledged and forced node-shutdown retirements
from ordinary deployment replacement outcomes.

## Cluster-authorized shutdown ingress

Whole-node poweroff begins as an operator-signed `CSHUTDN1` envelope, not as
an application request. The fixed 144-byte record binds a nonzero target node
key, a per-node monotonic sequence, a UTC validity interval, the whole-node and
per-phase grace budgets, and the `power-off` reason. Current policy permits at
most 24 hours of validity, 15 minutes of total grace, and 60 seconds per phase;
the phase grace cannot exceed the node grace. There is no broadcast target, so
a controller can preserve quorum while sequencing a cluster rollout.

`cluster-sign shutdown-sign` creates the envelope off-cluster and
`shutdown-notify` posts it to `POST /v1/shutdowns`. Any member may accept the
bounded request. A follower relays the complete signed proof over relmsg; the
leader re-verifies it against the committed cluster key and trusted UTC before
proposing it. The catalog state machine verifies the signature again, fences
stale or conflicting per-node sequences, stores the exact envelope in its
snapshot, and exposes only locally applied state to node agents.

The target deployment agent verifies the applied envelope again and moves its
memory owner through `catten_rt::owned::request_node_shutdown`. The kernel
consumes that capability on every result, admits only its uniquely delegated
agent, independently verifies the signature and target node against the NIC
driver's protected status page, and derives the actual deadline in the kernel
monotonic epoch. A kernel thread then owns the complete service and device
coordinator, so retiring the agent itself cannot orphan the shutdown. Failed
or unverified device quiescence still refuses terminal poweroff.

The signed intent is desired state: while it remains the latest replicated
record, a restarted target drains and powers off again. A later lifecycle
policy must use a higher sequence; the v1 format intentionally has no unsigned
clear operation.

## Reverse-dependency node services

After application-domain propagation, the kernel has a bounded node-service
coordinator. It transfers the published steady-state handles out of the launch
registry so no observer can use them to initiate new work, then advances only
after every domain in the current phase has exited and been generation-safely
reclaimed. The production order is:

1. deployment ingress, deployment control, deployment agent;
2. HTTP ingress and time;
3. cluster catalog/Raft, reliable messaging, and discovery;
4. TCP/IP and the frame router; and
5. object store.

Every phase has its own bounded grace, capped by the enclosing node deadline.
Dropping an unfinished coordinator requests forced termination for ordinary
service domains, matching the owner fallback used for deployments.

Deployment ingress, HTTP ingress, and the UTC time service now honor this
request directly. The two ingress services stop opening listeners and poll
their current accept or receive operation with a bounded lifecycle check.
Dropping their resource-owning serving scopes closes the listener and service
connections; an HTTP request already admitted may finish within the phase
grace. The time service drops its public endpoint and unwinds any pending NTP
call, UDP socket, and persistence connection as one owning scope. None of the
three begins new work after observing the lifecycle request.

Bounded socket receive uses an explicit `OP_CANCEL_RECV` exchange. Merely
closing a local pending-call capability is insufficient because tcpip retains
the remote reply token until data arrives. The cancellation exchange releases
that server-side slot before `receive_timeout` returns, so lifecycle polling
cannot discard the first bytes of the next request or strand a deferred reply.

The transport phase is cooperative as well. TCP/IP runs only after all socket
consumers have drained; it closes admission, completes retained receive calls,
stops any residual protocol sockets, and drops its endpoint and owned NIC
connection. The frame router then drops one owning scope containing its
single NIC receive call, deferred name lookups, routed service connections,
and in-flight moved-frame calls. Only after both domains exit can the node
coordinator transfer the NIC driver to device quiescence.

Startup is part of the same lifecycle. A long-lived service must not use an
unbounded dependency lookup before it starts polling `Context::lifecycle()`.
`catten_services::wait_for_local_ready_or_shutdown` performs bounded,
non-blocking name lookups with owned timer waits; HTTP, time, and TCP/IP can
therefore acknowledge a drain even when the node-ready gate never opens. The
kernel coordinator records acknowledged, unacknowledged, and forced counts per
phase before it reclaims each domain.

The deployment plane and cluster core follow the same rule. Deployment ingress
closes its listener; cluster control drops its endpoint and service
connections; and the agent interrupts dependency/reconciliation waits, drains
applications before their operational connectors, then releases its catalog
connections. DNS stops its Raft/catalog reactor and cancels pending transport
and catalog work. Reliable messaging rejects retained sends, completes its
deferred receive, and releases queued message pages before discovery stops its
probes and pending cluster-status waiters. `sleep_ms_or_shutdown` prevents a
configured retry or reconciliation interval from becoming shutdown latency.

The object-store phase is cooperative and durable. On `NodeShutdown` the
service closes its public endpoint before doing any more work, retries the
block-device flush until it succeeds (or the supervisor's deadline forces the
domain), then drops its owned block connection and transient IPC resources
before acknowledging readiness. A flush error therefore cannot be reported as
a successful graceful shutdown.

The service coordinator stops at `AwaitingDeviceQuiescence` and transfers the
remaining NIC, block, and entropy drivers to a separate device-shutdown owner
only after their consumers have gone. That owner publishes the authenticated
lifecycle request but deliberately has no force-abort transition. An ordinary
`READY` acknowledgement is insufficient: a driver must release its endpoint,
drain operations and transient DMA mappings, flush durable state where
applicable, mask interrupts, stop or reset the controller, publish
`DEVICE_QUIESCED`, and exit. Only then does the kernel reclaim the address
space and its launch-owned MMIO, interrupt, and IOMMU grants. A deadline overrun
or exit without the stronger acknowledgement retains the domain and device
state for diagnosis instead of risking DMA into reallocated memory.

Every in-tree hardware adapter now implements the contract. VirtIO entropy
resets status zero before tearing down its shared-DMA buffers and MMIO mapping.
NVMe closes admission, drains outstanding replies and DMA, completes a final
NVM Flush, masks interrupts, clears `CC.EN`, and observes `CSTS.RDY=0`.
VirtIO block completes its protocol flush before resetting status zero. AHCI
completes ATA Flush Cache, masks port interrupts, and stops both the command
and FIS receive engines. VirtIO net and E1000E close admission and deferred
receives, release queued frames, drain outstanding transmit descriptors, then
reset or halt their receive/transmit engines and mask interrupts. A failed
flush or drain still attempts to stop DMA, but withholds `DEVICE_QUIESCED` and
retains the domain rather than claiming a graceful shutdown.

After successful device quiescence, AArch64 has a terminal architecture
transition. The kernel reads the FADT Arm boot-architecture flags instead of
assuming an emulator-specific calling convention, selects the advertised SMC
or HVC conduit, masks local interrupts, and invokes PSCI `SYSTEM_OFF`. A return
from that firmware call is fatal: a node that has crossed the terminal drain
boundary must never resume scheduling. The AArch64 `--shutdown-test` requires
QEMU itself to exit only after the authoritative successful test result and
the PSCI request marker have reached the serial log.

The per-domain lifecycle request is authenticated by memory protection: only the kernel
can mutate the launch page. The application can acknowledge a request but
cannot clear it, extend its deadline, or request shutdown of another domain.

## Scope

This contract currently covers signed off-cluster shutdown admission,
follower-to-leader relay, replay-fenced Raft persistence, target-agent pickup,
kernel re-verification, deployment and HTTP ingress, the deployment agent, all
agent-owned applications and operational connectors, the UTC time service,
the high-level node-service order above, durable object-store shutdown, and all
in-tree hardware adapters. Service-specific `OP_SHUTDOWN` messages remain
useful for tests and targeted live upgrade, but are not the deployment
lifecycle authority. Device reset and IOMMU invalidation remain
adapter-specific proofs rather than a uniform hardware attestation record.
PSCI poweroff is implemented and boot-validated on AArch64; x86-64 still needs
its ACPI poweroff backend.
