# Owned TLS client

S3 and Kafka use `catten_services::tls_client::OwnedTlsStream` for TLS 1.3 with
AES-128-GCM-SHA-256. The stream owns its socket and stable read/write record
buffers together. Explicit consuming close returns the socket's logical close
result; ordinary Drop destroys the connection before wiping the buffers.
Neither logical close nor byte erasure proves physical retirement.

## Pinned backend

The root lockfile selects Embassy-based embedded-tls at
`6487c62bb084c33361822022e21836ddfd48388f`, Embassy crypto/backend at
`822a76ea91f2c0885c6eb0c65fe91181e31133e1`, and transitive unitrait at
`bb96601b3d293fa27d1ca2891f6ea148e534e433`. Upstream still labels this TLS
source 0.19.0; the exact source revision distinguishes its API from the former
registry release. Updates require changing the reviewed revision and lockfile,
then repeating interop, ownership, advisory and target qualification.

TLS enables `rustpki,p256,p384,ed25519`. The RustCrypto backend registers only
the required SHA-256/384/512, HMAC-SHA-256/384, P-256 ECDH/ECDSA, P-384 ECDSA and
Ed25519 operations. Charlotte registers the sole RNG and AES-128-GCM drivers.
Do not enable the stock GCM driver or backend `all` feature. The source pins
select crypto implementations; they do not introduce an Embassy executor.

The sole crypto-bigint version is 0.7.10, with P-256/P-384 0.14.0 and
elliptic-curve 0.14.3. The [advisory gate](dependency-advisories.md) checks the
complete root graph, including independent consumers of SHA/signature APIs.

## Entropy authority and admission

Before publishing TLS/SCRAM consumers, startup calls
`install_domain_entropy(EntropyConfig)`. The configuration owns an optional
`Connection` and its `EntropyLimits`. S3 and Kafka transfer their
dedicated owned name-service lookup result into this domain context. Never
adopt a launch-owned connection or store a borrowed `ConnectionRef` globally.
`service: None` explicitly installs an architectural-only source. A zero
request budget returns the complete new configuration without installation.
Installation is one-shot; a duplicate returns the complete new configuration
without replacing the original source or limits, even while checked out. There
is no live replacement or independent capacity pool.

`fill_domain_entropy` briefly takes the connection out of the context into one
owning lease. It releases the metadata mutex before architectural randomness,
RPC submission/wait, returned-memory mapping and copy. Ordinary success,
rejection and unwind return that same connection through lease Drop. Other
checkouts reject immediately as busy; they do not wait for the producer or
spin on an outstanding RPC. The installed connection lives until domain
retirement; forced termination remains a kernel teardown boundary.

The source first uses the kernel RNDR/RDRAND interface where available, then
fills the remainder through its owned `rng` connection. Its pending call and
returned memory/mapping use the runtime owners. SCRAM uses the fallible
`fill_domain_entropy` API. TCP/IP's independent startup path retains
`fill_entropy` with its explicit local borrow and limits, without installing TLS state.

### Request deadline and operation owner

`EntropyLimits::request_timeout_ms` is a positive startup setting, default
`5000` milliseconds, aligned with the existing five-second service-observation
policy as a starting value, not a hardware latency guarantee. Hardware filling
and service admission/observation share
one monotonic deadline; no stage restarts its budget. `fill_service_entropy`
accepts an existing caller deadline and deliberately makes no architectural
shortcut. The service-only request must fit the protocol's existing
`1..=entropy::MAX_REQUEST` bound; no prefix is submitted on rejection.

`ServiceEntropyRead::start` owns the accepted pending call and borrows the
source/output with the original deadline. Consuming `finish` combines it with
returned owners and a read-only mapping in one completion owner. It requires an
exact byte count, memory and no connection attachment; negative peer status,
malformed replies and mapping failure never become successful entropy. A deferred
`start` does not arm background cancellation: retaining that owner can retain
accepted work/backing past its deadline until `finish`, `cancel` or Drop. A late
reply is disposed without publication; a deadline crossed during the final copy
clears that destination before rejection. Output is usable only on success;
an architectural prefix may already have been filled when the service fails.

`ExpiredBeforeAdmission`, `Submission`, `Observation` and `ObservationTimeout`
remain distinct. `Observation` preserves the exact `IpcError`, including wait
admission rejection; it is never converted to timeout. Accepted work is not
replayed. The pending owner survives rejection until checked cancellation, and
explicit `cancel(self)` returns the complete operation/borrows if close rejects.
Its Drop retains the runtime's domain-abort policy on unsafe cancellation.

This bounds reply observation, not a whole TLS/SCRAM exchange, source lookup,
kernel mapping/cancellation latency or scheduling progress. Cancellation of a
request does not prove RNG device DMA quiescence. The service still allocates
reply backing on its own account; caller funding and retention pressure remain
separate audit work for holders of RNG authority.

Embassy's RNG callback cannot return an error. Opening TLS preflights entropy
and returns `EntropyUnavailable` on ordinary admission/source failure. A later
callback failure is fail-stop; it neither retries an accepted RPC nor invents
bytes. The old three-attempt timer retry is removed. Host randomness is confined
to a `cfg(test)` source and cannot be installed by service builds.

One nonblocking handshake admission owns the verifier's domain-global UTC
clock through the complete handshake and clears it on return. Concurrent or
reentrant opens return `CryptoBusy` before changing that clock or publishing
handshake bytes. Neither busy nor entropy rejection is classified as a
transport failure by `OpenError`. Kafka preserves that distinction; S3 retains
its existing generic application-error mapping. Established streams keep
independent record/key state.

## Authentication and erasure

An open requires a nonempty expected hostname and DER trust anchor, nonzero
synchronized Unix time, valid socket bounds and a complete optional client
certificate/private-key pair. There is no insecure verifier, clock bypass or
plaintext fallback. The verifier authenticates a leaf signed directly by the
configured root, checks its hostname and dates, and verifies the handshake
signature. It does not validate general intermediate chains.

mTLS accepts a typed P-256 SEC1 private key. P-256's parser preserves curve OID
and optional public-key consistency checks; the upstream scalar-length-only
convenience parser is not used. Conversion to Embassy's signing key uses a
zeroizing scalar temporary. The owning prepared handshake retains that key and
the clock guard together. `TlsContext::with_client_cert` borrows the key only
through the consuming handshake. This is the actual builder API, correcting
the preliminary assessment's reference to a method on `TlsConfig`.

The custom GCM context owns the already qualified AES-GCM 0.10.3 cipher, with
zeroize-enabled AES 0.8.4, GHASH 0.5.1 and POLYVAL 0.6.2. Embassy's opaque
destructor forwards that context's Drop. A plain retained key array, as in the
pinned stock GCM driver, would not preserve this contract. Actual selected-TLS
cipher vectors and tamper controls run on the host and signed native probe.
The AES erasure bounds, authentication-state destructor guards and freestanding
x86 `polyval_force_soft` selection remain enforced.

Curve secret/signing/shared-secret owners have erasing destructors. TLS traffic
secrets, IVs, HKDF PRK, compiler copies and forced-termination erasure remain
separate SEC-15 work. A clean advisory scan and functional interop do not prove
constant-time execution or complete physical-memory erasure.

## Compile-time capacities

The related constants are grouped at the top of
[`tls_client.rs`](../../crates/catten-services/src/tls_client.rs):

| Setting | Default bytes | Consequences of adjustment |
| --- | --- | --- |
| `TLS_RECORD_BUFFER_LEN` | `16640` per buffer | Two heap allocations per live stream; TLS record framing overhead and upstream minimum record storage must fit. Recheck heap pressure, record-size cases and error cleanup. |
| `TLS_CERTIFICATE_LEN` | `16384` | The verifier's bounded copy must hold the complete peer Certificate message; increasing it also increases handshake stack usage. It does not add chain validation. |

The separate entropy request budget travels in startup's `EntropyConfig` and
can be supplied through that Rust API. Bundled S3/Kafka and TCP/IP currently
select the one `EntropyLimits::DEFAULT` in
[`entropy_client.rs`](../../crates/catten-services/src/tls_client/entropy_client.rs).
Changing that default rebuilds services; no launch-profile/config-page field,
boot-module parser or live update is added. A longer budget also retains the
source checkout, pending metadata, output borrow and possible reply/mapping
longer. Qualify failure timing, queue/memory pressure and cleanup together.

These are service build inputs, not kernel resource-profile fields or boot-time
knobs. Change them together with workload sizing, stack/heap grants and the
peer's record/certificate behavior, then rebuild both service variants and
qualify maximum-size and rejection cases. The driver's opaque context layout
is a pinned upstream structural contract; its generated target-size/alignment
checks must pass when changing driver types or features.

The [entropy deadline follow-up](../reports/audits/2026-10-26-security-entropy-deadlines.md)
records the new client ownership/pressure scope. The [migration report](../reports/audits/2026-10-26-security-embassy-tls.md)
records executed checks, artifact identities and retained failures. SEC-14/15
remain partial; production trust remains disabled.
