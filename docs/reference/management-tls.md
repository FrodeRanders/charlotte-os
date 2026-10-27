# Management TLS and operator authentication

SEC-10 is being addressed as one migration of status and deployment ingress.
The current first component is `charlotte-management-tls`: a socket-independent,
`no_std` TLS 1.3 server engine. **It is not yet connected to either native
listener. Existing listeners remain plaintext and SEC-10 remains open.**

## Implemented protocol component

Rustls `0.23.46` implements the handshake, transcript, record sequence, peer
possession proof and TLS state machine. Its explicit custom provider uses the
same pinned Embassy SHA-256/HMAC/P-256 operations and erasing RustCrypto
AES-128-GCM family as the existing TLS client. The supported suite is
TLS_AES_128_GCM_SHA256 with P-256 ECDH and ECDSA/SHA-256 certificates and
handshake signatures. There is no RSA, alternate cipher, TLS 1.2, insecure
verification fallback, early data or session resumption. This provider adapter
does not introduce a new handshake or cryptographic primitive.

The native provider requires the embedding domain's existing Embassy drivers,
including its domain-owned entropy driver. An infallible crypto RNG callback
must abort on entropy failure; it cannot fabricate bytes or replay an accepted
request. Host fixtures register host entropy only in their test binary. Ring is
used by the independent host test client, **not by the native server**: its
SSE2 requirement conflicts with Charlotte's x86 service target. No SIMD feature
or context-save contract was changed to make it compile.

`Config::new` consumes a server certificate chain/private-key owner, client CA
roots and an explicit enrollment list. Each enrollment binds the SHA-256 of an
exact DER leaf certificate to `Reader` or `Administrator`. Duplicate entries,
including conflicting roles, reject. A CA-issued but unenrolled leaf rejects.
The additional pin does not bypass WebPKI's chain, validity, EKU or constraint
checks, or Rustls's proof of private-key possession. Typed P-256 key parsing
preserves curve/OID checks and the server certificate/key must match.

An authenticated `Identity` has private fields and is issued only after the
handshake completes. Its certificate digest supplies an audit identifier.
`Observe` is permitted to either role; `Mutate` requires `Administrator`.
Native HTTP dispatch must enforce that permission **before invoking its
handler**, and retain all existing signed mutation-body checks. The component's
role predicate does not itself protect an unwired handler.

The configuration is immutable. A replacement excludes revoked leaves from new
connections; callers must also retire old sessions/configuration factories.
Tickets, PSKs and early data cannot carry old authority into a reconnect.
Protected enrollment, rotation installation and rollback-resistant state remain
SEC-04/08 work. The UTC provider must return nonzero time at construction and
certificate verification, but its authenticity/freshness is still SEC-09.

## Ownership and reactor contract

`Session` owns erasing input/output buffers and returns application bytes in an
erasing owner only after authentication. Its original handshake and session
deadlines use caller-supplied kernel-monotonic milliseconds. Backward time,
arithmetic overflow, expiry, malformed input, capacity rejection and invalid
transmission receipts fence the session; later input cannot restart it.

`Event::Transmit` exposes the remaining encrypted prefix through `outgoing()`.
The containing transaction retains this session, its owned socket, pending RPC
and original deadline. It calls `transmitted(n)` only for a confirmed positive
prefix receipt. A partial receipt cannot cause a new encoding; unconfirmed
submission/observation failure cannot authorize replay. Processing resumes only
after earlier output has been fully acknowledged. TLS close-notify is available
as an encrypted terminal output, not a physical socket-cleanup proof.

`NeedInput` requires a real socket receive interest. `Progress` means bounded
local work remains; a reactor gives its other sources a turn. Neither event
justifies timer-driven receive polling. Socket cancellation, rejected CQ binding,
lifecycle observation and remote close belong to the containing service owner;
the protocol engine has no kernel capability, socket teardown or cleanup registry.

Erasure of the named local buffers, IV/nonce and selected cipher owners does not
qualify all Rustls/Embassy HKDF, HMAC, transcript, traffic-key or parser copies,
compiler spills, allocator history, forced termination or physical memory.
SEC-15 remains partial.

## Configuration and related dimensions

All component policy is collected in `Limits` in
`crates/charlotte-management-tls/src/policy.rs`. `Config::new` validates and
captures a complete value before creating sessions. These are constructor inputs,
**not yet a platform boot-resource or manifest interface**. Changing compiled
defaults requires a rebuild; existing configurations never resize in place.

| Field | Default | Unit / relation |
| --- | ---: | --- |
| `max_roots` | `8` | Client CA certificates |
| `max_enrollments` | `64` | Exact leaf/role entries |
| `max_chain_certificates` | `4` | Certificates per presented chain |
| `max_certificate_bytes` | `8192` | DER bytes per certificate or server key |
| `max_input_bytes` | `65536` | Retained incoming wire bytes per session |
| `max_output_bytes` | `65536` | Encoded output storage per session |
| `max_plaintext_bytes` | `16384` | Application bytes per engine output/write |
| `send_fragment_bytes` | `4096` | Rustls fragment setting, including its 5-byte header allowance |
| `handshake_ms` | `5000` | Original absolute handshake budget |
| `session_ms` | `60000` | Original absolute session lifetime, including handshake |
| `work_per_step` | `32` | Local TLS transitions per reactor step |

Protocol bounds are separate: TLS plaintext is at most `16384` bytes; output
must hold at least `16645` bytes and input conservatively covers Rustls's
`18437`-byte pre-negotiation allowance. Rustls also has an internal `65535`-byte
handshake-message ceiling. External buffers do not bound every internal allocation.
The fragment setting must be in `32..=16384`. Validation accounts for its
five-byte header deduction and 22 wire-overhead bytes per AES-GCM application
fragment, so reducing fragments can require increasing output storage.

Co-size roots, enrollments, chains, certificates, output flights, all concurrent
session buffers and service heap/stack admission. Larger chains/root hints can
still exceed a selected output-flight budget and reject; no prefix is accepted
as a complete handshake. Increase the session budget with request/response/close
budgets and revocation policy. These are observation/admission budgets, not a
bound on scheduler delay, allocator work or one cryptographic operation.

## Remaining migration gates

1. Supply one authenticated, complete startup credential/role/limit resource
   from the kernel's published trust context. Missing or invalid input must
   reject before listener publication; no independently chosen service trust.
2. Carry this engine in the existing owned, CQ-bound input/output/close phases
   for both status and deployment ingress. Authorize readers and administrators
   before dispatch; preserve signed mutation validation and multi-client progress.
3. Replace plaintext direct-node and VIP paths together, with standard verified
   host clients and explicit certificate-name checks. A proxy with an exposed
   plaintext backend does not meet this gate.
4. Qualify QEMU startup/restart, wrong/missing credentials, reader write rejection,
   peer substitution, replay, revocation, deadlines, partial sends, cancellation,
   rejected wait admission, slow clients and concurrent healthy traffic.

These gates deliberately remain substantial SEC-10 work. Component interop and
freestanding compilation do not establish native listener protection, protected
custody, authenticated time or production readiness. See the
[component evidence](../reports/audits/2026-10-27-security-management-tls-engine.md).

Primary API reference: [Rustls CryptoProvider](https://docs.rs/rustls/0.23.46/rustls/crypto/struct.CryptoProvider.html).
