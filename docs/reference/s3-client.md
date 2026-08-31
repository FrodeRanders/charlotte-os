# S3 client service

CharlotteOS provides a native, capability-oriented S3 data-plane client for
communicating with centrally managed object stores. The initial compatibility
targets are RustFS and Dell EMC ECS.

The implementation is intentionally not a port of the AWS SDK. It is split at
the platform boundary:

- `charlotte-s3` is a transport-independent `no_std` core. It implements
  S3 URI/query canonicalization, HMAC-SHA256, AWS Signature Version 4, bounded
  HTTP/1.1 response parsing, and incremental chunked-transfer decoding.
- `charlotte-protocol-s3` defines the application/service wire ABI. It contains
  no live capabilities and makes object streams explicit remote resources.
- `s3.elf` owns TCP connections and credentials and publishes the short name
  `s3`.
- `catten_services::s3_client` is the safe application API. `GetObject` and
  `PutObject` close or abort in `Drop`, while explicit consuming methods report
  remote teardown failures.

## Authority and credentials

Each service instance is configured for exactly one endpoint, bucket, key
prefix, credential identity, and rights mask. The connection capability is the
application's authority. Applications never receive the access or secret key
and cannot escape the configured bucket/prefix.

The rights bits are:

| Bit | Constant | Operations |
| --- | --- | --- |
| 0 | `RIGHT_GET` | GET, ranged GET, and HEAD |
| 1 | `RIGHT_PUT` | streaming PUT |
| 2 | `RIGHT_DELETE` | DELETE |
| 3 | `RIGHT_LIST` | reserved for the forthcoming bounded `ListObjectsV2` ABI |

Do not pass credentials through application IPC or store them in application
objects. A trusted provisioner should construct `service::launch::S3Profile`
and call `launch_s3_profile`. The launcher encodes the bounded immutable
`CHS3PF1` profile, transfers it read-only into the new service, and starts the
domain only after the transfer succeeds. The S3 service retains its secret key
in zeroizing storage.

The manifest keys below are retained only for legacy static boot images. New
static launches and operational pickup use `charlotte_protocol_s3::Profile` as
the format authority. Operational profiles are stored centrally as signed
HPKE envelopes; the privileged node path fetches the ciphertext and connector
ELF, the kernel re-verifies their authorization, decrypts into transient
zeroizing memory, validates `CHS3PF1`, and moves the resulting profile directly
into connector launch memory. No plaintext profile is returned through agent
or application IPC.

The manifest ABI uses the following eight-byte-or-shorter keys:

| Key | Type | Meaning |
| --- | --- | --- |
| `s3_ip` | four bytes | Resolved endpoint IPv4 address |
| `s3_host` | bytes | DNS host name, without a port; used for HTTP `Host`, TLS SNI, and certificate hostname verification |
| `s3_port` | unsigned | TCP port |
| `s3_tls` | unsigned | `1` requires TLS; `0` permits development HTTP |
| `s3_ca` | bytes | DER-encoded X.509 trust anchor; required when TLS is enabled |
| `s3regn` | bytes | Explicit SigV4 region, normally `us-east-1` for ECS |
| `s3buck` | bytes | Fixed bucket |
| `s3pref` | bytes | Optional fixed key prefix, without a leading slash |
| `s3access` | bytes | Access-key identity |
| `s3secret` | bytes | Secret key, visible only in the service domain |
| `s3_ns` | bytes | Optional ECS namespace; sent and signed as `x-emc-namespace` |
| `s3rights` | unsigned | Bitwise rights mask |

Path-style addressing (`/<bucket>/<key>`) is used for both RustFS and ECS. An
explicit region is always signed; the client does not issue a bucket-location
probe. This avoids known compatibility differences in centrally managed ECS
installations. The configured host name, with a non-default port appended, is
signed and sent unchanged, which supports ECS behind a load balancer.

## Application use

Resolve `s3` through the local name service and retain the returned owned
connection. Construct a borrowed client from it:

```rust
use catten_services::{s3, s3_client::Client, wait_for_registered_name_owned};

let (_, connection) = wait_for_registered_name_owned(ns, s3::NAME)?;
let client = Client::new(connection.as_ref());
let (mut object, metadata) = client.get(s3::ObjectRequest::get(b"report.json"))?;

while let Some(chunk) = object.read()? {
    let (memory, length) = chunk.into_parts();
    let mapping = memory.map_read_only()?;
    consume(&mapping.as_slice()[..length]);
}
object.close()?;
```

For PUT, calculate SHA-256 before beginning the operation. A persistent local
object can be read once to hash and again to stream, avoiding the more complex
AWS streaming-signature extension:

```rust
let request = s3::ObjectRequest::put(b"report.json", length, sha256);
let mut upload = client.put(request)?;
upload.write(chunk)?; // consumes the moved-memory chunk on submission
let metadata = upload.finish()?;
```

`PutWriteError::NotSubmitted` returns the memory object to the caller. Once a
chunk has transferred to the service, a later error cannot return that memory;
dropping the upload still aborts the remote operation. Operations are bound to
the caller's kernel domain/generation and have a five-minute inactivity lease,
so guessed IDs or crashed applications cannot retain sockets indefinitely.

## Time and retry semantics

SigV4 requests require synchronized UTC. The service obtains Gregorian UTC
from the `time` service and returns `ERR_UNSYNCHRONIZED` until a fresh NTP
sample is available; persisted holdover time is deliberately insufficient for
authentication.

The current implementation does not automatically replay S3 operations. A
caller may safely retry HEAD and GET after a transport failure. Repeating PUT
is appropriate only when replacing the key with the identical body is allowed;
use `ObjectRequest::create_only()` when overwriting must be rejected. DELETE is
idempotent at the S3 protocol level.

TCP sends and receives are bounded rather than waiting forever. A timed-out
stream is closed and reported as a transport error. The service currently
serializes application calls, so a slow object-store response can delay other
callers until that bounded wait ends.

## TLS security boundary

Setting `s3_tls=1` selects the owned TLS transport and never downgrades to
plaintext. It currently implements TLS 1.3 with AES-128-GCM-SHA-256 using
`embedded-tls`. SNI and hostname verification both use `s3_host`; chain and
validity-time verification use the DER trust anchor in `s3_ca` and synchronized
UTC from the time service. A TLS profile without a trust anchor is rejected at
startup.

Handshake randomness comes from the kernel's Arm `RNDR` or x86-64 `RDRAND`
source when available. Otherwise the client uses the node-local `rng` service,
whose userspace driver owns a delegated VirtIO RNG MMIO capability and private
protected-DMA domain. Both paths are fallible and TLS fails closed when neither
trusted source is available. There is no deterministic fallback, `NoVerify`
mode, or plaintext retry after a TLS failure.

The initial profile accepts one trust anchor because the immutable profile has
a bounded data area. A production provisioner should select the narrowest CA
that authenticates the managed endpoint. Do not place a public Web PKI root
bundle or credentials in application memory.

TLS 1.2 is not implemented. Verify that a managed Dell ECS listener permits
TLS 1.3 before deploying this client against it. The selected TLS engine also
does not currently document guaranteed in-memory zeroization of its complete
session key schedule; treat that as remaining hardening work for hostile
memory-forensics threat models.

Plain HTTP remains available only when an explicitly provisioned profile sets
`s3_tls=0`, for isolated development endpoints.

## RustFS integration test

The opt-in runner path starts an ephemeral RustFS container with a freshly
generated P-256 test CA and server certificate, provisions the
`charlotte-test` bucket, then boots CharlotteOS. The in-guest smoke application
performs PUT, HEAD, GET, body verification, and DELETE through the same owned
client API applications use:

```sh
scripts/run-aarch64.sh --s3-test --timeout 240
```

Docker, Docker Compose, and OpenSSL are required. The fixture binds only to
host port `19000`; QEMU's user-network gateway is `10.0.2.2`, while TLS and
SigV4 use the certificate name `rustfs.test`. The runner removes the container
and its named data volume on exit. Override `CATTEN_RUSTFS_IMAGE` or
`CATTEN_RUSTFS_CLI_IMAGE` to test a pinned RustFS/`rc` image.

QEMU's named `cortex-a710` model does not expose Arm `RNDR`. The ordinary
AArch64 runner therefore attaches `virtio-rng-pci` to `/dev/urandom`; the
steady-state `rng` service discovers it and supplies host entropy through the
same capability-oriented device and protected-DMA model as other userspace
drivers. This is operational functionality, not an `s3_test` bypass: the RNG
device and service are present on normal QEMU boots too.

## Implemented scope

Implemented now:

- SigV4 path-style signing;
- GET and ranged GET;
- streaming PUT with known length and precomputed payload hash;
- HEAD and DELETE;
- fixed bucket/prefix policy;
- `Content-Length` and chunked GET responses;
- ETag, version ID, and request-ID metadata; and
- owned operation cleanup, caller binding, and inactivity leases;
- verified TLS 1.3 with SNI, hostname/chain/time validation, provisioned trust,
  and kernel-provided cryptographic randomness; and
- an end-to-end TLS RustFS Docker fixture.

Not yet implemented:

- external DNS and TLS 1.2;
- `ListObjectsV2`;
- multipart upload;
- temporary session credentials;
- parsed S3 XML error bodies; and
- automatic retry/backoff and connection pooling.
