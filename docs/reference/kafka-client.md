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
The launcher encodes profile version 7 (`CHKAFP7`), calculates a SHA-256
integrity digest, and transfers its memory capability with kernel-enforced
`MAP_READ` rights only.
Typed profile-capability metadata carries the exact meaningful byte length; it
is not hidden in a generic capability-flags field. Before opening a socket,
`kafka.elf` validates that length, the version, digest, UTF-8 DNS host-label
syntax, field bounds, route and broker counts, safety ceilings, and duplicate
routes or broker destinations. The hash detects corruption;
profile provenance and immutability come from the trusted launcher and the
read-only capability, not from an unkeyed digest.
Only `kafka.elf` receives this profile. Producer, consumer, or transactional
step logic receives a connection capability to that specific service instance,
not the profile or its broker/TLS/SASL configuration. Client-certificate
secrets reside in the same connector-only profile and remain absent from the
application-facing IPC protocol.

Authentication is one closed choice: `None`, `ScramSha256`, `MtlsP256`,
or `ScramSha256AndMtlsP256`. The fixed header declares the four credential
lengths; fields for disabled mechanisms must be empty and fields for selected
mechanisms must satisfy their existing nonempty size/syntax bounds. Unknown
choices and retired formats reject before broker connection. There are no
optional/critical extension sections or skipped authentication material.
The encoder copies borrowed credentials directly into the final profile;
there is no second temporary credential-section allocation. The trusted
launcher borrows the encoder's returned zeroizing owner through its existing
launch scope, and the connector retains its existing zeroizing password/private-key
copies.

### Current profile wire contract

The only accepted profile is `CHKAFP7\0`, version 7, with an exact 112-byte
header. Integers are little-endian; the total meaningful length must equal the
input length. SHA-256 covers the entire record with bytes 16--47 zeroed during
hashing. Reserved bytes and unknown TLS flags reject.

| Offset | Field |
| --- | --- |
| 0 | Eight-byte magic |
| 8, 10, 12 | Version `u16`, header length `u16`, total length `u32` |
| 16 | 32-byte integrity digest |
| 48, 52, 54 | IPv4 `[u8; 4]`, port `u16`, TLS flags `u16` |
| 56, 64, 68 | Rights `u64`, transaction timeout `u32`, consume partition `i32` |
| 72, 74 | Produce route count and declared ceiling, each `u16` |
| 76, 78, 80, 82 | Host, topic, group and transactional-ID lengths, each `u16` |
| 84, 88 | CA length `u32`, additional broker count `u16` |
| 90 | Authentication choice `u16`: 0 none, 1 SCRAM, 2 mTLS, 3 both |
| 92, 94 | Username and password lengths, each `u16` |
| 96, 98 | Instance-name length and authority-endpoint count, each `u16` |
| 100, 104 | Client-certificate and private-key lengths, each `u32` |
| 108 | Four zero reserved bytes |

Payload order is instance name, host, CA, topic, group, transactional ID,
username, password, client certificate and private key, then produce routes,
additional brokers and authority endpoints. Empty disabled fields occupy no
payload bytes. Existing route (8-byte header), broker (8-byte header) and
authority (16-byte header) records retain their bounds and zero-reserved fields.
No suffix or padding is accepted. `CHKAFP1`--`CHKAFP6` and the retired 104-byte
section layout reject even with a correct integrity digest; old encrypted
operational envelopes must be regenerated from reviewed current profiles,
not relabelled or filled with defaults. Their outer envelope and catalog
formats are unchanged: connector decoding remains the inner-format authority.

Profile-backed launch is prepared as one kernel transaction. Until the initial
thread starts, an owning launch object retains the unloaded domain and any
kernel-owned profile memory. Connection delegation, profile allocation, or
read-only transfer failure closes all earlier resources and the target address
space; `Drop` remains a best-effort fallback for future early-return paths.

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

The scoped launch API gives the runner `grantctl` and a signed descriptor
rather than a name-service connection. The descriptor can authorize the exact
connector and procedure names, and `catten_services::grant_client` returns only
those attenuated connections. The node agent supports signed central-S3 pull
and scoped launch for arbitrary artifact names. Generated Kafka-step rollout
still needs the release and rollback controller plus per-component connector
provisioning; artifact-name specialization is no longer a blocker.

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

The connector status page exposes cumulative requests, produced and consumed
records, commits, aborts, backpressure, group generation/assignment,
heartbeats, rebalances, metadata refreshes, reconnects, retry attempts,
application-visible terminal errors, and fencing events. It also publishes
current metadata age and consumer lag plus one cumulative produced-record
counter per profile route. Monitoring derives rates from the per-route
counters; the connector does not maintain an unbounded time series.

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

The shared transport uses the pinned Embassy-based TLS API and one domain-owned
entropy context installed before consumer publication. Its exact revisions,
one-time ownership/admission, typed mTLS parser and grouped compile-time
capacities are documented in [Owned TLS client](tls-client.md).

The shared TLS transport enables AES round-key and GCM authentication-key
scratch erasure. The current AArch64 software POLYVAL backend also erases its
state on ordinary Drop, with a build guard requiring destructor presence. The
freestanding x86 target selects the erasing software state and requires its
destructor too, avoiding the pinned non-dropping dispatcher. Removing its backend
flag or erasure feature rejects the build. PCLMUL authentication acceleration is
disabled; hosted x86 outside that target policy is unqualified. Optional ARM PMULL
still rejects the AArch64 guard. TLS traffic-key/IV/HKDF state and compiler copies
remain separate SEC-15 work. See the [x86 follow-up](../reports/audits/2026-10-25-security-x86-gcm-owner.md), [AES review](../reports/audits/2026-10-21-security-aes-erasure.md) and
[GCM review](../reports/audits/2026-10-22-security-gcm-erasure.md).

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

SCRAM now shares HMAC with SigV4. HMAC and PBKDF2 return zeroizing result owners;
the shared SHA context and compression scratch also erase on ordinary teardown.
The [secret-state inventory](secret-state-lifetimes.md) distinguishes these
owned fields from caller inputs, scalar/compiler copies and third-party state.

The exchange has two consuming phases. `Client::receive_server_first(self, ...)`
releases its password/transcript owner on success or rejection and returns
`ClientFinal` on success. That phase owns its proof bytes and expected server
signature together. `message()` only borrows the proof for encoding;
`receive_server_final(self, ...)` consumes both fields on every outcome,
including empty or oversized input. Reconnect begins with a new `Client`.
The configured credential owner remains available for future connections; only
the exchange-local copy ends at challenge processing. See the
[phase-ownership report](../reports/audits/2026-10-19-security-scram-phases.md).

Base64 helpers construct their zeroizing output owners before writing. Encoding
returns `Zeroizing<String>`; decoding reserves its complete output bound once
and returns `Zeroizing<Vec<u8>>`. Late decode rejection drops the same prefix
owner. Nonce/proof/verifier callers retain the returned owner directly. Local
quantum digits and packed words also have erasing owners. This does not cover
transport copies; see the [scratch report](../reports/audits/2026-10-19-security-base64-scratch.md).

`sasl_authenticate_request` validates the complete payload bound before copying
and returns `Zeroizing<Vec<u8>>` from one reserved allocation. Reply parsing
consumes a `Zeroizing<Vec<u8>>` on success or rejection. `SaslAuthenticate`
retains that complete frame and lends its authentication range through
`auth_bytes()`; the former independent `auth_bytes` vector is removed. The
owner implements neither `Clone` nor `Debug`. The connector releases
its first request/reply buffers when challenge handling returns the final
SCRAM phase. See the [frame report](../reports/audits/2026-10-19-security-sasl-frames.md).

The broker transport now retains `FrameReceiver` and owned receive chunks.
It validates the inline four-byte header before reserving the complete reply,
then copies into that allocation without growth. Consumed chunk ranges erase;
coalesced replies retain their remaining bytes in the same chunk owner.
Completed frame ownership continues through routing into the parser. A failed
or interrupted read fences the receiver until reset; the connector discards
its old stream and resets on send/read failure and before new authentication.
This adds no request replay permission. TLS/plain chunks also erase on ordinary
release; the shared TLS helper creates its owner before calling the reader.
Library record/key-schedule state and kernel socket backing remain separate
qualification. See the [receive report](../reports/audits/2026-10-19-security-receive-owners.md).

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
- metadata-driven leader and coordinator routing, with bounded leader refresh
  for produce/fetch and coordinator rediscovery during group and transactional
  operations;
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
listeners, creates three-replica `charlotte-events` and `charlotte-results`
partitions on different leaders, and boots an
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
- the bounded observability layout, including metadata refreshes, terminal
  authorization errors, coordinator refreshes, output-route production, and
  non-negative consumer lag; and
- the generic step runner's success, retry, timeout, and terminal-DLQ paths.

Once the connector has discovered its initial routes, the guest opens a
bounded fault window. The host kills the broker leading `charlotte-results`,
keeps it down beyond the connector's bounded receive timeout, and then
restarts it. The verifier requires the transaction to finish through the
replacement leader, at least one routing retry, and a successful post-fault
metadata refresh. A steady-state run that never encounters the injected fault
therefore fails.
The fixture chooses group and transactional IDs coordinated away from the
killed broker so this scenario measures partition-leader movement independently
of the separate coordinator-migration case.

Broker exchanges are synchronous today. The connector therefore requests a
45-second consumer-group session while heartbeating every two seconds; the
lease exceeds its bounded dead-peer detection and metadata-refresh window.
This prevents a recoverable leader migration from expiring the group generation
before the transaction can include its consumed offset. A future concurrent
broker-exchange loop can decouple heartbeats from data-plane latency.

Run it with:

```sh
scripts/run-aarch64.sh --kafka-test --timeout 300
```

The companion coordinator-movement scenario reads the transaction coordinator
selected by Kafka from the connector's startup diagnostics and kills that
broker after bootstrap. It requires transaction-coordinator rediscovery before
accepting the same end-to-end transaction:

```sh
scripts/run-aarch64.sh --kafka-coordinator-test --timeout 300
```

The producer-fencing scenario launches a second connector with the same
transactional ID but distinct capability names. Kafka advances the producer
epoch, after which a private probe asks the original connector to produce. The
test requires `ERR_FENCED`, a non-zero fencing counter, and a connector fault
diagnostic; the fencer capability and both connectors' credentials remain
absent from the probe's application ABI:

```sh
scripts/run-aarch64.sh --kafka-fencing-test --timeout 300
```

The certificate identifies the three `kafka-N.test` endpoints; their
provisioned transports connect to QEMU's user-network gateway at
`10.0.2.2:19092`, `:19094`, and `:19096`. The fixture and
its volumes are removed on exit. Set `CATTEN_KAFKA_IMAGE` to exercise another
compatible Kafka image. The switch adds the fixture, test trust anchor, and
verifier; the ordinary network, DHCP, TCP/IP, entropy, and time services are
not test-only functionality.

## Encoded launch-profile lifetime

`charlotte_protocol_kafka::Profile::encode` returns
`Result<Zeroizing<Vec<u8>>, ProfileError>`. Validation and the complete checked
frame size precede allocation. The encoder installs the erasing owner before
writing any credentials, retains its existing single zero-filled frame and
returns the owner directly. No plain-output compatibility encoder remains.
The kernel's `launch_kafka_profile` already wrapped the former plain result;
it now uses the returned owner without another wrapper, allocation or copy.
The owner stays live while the launcher prepares the read-only profile memory.

The borrowed `Authentication` and `Profile` types have no aggregate Debug
implementation. Existing borrowed Copy/Clone/equality behavior remains; profile
Clone still allocates its metadata vectors while borrowing credential fields.
Individual fields and encoded bytes can still be explicitly copied or formatted.
This does not qualify every diagnostic/provisioning path. CHKAFP7 bytes, integrity
digest, authentication choices, bounds and rejection behavior are unchanged.

Ordinary Drop erases the encoded frame after launch or abandonment. Its source
credentials, original launch memory, connector-local profile copies, third-party
TLS state, digest/scalar intermediates and physical memory retain separate
contracts. Drop requires execution. See the
[profile-encoding report](../reports/audits/2026-10-20-security-kafka-profile-encoding.md) for baseline controls, the independent wire
fixture and native regression evidence.

## SCRAM initial-phase lifetime

`charlotte_kafka::scram::Client::new` computes the complete escaped username and
initial-transcript byte lengths before allocation. It writes comma/equal escapes
directly into the reserved erasing transcript; the separate escaped-username
string is removed. Original UTF-8 bytes and SCRAM transcript/proof bytes are
preserved. Empty inputs and invalid nonce bytes retain their existing rejection
policy. Checked length overflow rejects as `InvalidCredential`; this is no
aggregate heap-admission or fallible-allocation guarantee.

The phase now owns its nonce as `Zeroizing<String>`. Password and nonce owners
exist before their bytes are copied. The constructor uses three allocations,
without growth: transcript, password and nonce. Ordinary Drop or a consuming
server-challenge transition releases all three, including rejection. The final
phase retains its existing proof/verifier policy. The service's original
configured credentials and generated nonce have their separate lifetimes.
Nonces and usernames remain SCRAM wire metadata. No new secrecy, authentication,
replay or remote-cleanup authority follows from local erasure. See the
[initial-owner report](../reports/audits/2026-10-20-security-scram-initial-owners.md) for release controls, the Python reference and
native regression evidence.
