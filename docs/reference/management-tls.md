# Management TLS and operator authentication

Status (`443`) and deployment ingress (`7444`) now use the same mandatory
mutual TLS 1.3 engine and boot credential resource. There is no plaintext backend
or fallback. Missing credentials disable network admission while preserving
cooperative shutdown. SEC-10 is partial pending the broader execution qualifications
listed below; public development trust is not production enrollment.

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
Native HTTP dispatch enforces that permission **before invoking its handler**,
and retains all existing signed mutation-body checks. Status requires `Observe`;
deployment status/ingress-policy reads require `Observe`; every mutation requires
`Mutate`. A reader's incomplete POST is rejected without awaiting its body or
starting a controller RPC. Logs bind the full certificate digest, role, permission
and decision to a nonreused local connection ID; request bodies and keys are not
logged. Local controller capability authorization remains a separate boundary.

The configuration is immutable. A replacement excludes revoked leaves from new
connections; callers must also retire old sessions/configuration factories.
Tickets, PSKs and early data cannot carry old authority into a reconnect.
Protected enrollment, rotation installation and rollback-resistant state remain
SEC-04 work; cluster peer authentication remains SEC-08. `PreparedConfig` checks
credentials without inventing UTC; only `activate()` with nonzero real time
produces the configuration accepted by `Session`. Each accepted native client
obtains a fresh positive time-service reply under its original handshake budget.
Certificate verification uses that UTC projected through the kernel monotonic
clock. Zero/unavailable time, rollback and arithmetic overflow reject. This is
not authenticated or rollback-resistant time: the source still uses SNTP/holdover
and SEC-09 remains open.

## Complete boot resource and enrollment

`charlotte-launch::management` defines one current profile, `CMGMTL01`, with an
exact 128-byte header, all eleven limits, explicit counts, ordered server chain,
client roots, exact leaf/role records and the private server DER key. Counts and
lengths are checked before use; missing fields, unknown roles, duplicates,
reserved bytes, retired tags, every truncation and extensions reject. No aliases
or per-field manifest defaults fill missing policy.

The `CMGTENC1` envelope has a 176-byte header and HPKE ciphertext. It uses the
existing RFC 9180 X25519/HKDF-SHA256/ChaCha20-Poly1305 suite with a distinct
management context, and an Ed25519 operations signature. The context binds the
**entire canonical selected admission policy**, including cluster, sequence and
all four keys. Signature/context verification precedes private work. Opening
checks the zeroizing recipient owner's public binding, decrypts into an existing
erasing owner, then checks the complete profile. It is not an operations-bundle
variant or an exception to that bundle's expiry checks.

The kernel build accepts only an absolute regular-file
`CATTEN_MANAGEMENT_TLS_RESOURCE` input and embeds ciphertext. Boot opens it using
the same `PreparedBootTrust` before trust publication or management service
publication. The prepared plaintext lives through launch of both services, each
receiving its own read-only launch memory through the existing owning launch
transaction. No service receives the operations signer or HPKE recipient key.
Invalid envelope/policy input rejects steady-state launch; invalid DER/config
prevents the affected service from opening its listener. An absent resource
leaves both network interfaces disabled, with no plaintext substitute.

Host commands use files, never inline secret key arguments:

```sh
cluster-sign management-limits-default limits.txt
# Review every limit; DER inputs must be P-256 ECDSA identities.
cluster-sign management-profile-create private-profile.bin limits.txt server-key.der \
  chain=server.der root=client-ca.der reader=reader.der administrator=operator.der
cluster-sign management-resource-seal management.bin policy.bin operations-key-file private-profile.bin
cluster-sign management-resource-verify management.bin policy.bin
```

The private profile is created exclusively with mode `0600`. Keep it and the
server key in restricted storage and remove disposable copies explicitly.
Verification checks signature/context against the supplied public policy; it
does not install policy or prove custody. `management-policy-development` exports
only the repository's public development fixture; it cannot enroll production.

For replacement/revocation, advance the installed policy revision, regenerate
and seal the complete profile, rebuild the kernel with that ciphertext and
restart both services. Existing configurations/sessions never resize or acquire
new roles. The current resource transport is a build input delivered as a boot
resource, not a live control-plane update. Policy binding does not select a
unique resource among competing signatures at the same revision, nor supply
protected installation, rollback-resistant storage or recipient custody.
Production images remain disabled under SEC-04.

Verified `cluster-sign` management commands require PEM files in
`CATTEN_MANAGEMENT_CA_FILE`, `CATTEN_MANAGEMENT_CERT_FILE` and
`CATTEN_MANAGEMENT_KEY_FILE`. Numeric endpoint addresses are certificate-checked
as IP identities; optional `CATTEN_MANAGEMENT_SERVER_NAME` selects a DNS/IP
certificate identity and SNI. It does not disable verification. No application
bytes are submitted before TLS completes. Configuration/verification rejection
is terminal; accepted HTTP uncertainty never authorizes replay. One original
deadline covers connect, handshake and every socket I/O, including internal
Rustls reads/writes.

For browser access, distribute the server CA through the organization's trusted
channel and issue a P-256 client certificate with `clientAuth` EKU. Enroll that
exact DER leaf in the selected role. A browser can import a password-protected
PKCS#12 containing its client identity; for example,
`openssl pkcs12 -export -inkey operator.key -in operator.pem -certfile client-ca.pem -out operator.p12`
prompts for its export password rather than putting it in argv. Import the
identity into the browser/OS client-certificate store and open an HTTPS name
covered by the server certificate's SAN. A client CA and server CA may differ:
the profile's `root=` inputs authenticate clients, while the host/browser CA
authenticates the server. Installing a browser identity does not enroll its
leaf or assign its role. There is no password, bearer-key or JavaScript bypass.

Explicit runner probes generate fresh short-lived fixture credentials in a
mode-`0700` directory and use the public development authorities. A supplied
resource is never replaced by the runner; corresponding client file settings
are required. Reusing a built kernel requires its exact existing resource and
clients. Fixture directories retain client keys for repeatable probes and must
be removed when no longer needed; they are not published audit evidence.

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

`catten_services::management_io::Connection` is the containing owner. Its pending
RPC precedes session/socket fields, so confirmed observation/cancellation ends
before local TLS state or raw socket release. Receives own returned memory and
mappings; output retains encrypted prefixes until positive receipts. A final
plaintext receipt also confirms encrypted close-notify output. Consuming logical
close moves the original socket into the existing close pool, without claiming
physical cleanup. Rejected CQ binding aborts with accepted work retained.
Queue-full retry applies only before admission, under the original deadline;
accepted calls are never retried. The sequential keyhole waits on that CQ and
lifecycle, while deployment rotates its existing input/control/output/close
phases so a slow TLS peer cannot monopolize a blocking handshake.

Erasure of the named local buffers, IV/nonce and selected cipher owners does not
qualify all Rustls/Embassy HKDF, HMAC, transcript, traffic-key or parser copies,
compiler spills, allocator history, forced termination or physical memory.
SEC-15 remains partial.

## Configuration and related dimensions

All TLS workload policy is collected in `Limits` in
`crates/charlotte-launch/src/management/limits.rs`, reexported by the engine.
The complete resource carries these boot settings; `management-limits-default`
writes every named setting without grouping separators. Missing/unknown/duplicate
settings and invalid relationships reject before private input or output
publication. Existing configurations never resize in place.

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
budgets and revocation policy. `MAX_PROFILE_BYTES = 65536` and
`USER_STACK_PAGES = 32` (131072 reserved bytes, committed on demand) are central
build-time knobs in `charlotte-launch::management`. Both management services use
that same stack setting, including startup without a credential profile. The
entry frame exists before credential checks; selecting the general four-page
stack on the missing-profile branch faulted before deployment could enter its
disabled state. Native P-256 verification also exceeded that general stack.
Requalify both architectures when changing the provider or stack.
The host PEM reader bound is twice the profile ceiling; its certificate/count
limits currently use compiled `Limits::DEFAULT`, so scale those with a larger
profile. Existing HTTP framing, concurrency, aggregate bytes and close deadlines
remain their own documented service dimensions. These are observation/admission budgets, not a
bound on scheduler delay, allocator work or one cryptographic operation.

## Execution qualification and remaining gates

The [native report](../reports/audits/2026-10-27-security-management-tls-native.md)
records direct-node/VIP identities, role dispatch, deadlines and healthy progress.
The [startup follow-up](../reports/audits/2026-10-27-security-management-startup.md)
records missing-resource and malformed-DER startup, envelope policy/signature/AEAD
rejection, the missing-profile stack correction and host acceptance-gate controls.
The [rotation follow-up](../reports/audits/2026-10-27-security-management-rotation.md)
qualifies x86 rebuild/restart from compiled installed revision 7 to 8. It retains
both client CA roots while replacing exact leaf enrollments, and records the
remaining ability to replay an older still-authorized kernel separately.
Historical reports retain their own unqualified scopes and failed runs.

Run the native startup campaign with:

```sh
python3 scripts/run-management-startup-tests.py --architectures x86_64 aarch64
```

It builds each signed service bundle once and rebuilds each case's exact kernel
input before booting fresh, isolated storage. `--cases`, `--status-port`,
`--deployment-port`, `--timeout` and `--build-timeout` select the test scope and
observation budgets. `--reuse-bundle` requires an already built and signed
current-source bundle; the report identifies the exact artifacts actually used.
One temporary directory owns private fixture keys and profiles through the
campaign. Published evidence must retain concise observations and hashes, not
links to its temporary captures.

Each runner also accepts `--management-startup-test EXPECTATION` for an isolated
timed user-network boot: `ready`, `validity`, `disabled`, `WrongPolicy`, `Signature` or
`Authentication`. This is a host assertion, never a kernel credential or policy
override, and it does not automatically create a missing resource. The terminal
rejection modes require `--no-failure-capture` and make no self-test success claim.
Missing or malformed-DER credentials must produce both explicit disabled-state
events after exactly one policy publication. Envelope failures must reach the
exact terminal boot gate before policy/service publication. The public revision
publication marker is emitted after the policy guard leaves.

The `validity` assertion adds four deliberately enrolled client leaves: expired
and future-dated readers and administrators. Run just that qualification with
`python3 scripts/run-management-startup-tests.py --cases validity`. The disposable
fixture signs explicit date windows around current host UTC and retains their
public hashes/windows in the campaign evidence; it does not override guest time.
Its complete resource includes all six enrollments. OpenSSL verifies each control's
signature, chain and client purpose with time disabled, then requires the exact
current-time expiry/future failure before any guest boots. The ordinary CA key is
removed after issuing the controls. The host-only
`CATTEN_MANAGEMENT_VALIDITY_DIR` selects that private probe fixture, not kernel
trust or service configuration. The startup campaign supplies it automatically.

SLIRP's host listener can accept while the guest supplies no terminal TCP result.
Closed-port EOF/reset/refusal and bounded transport timeouts are recorded
separately. A timeout is not rejection, physical closure or quiescence proof;
disabled-state acceptance comes from the explicit native startup events. The
pre-publication cases use the exact terminal boot rejection, before network
service composition, rather than host-forwarder silence.

Host acceptance rejects a failed identity or liveness control even when later
metrics responses are healthy. Management Python probes reject optimized Python
execution so their runtime checks cannot silently disappear.

Run `scripts/run-management-rotation-test.py` with explicit `--firmware-code`
and `--vars-template` paths after building the x86 service bundle. It reuses the
Secure Boot firmware-authority preparation, owns disposable credentials and
guest processes, and keeps one firmware-variable image and one data image across
the revision transition. The test-only `CATTEN_BOOT_TEST_INSTALLED_POLICY`
selects an absolute complete public signed snapshot compiled into an x86
`boot_trust_test` kernel, independently of the candidate boot module. The default
remains revision 7. Boot authenticates the compiled snapshot and requires an
exact module match; production and other build contexts reject this input.

Native rejection auditing exposes the bounded `client policy rejected` category
without constructing an authenticated identity or logging raw certificate/error
contents. This category includes enrollment and the verifier's time/credential/
capacity gates; it is not universally proof of revocation. The rotation test also
verifies both old client chains against the combined trust store, confirms current
replacement clients, and requires that native event before accepting an old
client's transport failure as a policy rejection. Transport silence alone fails.

`Error::client_rejection()` returns one bounded `ClientRejection` category for
policy, expiry or future validity. The native adapter logs only the existing
connection ID and its static message, without authenticating or naming a rejected
peer. Validity probes require a new exact expiry/future event from a distinct
native service/connection episode, no application byte (including partial HTTP), and a verified healthy
reader after each rejection. UART delivery and transport close share the same
original probe deadline; a timeout or server-verification failure cannot satisfy
the assertion. The verified exchange helper is shared with the rotation campaign;
both campaigns' negative exchanges reject partial application data immediately.

The [validity follow-up](../reports/audits/2026-10-27-security-management-validity.md)
qualifies sixteen expired/future enrolled-client controls across both interfaces
on x86 and ARM, each followed by a healthy reader. The current time service is
still unauthenticated; this qualifies date enforcement at a fresh handshake,
not a trusted security-time boundary or expiry during an established session.

The [leader-loss follow-up](../reports/audits/2026-10-27-security-management-failover.md)
corrects election jitter to scale with the configured election timeout. Three
native ARM voters replace the killed leader from term 1 to term 2; a signed
mutation through a surviving follower advances policy generation/sequence 1 to
2, read back on both survivors. Both the full VIP campaign and a direct-node
repeat nevertheless fail on later TLS handshakes. The direct repeat completes
only two client-rejection/recovery controls before a healthy reader times out
at the other survivor. Neither timeout proves authentication rejection. Complete
management failover availability and the rejection matrix remain unqualified.

Remaining native campaigns include established-session/issuer/server expiry,
ARM installed-resource rotation, resolving those multi-node transport failures,
new adapter fault injection and broad pressure.
Protected installation/rollback and live factory/session retirement remain
separate boundaries. Shared host controls for peer substitution, replay, partial
sends and cancellation do not replace those execution qualifications.

SEC-10 remains partial while these qualifications remain incomplete. Protected
custody, authenticated time, cluster peer authentication and full secret erasure
remain separate gaps. Preserve the historical
[component evidence](../reports/audits/2026-10-27-security-management-tls-engine.md).

Primary API reference: [Rustls CryptoProvider](https://docs.rs/rustls/0.23.46/rustls/crypto/struct.CryptoProvider.html).
