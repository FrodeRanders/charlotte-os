# SEC-14/15: pinned Embassy TLS migration

Date: 2026-10-26. Reviewed baseline: `9cbd5474`.

The active workspace now uses the pinned Embassy-based embedded-tls main branch
and modern RustCrypto curve family. The complete root graph contains one
crypto-bigint, **0.7.10**, and ctutils **0.4.6**. Its fresh advisory gate passes
without ignores. S3 and Kafka use a domain-owned entropy context, typed P-256
mTLS preparation and an erasing AES-GCM driver behind the new interface.

This implements the [migration assessment](2026-10-26-crypto-bigint-embassy-migration.md).
The [tracked evidence](evidence/2026-10-26-security-embassy-tls.json) preserves
source/lockfile/artifact hashes, scanner output, host results, native terminal
oracles and failure diagnostics. The numbered ledger remains **29 corrected /
6 partial / 3 open**. SEC-14 and SEC-15 remain partial; production trust remains
disabled. Advisory removal does not establish platform-wide constant-time
execution or complete secret-state erasure.

## Dependency and API replacement

The TLS revision is `6487c62bb084c33361822022e21836ddfd48388f`; its Embassy
crypto/backend revision is `822a76ea91f2c0885c6eb0c65fe91181e31133e1`, with
transitive unitrait `bb96601b3d293fa27d1ca2891f6ea148e534e433`. These are full
immutable revisions, not floating branch selectors. The source still declares
TLS version 0.19.0 despite changing the API. P-256/P-384 are 0.14.0,
elliptic-curve is 0.14.3 and ECDSA is 0.17.0.

Only required hash, HMAC, curve and Ed25519 backend operations are selected.
Charlotte registers its RNG and AES-128-GCM drivers; the stock GCM driver and
backend `all` feature are disabled. No Embassy executor, TLS fork, arithmetic
backport or old-provider compatibility path is introduced. The former direct
rand_core 0.6/signature dependencies and per-handshake borrowed entropy
parameters/fields are removed. Independent current signature/SHA consumers
remain in the complete graph; no claim of eliminating every duplicate API
family is made.

The complete **182-package** root lockfile scan uses cargo-audit 0.22.2 and
RustSec commit `a78282a1414d6c28cc228c4eae3570451affb0dc` (1378 advisories).
All architectures, severities and informational categories are enabled, with
fresh database/registry access and yanked checks. There are zero vulnerabilities
and warnings. The production wrapper also validates the JSON report, including
notice handling. Earlier failing scans and the isolated candidate remain
historical evidence; this pass applies to the active root lockfile.

## Entropy, verification and cleanup

S3 and Kafka transfer their dedicated owned entropy lookup result into one
domain-lifetime context before publishing consumers. A duplicate install returns
its new owner; it cannot replace an installed or checked-out source. Explicit
`None` selects architectural randomness only. Launch-owned capabilities remain
borrowed through `Context`; no borrowed connection or owning scalar is stored
globally.

A checkout moves the connection into a private owning lease. The metadata
mutex leaves before hardware randomness or RPC submission/wait, returned-memory
mapping and copy. Other checkouts reject as busy; ordinary success/rejection or
unwind returns that same owner. The pending call and reply/mapping retain their
existing runtime owners. Unit rejection-stage controls use a synthetic owner,
not a claim of separately injecting every kernel submission/mapping failure.
The native AArch64 S3/Kafka runs exercise the real service path: their named
Cortex CPU lacks RNDR and the runner supplies the protected-DMA VirtIO RNG.

SCRAM retains a fallible nonce-fill API using the installed source. TCP/IP's
independent borrowed-source startup helper remains separate. TLS preflights
entropy; subsequent failure inside upstream's infallible RNG callback panics
into the service entry macro's domain-abort policy. It does not replay accepted
entropy RPCs or fabricate bytes. The old three-attempt timer retry is removed.
Host entropy is a `cfg(test)` source and is absent from service builds.

One nonblocking admission owns and clears the verifier's static UTC clock
through a complete handshake. Overlap rejects with `CryptoBusy` before changing
the clock or emitting handshake bytes. Busy/entropy/authentication rejection is
not a transport failure at the TLS `OpenError` boundary. Kafka preserves this
distinction; S3 keeps its existing generic application-error mapping. Existing
transport diagnostic codes 1/6 retain their
classification; the new upstream missing-PSK/server-name errors remain
nontransport failures.

The prepared handshake owns its clock guard and optional erasing mTLS key
together. Typed P-256 SEC1 parsing rejects malformed/wrong-OID keys before
admission; a zeroizing scalar temporary supplies the Embassy signing key.
`TlsContext::with_client_cert` borrows it only through consuming handshake
completion. This is the actual builder, correcting the assessment's preliminary
reference to a method on `TlsConfig`. Nonzero synchronized time, explicit root
and expected hostname remain required; no insecure verifier or plaintext
fallback is added. Directly root-signed leaf verification remains the chain
restriction.

`OwnedTlsStream` retains the same socket and stable record-buffer owners,
connection-before-buffer destruction, explicit close and ordinary error Drop.
The custom GCM context owns the qualified AES-GCM 0.10.3 cipher with erasing
AES/GHASH/POLYVAL features. The pinned stock context's plain retained key array
would not preserve that contract. Existing AES erasure bounds and target
authentication-state guards remain, including freestanding x86's software
POLYVAL selection. The actual selected TLS cipher is checked in both host and
signed guest controls. Typed key/shared-secret destructors do not clear all
TLS traffic-key/IV/HKDF fields, compiler copies or forced-termination memory.

The C18 [cleanup row](../../reference/cleanup-recovery.md),
[TLS reference](../../reference/tls-client.md), secret-state inventory and API
ledger record these contracts. No new custody family, physical completion
proof, root/capability admission policy, wire format or request replay authority
is introduced. The Configuration guide/manual document the existing grouped
record/certificate constants and their stack/heap consequences; no capacity is
raised or new live resizing installed.

## Host controls and failures

Both debug and release pass **476 distinct runtime tests plus one doctest per
mode**: 242 service-library tests, three reconnecting-client example tests,
64 shared-launch tests, 74 transport-erasure controls and 93 signer tests.
The 15 added library tests cover driver operations, typed keys, source ownership,
concurrent/reentrant admission and real OpenSSL interop. Repeated preliminary
passes are not counted again.

OpenSSL **4.0.3** host fixtures generate fresh public test roots/keys. They
exercise the production prepared handshake, verifier, identity parser and
registered crypto drivers over local TCP, with only the entropy source replaced
by its test-only owner. Server authentication and required P-256 mTLS complete
an HTTP round trip. Wrong hostname/root, expired/not-yet-valid dates, missing
or untrusted client identity, tampered handshake bytes, truncated records and
write rejection fail. Missing time, malformed/incomplete identity, overlapping
handshake and unavailable entropy reject before handshake. An unavailable
infallible RNG callback panics without replacing the fixture destination.

All three backend guard controls pass: the deployed erasing software owner
compiles; removing its target flag or erasure features rejects. Strict service
Clippy passes on both freestanding targets. Workspace formatting passes.
Eight advisory-policy controls also pass; the default harness skips its four
opt-in scanner/database fixtures. The production full-root scan runs separately
and passes. The complete cross-workspace host-driver script is not rerun.

Preliminary construction failures are retained separately:

- A fixture tried mutating configuration while a prepared handshake borrowed
  it; Rust rejected E0506. A distinct competing configuration fixes the test.
- rustfmt split a long escaped certificate-extension literal, producing an
  invalid boolean string and poisoning the fixture's serial mutex. The fixture
  now uses `concat!`, and mutex recovery permits independent failures to report.
- Initial Unix-socket runs failed readiness in the sandbox. A permitted run
  exposed macOS's `SUN_LEN` pathname limit; a shorter-path run then reported
  OpenSSL `ai_family not supported` and connection refusal. These are fixture
  observations, not a TLS authentication regression or a proven cause for every
  preceding startup failure. Local TCP with exact producer readiness resolves
  this setup. Its first draft needed a no_std formatting correction (E0599).
- The x86 bundle builder rejected an unsupported `--embed` argument before
  building; its normal invocation stages/signs automatically.

No rejected fixture expectation is weakened to obtain a pass.

## Final native artifacts and execution

Both architecture bundles and the separate Sitas image are rebuilt and signed.
Final-source rebuilds change all forty AArch64 service hashes and the x86
bundle hashes, so earlier passing guests are retained as preliminary runs and
the final artifacts are exercised separately. Every final native manifest's
complete bundle matches the current frozen ELF hashes, including Sitas.

| Final rebuilt bundle / fixture | Deferred passed / failed / pending | HTTP checks |
| --- | --- | --- |
| AArch64 SMMUv3, TLS RustFS S3 + security | 22 / 0 / 0 | 14 |
| AArch64 SMMUv3, Kafka TLS/mTLS/SCRAM fault/recovery | 20 / 0 / 0 | 0 |
| x86_64 AMD-Vi, security | 18 / 0 / 0 | 14 |
| x86_64 Intel VT-d, TLS RustFS S3 + security | 20 / 0 / 0 | 14 |

All four final runners return zero with diagnostic `reason: complete`. S3
performs its 33-byte PUT/HEAD/GET/DELETE round trip on both architectures. The
Kafka oracle reports four committed/produced generic steps, two retries,
one DLQ result, one deliberate timeout, group generation 3, three heartbeats,
two metadata refreshes, nine retry attempts, two terminal errors and output
route production of five records. Its runner kills/restarts the selected route
leader; recovery completes with required mTLS and SCRAM still enabled. A TLS
I/O diagnostic in that controlled fault window is preserved, not hidden or
reclassified as an authentication retry.

The three final HTTP runs retain **15 transport retry diagnostics**: each has
one initial-metrics `TimeoutError` and four `ConnectionResetError` retries
around EOF, abortive and idle-client controls. All affected probes pass within
their existing budgets. The four preliminary native runs also pass and retain
their separate fourteen HTTP retry diagnostics. No deadline, admission limit,
attempt budget or test expectation is widened. These results do not establish
causes for older intermittent scheduler/IPC/root-close/NIC failures.

The final manual builds in two passes to **252 pages**, with 189 global overfull
warnings and no undefined references. One new Configuration overflow was fixed
with a descriptive source link. Affected/adjacent pages 14, 117-119, 198-200 and
205-206 are rendered and visually checked; the rest of the manual is not
re-audited. Source/input and ELF hashes, local links and whitespace are checked.
No formal-model, production enrollment, license/provenance or physical-platform
side-channel qualification is claimed. The dependency advisories are addressed;
the broader audit acceptance gates retain their existing dispositions.
