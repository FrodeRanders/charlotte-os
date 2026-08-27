# Kafka client service

CharlotteOS has a native, capability-oriented Kafka data-plane client. It can
produce idempotently, consume with `read_committed` isolation, and atomically
commit produced records together with consumed offsets in a Kafka transaction.
It does not embed a general-purpose hosted Kafka library.

The implementation is split at the userspace boundary:

- `charlotte-kafka` is a transport-independent `no_std` Kafka protocol and
  RecordBatch v2 codec;
- `charlotte-protocol-kafka` defines the bounded application/service ABI and
  contains no live capabilities;
- `kafka.elf` owns the broker socket, producer IDs, epochs, sequence numbers,
  group state, and transactional identity; and
- `catten_services::kafka_client` is the owned application API; and
- `kafka_step.elf` plus `charlotte-kafka-step` provide a generic,
  capability-separated consume--procedure--produce runner.

## Provisioning and authority

One `KafkaProfile` fixes a connector instance identity and grants an
allow-list of at most 32 broker endpoints, a fixed
consume topic/partition, an ordered allow-list of produce topic/partition
routes, a consumer group, transactional identity, and connector rights ceiling.
It also declares one or more named authority endpoints, each with its own
subset of that ceiling. Applications receive only the connection capability
for their declared authority endpoint. They cannot name an arbitrary
broker or topic at runtime: `Route::provisioned(n)` selects an entry already
admitted by the launch profile, and the service rejects every other index.

The rights are `RIGHT_PRODUCE`, `RIGHT_CONSUME`, and `RIGHT_TRANSACTION`.
A trusted launch component calls `launch_kafka_profile`; application code must
not receive or reconstruct the launch profile.

The complete profile is one versioned, immutable object rather than a set of
config-page entries. It contains broker addresses and TLS identities/trust
anchor, consume route, ordered produce routes, consumer group, transactional
identity, optional connector-only SASL credentials, rights, transaction timeout,
and an operator-selected route ceiling.
The launcher encodes profile version 5, calculates a SHA-256 integrity digest, and
transfers its memory capability with kernel-enforced `MAP_READ` rights only.
Before opening a socket, `kafka.elf` validates the exact object length, version,
digest, UTF-8 hostnames, field bounds, route and broker counts, safety ceilings,
and duplicate routes or broker destinations. The hash detects corruption;
profile provenance and immutability come from the trusted launcher and the
read-only capability, not from an unkeyed digest.
Only `kafka.elf` receives this profile. Producer, consumer, or transactional
step logic receives a connection capability to that specific service instance,
not the profile or its broker/TLS/SASL configuration. Client-certificate
secrets reside in the same connector-only profile and remain absent from the
application-facing IPC protocol.

The instance name and every access-point name are non-empty printable ASCII and
at most 256 bytes. They are covered by the profile digest and cannot be
selected or changed through the Kafka application ABI. Short names use the
compact name-service operation; longer names use the memory-carried operation.
The connector publishes only the declared access points. On each call it maps
the opcode to its required Kafka rights and rejects missing rights with
`ERR_DENIED` before decoding attached memory or touching delivery/transaction
state. A producer-only caller therefore cannot open a consumer or transaction,
even when the same connector also publishes a transactional access point.

The implementation hard ceiling is 64 produce routes. A deployment may set
`KafkaProfile::max_produce_routes` lower; both the declared limit and actual
route count must be at most 64. This bound limits startup work and memory while
remaining independent of the launch manifest's 32-record capacity.

## Owned application API

Resolve the authority-endpoint name from the deployment contract, keep the
returned owned connection, and borrow it through a client:

```rust
use catten_services::{
    kafka_client::Client,
    wait_for_registered_name_bytes_owned,
};
use charlotte_protocol_kafka::RecordRequest;

let (_, connection) =
    wait_for_registered_name_bytes_owned(ns, b"kafka/orders/producer")?;
let client = Client::new(connection.as_ref());
let offset = client.produce(RecordRequest::new(Some(b"key"), Some(b"value")))?;
```

Route zero is the fixed consume topic and preserves the original API.
Additional routes are numbered from one in `KafkaProfile::produce_routes`
order. Both ordinary and transactional production can select them:

```rust
use catten_services::kafka_client::Route;

let mut transaction = client.begin_transaction()?;
transaction.produce_to(
    Route::provisioned(1),
    RecordRequest::new(None, Some(b"result")),
)?;
transaction.include(delivery_token)?;
transaction.commit()?;
```

Producer sequence numbers and `AddPartitionsToTxn` state are tracked per route.
The primary endpoint and `KafkaProfile::broker_endpoints` are the only network
destinations. Each tuple supplies an expected broker-advertised hostname/port
and a separately provisioned IPv4 address. Kafka metadata may select a broker
only when its advertised hostname and port exactly match one of those tuples;
metadata never grants authority to an address. Startup requests metadata for
all distinct profile topics in one exchange and routes requests to the selected
leaders and group/transaction coordinators. The authorized endpoints also act
as alternate metadata seeds. Retriable produce/fetch leader errors refresh
metadata before an idempotent retry.

An application may hold several independently provisioned Kafka service
connections. This is useful for disjoint producer or consumer authorities,
but a Kafka transaction cannot span those services because each owns a
different producer identity. Atomic consume--transform--produce therefore
uses one profile whose consume route and every output route are declared
together.

`Consumer`, `DeliveryToken`, and `Transaction` are linear owners. There is no
public raw-ID constructor.

- `Consumer::poll` issues at most one outstanding delivery for that consumer.
  This is the service's bounded backpressure credit.
- `DeliveryToken::commit(self)` advances the group offset. Dropping the token
  releases it without advancing, so the record can be delivered again.
- `Transaction::include(token)` consumes a delivery token and stages its next
  offset in the transaction.
- `Transaction::commit(self)` atomically commits produced records and included
  offsets. `abort(self)` aborts them explicitly. Dropping a live transaction
  performs a best-effort abort.
- `Consumer::close(self)` reports remote teardown failure; `Drop` remains the
  fallback for early returns and panics.

The record bytes arrive in an `OwnedMemory`. `Delivery::into_parts` returns the
delivery token, memory owner, and validated key/value ranges. Keep the memory
behind its mapping borrow while inspecting those ranges.

## Generic transactional-step runner

Business procedures that only transform an input record should not own the
Kafka connection or its delivery and transaction resources. A trusted launcher
can instead start `kafka_step.elf` with a read-only `charlotte-kafka-step`
profile. That profile names the connector and procedure instances, lists the
only output route indices the procedure may select, identifies one required
DLQ route, and bounds outputs, attempts, timeout, backoff, and polling.

The procedure receives a borrowed `DeliveredRecord` memory object and the
one-based attempt number. It replies with either no output, a bounded encoded
`OutputBatch`, retry, or terminal failure. The runner validates the entire
batch before beginning a transaction, produces every admitted record, includes
the input delivery, and commits. Dropping its single transient operation owner
cancels/releases the pending resources on every early return. Procedure
timeouts and retry replies redeliver; terminal or invalid replies and retry
exhaustion transactionally write the original record to the DLQ. Broker-side
failures abort and redeliver without charging the business attempt count.

The current development launch path resolves exact connector and procedure
names through the name service. Production deployment still needs a grant
controller that directly injects those connections, so neither the procedure
nor unrelated callers receive ambient connector authority.

## Kafka behavior and failure semantics

The service negotiates and validates the exact legacy request versions it
implements. It uses idempotent producer IDs and sequence numbers outside
transactions as well as a separately initialized transactional producer.
Fetch uses RecordBatch v2 and `read_committed`; aborted transactional batches
are filtered using the broker's aborted-transaction ranges and control
markers.

Each connector joins its configured group with the bounded
`charlotte-fixed-v1` assignor. A member advertises exactly the topic/partition
authorized by its immutable profile. The group leader assigns each advertised
topic/partition to one member; duplicate members remain heartbeating standbys
with an empty assignment and can acquire the partition after a join, leave, or
session failure. One connector admits one local consumer because the connector
owns one group membership. Deploy additional independently named connectors
for replicas or other partitions.

Deliveries and offset commits are generation-fenced. Ordinary `OffsetCommit`
requests carry the current generation and member ID. Transactional commits use
Kafka's flexible `TxnOffsetCommit` v3 encoding and carry the same identity. On
a rebalance, the connector revokes outstanding deliveries, aborts a touched
transaction, synchronizes the new assignment, and reloads the committed offset
before polling resumes. A stale commit returns `ERR_FENCED`; it cannot silently
advance the new owner's offset. The service loop maintains heartbeats with a
monotonic deadline even when no application request arrives.

This fixed-partition protocol is for Charlotte connector members. A conventional
Kafka consumer that does not advertise `charlotte-fixed-v1` cannot join the same
group. It is deliberately smaller than Kafka's subscription assignors: topic
discovery and arbitrary partition selection remain deployment-controller work.

Network operations and coordinator initialization use bounded waits. A
transport error can be ambiguous: the broker may have accepted a request
before the reply was lost. The service preserves an idempotent producer
sequence until success is known, but applications must still treat a failed
transaction as aborted and start a new transaction after rediscovery/retry.

## TLS and SASL security boundary

Setting `KafkaProfile::tls` selects the shared owned TLS transport and never
downgrades to plaintext. The client verifies the broker's certificate chain,
DNS identity from `kfk_host`, validity interval against synchronized UTC from
the time service, and its signature using the explicitly provisioned DER trust
anchor. It currently uses TLS 1.3 with AES-128-GCM-SHA-256.

Handshake randomness comes first from the architecture RNG syscall and then
from the optional `rng` service backed by protected VirtIO RNG DMA. If trusted
time, entropy, or verification is unavailable, connection establishment fails
closed. Broker sockets and both TLS record buffers are aggregated in
`catten_services::tls_client::OwnedTlsStream`, so reconnect and error paths
release the whole transport by dropping one owner.

`KafkaAuthentication::ScramSha256` authenticates the connector as a Kafka
principal after TLS establishment. Every bootstrap, seed, leader, and
coordinator connection performs `SaslHandshake` followed by the bounded
SCRAM-SHA-256 exchange before carrying data-plane requests. Client nonces come
from the same system entropy path as TLS. The client enforces the RFC 7677
minimum iteration count, caps broker-selected work and message/salt sizes,
verifies the server signature in constant time, and erases passwords and
derived keys when their owners are dropped. Authentication is rejected unless
verified TLS is enabled, so credentials cannot be sent over plaintext.
Profile usernames and passwords are bounded to 256 and 1,024 printable ASCII
bytes respectively; this deliberately avoids silently applying an incomplete
Unicode SASLprep implementation.

The profile supports `None`, `ScramSha256`, `MtlsP256`, or
`ScramSha256AndMtlsP256`. The mTLS variants carry one DER-encoded X.509 client
certificate and its DER-encoded SEC1 P-256 private key. The connector signs the
TLS 1.3 client-authentication transcript with ECDSA P-256/SHA-256 on every
broker connection. Certificate and key sizes are bounded at profile decoding;
the key is parsed before the connection is opened and retained in zeroizing
storage. SCRAM-SHA-512, OAuth bearer tokens, Kerberos, certificate chains, and
other client-key algorithms are not yet implemented.

## Current interoperability boundary

The implemented broker protocol was exercised against Apache Kafka 4.1.1. It
uses the bounded request versions listed in `charlotte-kafka`; most are legacy
non-flexible forms, while generation-fenced transactional offset commit uses
the compact/tagged flexible v3 schema.

This first profile is deliberately narrow:

- up to 32 statically provisioned IPv4 broker destinations; one consume
  partition and up to 64 allow-listed produce topic/partition routes;
- metadata-driven leader and coordinator routing, with leader refresh for
  produce/fetch; coordinator migration during an active transaction remains
  limited;
- fixed-partition consumer-group join, sync, heartbeat, leave, standby failover,
  and generation fencing, but no topic-pattern subscription or cooperative
  assignor interoperability;
- no compression, headers, dynamic SASL mechanism negotiation, client
  certificate chains, external hostname resolution, or TLS 1.2; and
- records are bounded by the one-page application ABI.

For a centrally managed cluster, verify that SCRAM-SHA-256 and/or P-256 mTLS is
permitted, or add the site's required authentication mechanism; do not expose
an unauthenticated listener merely to fit the client. Verify that the broker
permits TLS 1.3 and a certificate signature supported by the selected embedded
TLS feature set.

## Docker integration test

The opt-in runner creates an ephemeral CA, server certificate, and P-256 client
identity, starts a fresh three-broker Apache Kafka KRaft fixture with verified
external TLS, required mTLS, and SCRAM-SHA-256 client authentication
listeners, creates `charlotte-events` and `charlotte-results` on different
leaders, and boots an
in-guest smoke application
that checks:

- idempotent production;
- read-committed consumption and explicit offset commit;
- atomic consume-transform-produce across both topics with per-route producer
  sequences, `AddPartitionsToTxn`, `AddOffsetsToTxn`, and generation-fenced
  flexible-v3 `TxnOffsetCommit`;
- group join/leave across successive consumers, generation advance, and
  heartbeat operation;
- abort filtering; and
- the generic step runner's success, retry, timeout, and terminal-DLQ paths.

Run it with:

```sh
scripts/run-aarch64.sh --kafka-test --timeout 300
```

The certificate identifies the three `kafka-N.test` endpoints; their
provisioned transports connect to QEMU's user-network gateway at
`10.0.2.2:19092`, `:19094`, and `:19096`. The fixture and
its volumes are removed on exit. Set `CATTEN_KAFKA_IMAGE` to exercise another
compatible Kafka image. The switch adds the fixture, test trust anchor, and
verifier; the ordinary network, DHCP, TCP/IP, entropy, and time services are
not test-only functionality.
