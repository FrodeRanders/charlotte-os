# Kafka service roles and procedural workers

Charlotte should expose Kafka authority by role rather than make every
application implement a broker loop. The roles share the same protocol codec,
TLS transport, metadata handling, producer fencing, and owned-resource model;
they are separate authority surfaces, not three unrelated Kafka stacks.

## Broker connector boundary

The broker-facing deployment unit is `kafka.elf`. Only that service receives
the immutable profile capability containing the broker address, TLS trust
material, topic/partition authority, group, transactional identity, and future
SASL or mTLS credentials. IP addresses are not normally secrets, but they are
still deployment configuration and authority that application code does not
need. The profile capability is never delegated onward.

Logic above the connector receives only a connection capability to the
specific provisioned Kafka service instance. Its IPC calls select bounded route
indices and resource owners; they cannot read credentials or substitute a
broker, group, transactional identity, or topic. A deployment controller can
therefore replace or rotate the connector profile independently of rebuilding
the producer, consumer, transactional-step, or procedure artifacts.

For a transactional step the authority chain is:

```text
launcher/controller --read-only secret profile--> kafka.elf
launcher/controller --Kafka connection---------> kafka-step
launcher/controller --procedure connection-----> kafka-step
kafka-step        --bounded invocation----------> activity procedure
```

The current data-plane supports verified TLS trust anchors but not SASL user
names/passwords or client-certificate authentication. Those credentials should
be added to the same connector-only profile when the authentication mechanism
is implemented; they must not be added to the application or procedure ABI.

## Producer role

A producer endpoint grants `RIGHT_PRODUCE` and an immutable allow-list of
topic/partition routes. Calls select a small route index, never an arbitrary
topic string. This is suitable for ingress, lifecycle events, and applications
whose correctness does not depend on consuming an offset in the same
transaction.

## Consumer role

A consumer endpoint grants `RIGHT_CONSUME` for one topic/partition/group tuple.
`Consumer`, `Delivery`, and `DeliveryToken` remain linear owners. Dropping a
delivery releases it for redelivery; committing consumes it. Several such
endpoint capabilities may be held by one application when their commits are
independent.

## Transactional-step role

Durga activities should normally use a generic transactional-step service. Its
profile contains one consume route, every allowed output route (including DLQ
and lifecycle routes), the consumer group, and a fenced transactional identity.
The profile is a versioned, SHA-256-authenticated object delivered as a
kernel-enforced read-only launch capability. Its configurable route ceiling is
bounded by the current hard maximum of 64, so deployment policy can choose a
smaller authority/work limit without being constrained by config-page slots.
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

The current low-level Kafka endpoint already supports one fixed consume route
plus allow-listed produce routes in a single transaction. The remaining work
for the higher-level transactional-step role is the bounded procedure
request/reply ABI, capability injection by the launcher/controller, retry and
timeout policy, and observed-state integration for readiness and rollout.
