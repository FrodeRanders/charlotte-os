# Kafka service roles and procedural workers

Charlotte should expose Kafka authority by role rather than make every
application implement a broker loop. The roles share the same protocol codec,
TLS transport, metadata handling, producer fencing, and owned-resource model;
they are separate authority surfaces, not three unrelated Kafka stacks.

## Broker connector boundary

The broker-facing deployment unit is `kafka.elf`. Only that service receives
the immutable profile capability containing the broker address, TLS trust
material, topic/partition authority, group, transactional identity, and
connector authentication credentials. IP addresses are not normally secrets, but they are
still deployment configuration and authority that application code does not
need. The profile capability is never delegated onward.

Logic above the connector receives only a connection capability to the
specific provisioned Kafka service instance. Its IPC calls select bounded route
indices and resource owners; they cannot read credentials or substitute a
broker, group, transactional identity, or topic. A deployment controller can
therefore replace or rotate the connector profile independently of rebuilding
the producer, consumer, transactional-step, or procedure artifacts.

Profile version 6 separates a diagnostic connector instance name from a
bounded set of application-facing authority endpoints. Each endpoint has a
non-empty printable-ASCII name of at most 256 bytes and a Kafka-rights ceiling
that must be a subset of the connector's profile ceiling. The connector checks
that ceiling against every opcode received on that exact endpoint before
decoding request memory or creating resources. Deployments can therefore
publish `kafka/claims/producer`, `kafka/claims/consumer`, and
`kafka/claims/transactional` from one secret-bearing connector without giving
all three callers uniform authority. The application cannot create or rename
an access point.

Connector-only authentication is encoded as bounded, length-delimited profile
sections. Known SCRAM-SHA-256 and P-256 mTLS sections are critical; an older
decoder may skip a future optional section but must reject an unknown critical
one. Section count and aggregate bytes are bounded before any credentials are
exposed to the broker adapter.

For a transactional step the authority chain is:

```text
launcher/controller --read-only secret profile--> kafka.elf
launcher/controller --Kafka connection---------> kafka-step
launcher/controller --procedure connection-----> kafka-step
kafka-step        --bounded invocation----------> activity procedure
```

The current data-plane supports verified TLS trust anchors, connector-only
SASL/SCRAM-SHA-256 credentials, and connector-only P-256 mTLS identities.
Usernames, passwords, client private keys, salted keys, nonces, and broker
challenges never cross the application IPC boundary; password, private-key,
and derived-key copies use zeroizing storage. SCRAM and mTLS may be provisioned
independently or together in the same connector profile.

## Producer role

A producer access point grants `RIGHT_PRODUCE` and an immutable allow-list of
topic/partition routes. Calls select a small route index, never an arbitrary
topic string. This is suitable for ingress, lifecycle events, and applications
whose correctness does not depend on consuming an offset in the same
transaction.

## Consumer role

A consumer access point grants `RIGHT_CONSUME` for one topic/partition/group tuple.
`Consumer`, `Delivery`, and `DeliveryToken` remain linear owners. Dropping a
delivery releases it for redelivery; committing consumes it. Several such
endpoint capabilities may be held by one application when their commits are
independent.

Each connector is one Kafka group member using the bounded
`charlotte-fixed-v1` assignor. Connectors with the same authorized partition
elect one active member while the others heartbeat with empty assignments as
standbys. Group-generation and member identity fence ordinary and transactional
offset commits. A rebalance revokes outstanding delivery authority and aborts
an in-flight transaction before installing the new assignment. This supports
Charlotte replicas without revealing group credentials or broker authority to
their application procedures; it does not interoperate with consumers that do
not advertise the Charlotte assignor.

## Transactional-step role

Durga activities should normally use a generic transactional-step service. Its
profile names one Kafka connector and one procedure endpoint, admits a bounded
set of output route indices (including the DLQ), and sets the procedure
timeout, retry backoff, maximum attempt count, idle-poll interval, and maximum
output count. Broker destinations, the consume route, group, transactional
identity, TLS material, and credentials remain solely in the separately
provisioned connector profile.
The profile is a versioned, SHA-256-protected object delivered as a
kernel-enforced read-only launch capability. Its configurable route ceiling is
bounded by the current hard maximum of 64, so deployment policy can choose a
smaller authority/work limit without being constrained by config-page slots.
The hash detects corruption; the launch capability is the authenticity and
provenance boundary.
The service performs this sequence:

1. poll one input delivery;
2. call the activity's procedure endpoint with borrowed input bytes and
   immutable invocation metadata;
3. validate the reply against the output route allow-list and size limits;
4. begin a Kafka transaction and produce all returned records;
5. include the input delivery's next offset; and
6. commit, or abort and release for redelivery on any failure.

The activity procedure does not receive a Kafka connection and cannot commit,
drop, or leak a delivery. Its generated handler is ordinary request/reply code,
which removes Kafka cleanup ladders from application development. A procedure
that needs S3 or another service receives that capability separately through
its launch contract.

This path is implemented by `kafka_step.elf` and the bounded `no_std`
`charlotte-kafka-step` ABI. The runner keeps the pending procedure call,
borrowed input, delivery token, and transaction in owners whose `Drop`
implementations cancel or release an incomplete attempt. A successful reply may
return up to 16 records; every route is checked against the step profile before
the transaction begins. Retry replies and timeouts cause redelivery after the
configured backoff. Terminal or malformed replies, and retry exhaustion, copy
the original record to the configured DLQ and include its input offset in the
same transaction. Kafka transport or commit failures abort and redeliver
without consuming the procedure-attempt budget.

The transaction covers Kafka records and offsets only. External side effects
cannot be made part of a Kafka transaction; such BPMN activities need an
outbox, an idempotency key, or an explicit compensation/saga policy.

## Capability and deployment consequences

The deployment descriptor must distinguish the generic transactional-step
service instance from the activity procedure artifact. It binds the two with a
specific procedure connection and gives the step service only the broker
profile it needs. The procedure gets no ambient name-service authority beyond
what is required to resolve explicitly declared dependencies.

For the first deployment controller, a step instance and its procedure may be
co-located using a shared `PlacementPolicy` affinity group. Correctness must not
depend on co-location: migration and restart require a generation-fenced
procedure capability and a fenced Kafka transactional identity.

The runner publishes bounded readiness and operation counters for polls,
invocations, output records, commits, retries, DLQ records, timeouts, aborts,
and fatal startup errors. The AArch64 Kafka fixture exercises successful output,
explicit retry, procedure timeout and terminal DLQ paths through the generic
runner.

The connector publishes a separate bounded operational view: metadata refresh
count and age, reconnects, retry attempts, terminal failures, fencing events,
group generation/heartbeats/rebalances, consumer lag, and cumulative produced
records for every profile route. Per-route throughput is derived by sampling
those monotonic counters; no application receives the status capability merely
because it can produce or consume.

The development launcher currently resolves the exact access-point and procedure names
through its bootstrap name-service connection. A production deployment
controller must instead mint and inject only those two connection capabilities,
manage service-generation fencing, and interpret the status page for rollout.
Fixed-partition Kafka group membership is implemented in the connector;
controller-managed stable instance leases and cooperative assignor
interoperability remain future work.
