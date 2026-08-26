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
- `catten_services::kafka_client` is the owned application API.

## Provisioning and authority

One `KafkaProfile` grants one broker endpoint, topic, partition, consumer
group, transactional identity, and rights mask. Applications receive only the
service connection capability. They cannot select another broker, topic,
partition, group, or transaction identity at runtime.

The rights are `RIGHT_PRODUCE`, `RIGHT_CONSUME`, and `RIGHT_TRANSACTION`.
A trusted launch component calls `launch_kafka_profile`; application code must
not receive or reconstruct the launch manifest.

The manifest keys are:

| Key | Type | Meaning |
| --- | --- | --- |
| `kfk_ip` | four bytes | Resolved broker IPv4 address |
| `kfk_host` | bytes | Provisioned broker host identity |
| `kfk_port` | unsigned | Broker TCP port |
| `kfk_tls` | unsigned | Transport security selection |
| `kfk_ca` | bytes | Reserved DER trust anchor for TLS |
| `kfktopic` | bytes | Fixed topic |
| `kfkpart` | unsigned | Fixed partition |
| `kfkgroup` | bytes | Fixed consumer group |
| `kfktxn` | bytes | Fixed transactional ID |
| `kfkright` | unsigned | Rights mask |
| `kfktout` | unsigned | Broker transaction timeout in milliseconds |

## Owned application API

Resolve `kafka`, keep the returned owned connection, and borrow it through a
client:

```rust
use catten_services::{
    kafka,
    kafka_client::Client,
    wait_for_registered_name_owned,
};
use charlotte_protocol_kafka::RecordRequest;

let (_, connection) = wait_for_registered_name_owned(ns, kafka::NAME)?;
let client = Client::new(connection.as_ref());
let offset = client.produce(RecordRequest::new(Some(b"key"), Some(b"value")))?;
```

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

## Kafka behavior and failure semantics

The service negotiates and validates the exact legacy request versions it
implements. It uses idempotent producer IDs and sequence numbers outside
transactions as well as a separately initialized transactional producer.
Fetch uses RecordBatch v2 and `read_committed`; aborted transactional batches
are filtered using the broker's aborted-transaction ranges and control
markers.

The initial consumer profile is intentionally assignment-based rather than a
Kafka group-protocol member. It reads and commits the configured group's
offset for one fixed partition but does not participate in dynamic group
rebalancing. Run only one active CharlotteOS profile for a given
group/topic/partition tuple.

Network operations and coordinator initialization use bounded waits. A
transport error can be ambiguous: the broker may have accepted a request
before the reply was lost. The service preserves an idempotent producer
sequence until success is known, but applications must still treat a failed
transaction as aborted and start a new transaction after rediscovery/retry.

## Current interoperability boundary

The implemented broker protocol was exercised against Apache Kafka 4.1.1. It
uses the non-flexible request versions listed in `charlotte-kafka` for metadata,
coordinators, offsets, produce/fetch, producer initialization, and transaction
operations.

This first profile is deliberately narrow:

- one statically provisioned IPv4 broker and one partition;
- no metadata-driven multi-broker routing or leader migration;
- no dynamic consumer-group membership/rebalancing;
- no compression, headers, SASL, DNS, or TLS; and
- records are bounded by the one-page application ABI.

`kfk_tls=1` is rejected rather than downgraded. The CA manifest key reserves a
compatible provisioning boundary for a future verified TLS transport. For a
centrally managed cluster, TLS and the site's SASL mechanism must be added
before production deployment; do not expose a plaintext listener merely to
fit the current client.

## Docker integration test

The opt-in runner starts a fresh single-node Apache Kafka KRaft container,
creates `charlotte-events`, and boots an in-guest smoke application that checks:

- idempotent production;
- read-committed consumption and explicit offset commit;
- atomic consume-transform-produce with `AddOffsetsToTxn` and
  `TxnOffsetCommit`; and
- abort filtering.

Run it with:

```sh
scripts/run-aarch64.sh --kafka-test --timeout 300
```

The fixture advertises QEMU's user-network gateway at `10.0.2.2:19092` and is
removed with its volumes on exit. Set `CATTEN_KAFKA_IMAGE` to exercise another
compatible Kafka image. The switch adds the fixture and verifier; the ordinary
network, DHCP, TCP/IP, and time services are not test-only functionality.
