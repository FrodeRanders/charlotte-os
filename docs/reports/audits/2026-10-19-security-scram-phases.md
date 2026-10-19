# SEC-15 SCRAM phase ownership — 19 October 2026

## Result and scope

Baseline `caee4ac` commits owned SHA-state erasure and shared HMAC results. This
batch continues the SCRAM lifetime review. The former mutable `Client` kept its
password copy after proof generation. An oversized final response rejected
before taking its expected signature, leaving that verifier present and the
same exchange callable again.

Two host regression controls reproduce those retained fields on the committed
baseline. This is a library lifetime/ordering gap. The production connector
already returned from authentication on error and dropped its local client;
no production replay or authentication bypass was reproduced.

The replacement has two consuming phases. Challenge processing retires the
initial password/transcript owner. The final phase owns its proof and expected
server signature together, and consuming verification retires both on every
outcome. Compiler controls reject phase reuse. The actual connector passes the
TLS/mTLS/SCRAM broker leader-failure run.

SEC-15 remains partial. Configured credentials, caller/compiler copies,
third-party crypto state and forced termination retain their separate limits.
Finding counts remain **29 corrected in scope, six partial and three open**.
The [tracked evidence](evidence/2026-10-19-security-scram-phases.json) contains
source/artifact hashes, diagnostics, model fingerprints and validation results.

## Owning phases

| Phase | Owned material | Consuming boundary |
|---|---|---|
| `Client` | Exchange-local zeroizing password and initial transcript; nonce | `receive_server_first(self, ...) -> Result<ClientFinal, Error>` drops the initial owner on success or rejection. Success transfers only the proof and expected signature. |
| `ClientFinal` | One zeroizing proof message and expected server signature | `message()` borrows proof bytes for encoding. `receive_server_final(self, ...) -> Result<(), Error>` consumes the phase on success, invalid length/UTF-8, server rejection, malformed signature or mismatch. Ordinary cancellation drops the same owner. |

The initial phase cannot verify a server final, and the final phase has no
password field or challenge-processing method. Neither is cloneable. The
connector encodes the borrowed proof into its existing zeroizing request buffer
and retains the final owner through the reply. Transport/framing/broker errors
leave the function with that owner still in scope, so ordinary Drop releases it.
Reconnect creates a fresh exchange with a fresh nonce; it cannot reuse either
consumed phase.

The long-lived profile credential and the caller's existing authentication copy
remain available for future connections. The lifetime improvement applies to
the additional copy inside this exchange. Existing local zeroizing field
destructors perform erasure/deallocation in ordinary userspace authentication
context. No capability syscall, physical-release proof, recovery registry or
new retry authority is added. C18 records this extension in
[cleanup/recovery](../../reference/cleanup-recovery.md); the
[secret-state inventory](../../reference/secret-state-lifetimes.md) records its
remaining boundaries.

## Reproduction and validation

The baseline controls use a public fixture password and a valid bounded
challenge. Both fail with explicit diagnostics:

```text
proof_generation_releases_password_owner: password retained after proof generation
rejected_final_retires_verifier: rejected final retained a reusable verifier
```

They inspect live private fields rather than freed memory, and print no secret
bytes. The new API removes those mutable states. Its runtime tests keep the
RFC 7677 proof/server-signature bytes unchanged and cover malformed/oversized
challenges, wrong nonce, excessive work, empty/oversized/invalid-UTF-8 finals,
server errors, malformed signatures, extension suffixes and mismatches.

Both debug and optimized Kafka suites pass **16 runtime tests and two
compile-fail doctests**. Two independent Cargo-managed fixtures also require
`E0382` for initial-phase and final-phase reuse. The second attempts reuse after
a rejected final. This establishes consumption at the API boundary; it is not
a new memory-forensics measurement. Warning-denied Clippy across the package's
targets and workspace formatting pass. AArch64 and x86 bundles build/sign; ARM
final-source/signature freshness passes.

The native AArch64 fixture uses three Apache Kafka 4.1.1 brokers, verified TLS,
required P-256 mTLS and SCRAM-SHA-256. It runs in its own Compose project,
`charlotte-sec15-scram`; no existing containers were found before startup.
After authentication, the host kills output-route leader `kafka2` in the
required fault window. The suite completes **20 passed / zero failed / zero
pending**, bitmap `0x503ffff`. Its diagnostic records:

```text
low-level offset 1, generic step commits=4 produced=4 retries=2 dlq=1
timeouts=1 group_generation=3 heartbeats=3 metadata_refreshes=2
coordinator_refreshes=0 retry_attempts=8 terminal_errors=2
output_route_produced=5 lag=1
```

A `TLS handshake transport failed (6)` diagnostic occurs during the injected
failure window. It is retained with the passing fault result, without treating
it as a separate successful authentication or assigning a stronger causal
conclusion. The run exercises leader recovery and generic transactional steps;
coordinator-movement and producer-fencing scenarios were not rerun. The test
project's containers and volumes are confirmed absent after cleanup.

Extra direct-`rustc` checks initially failed because the host test library and
project compiler differed, then because standalone commands could not resolve
Cargo's split metadata/dependencies. Those setup failures are preserved
separately and supply no consumption evidence. Cargo-managed repeats use a
consistent dependency graph and reject specifically with `E0382`. Initial
sandboxed Docker inspection was likewise rejected before daemon access; the
permitted inspection and fixture then succeeded.

## Documentation, models and limits

Markdown contracts and the LaTeX manual now describe the consuming phases.
The manual builds in temporary output and retains 234 pages. Changed pages
and the affected contents page receive visual review. Pre-existing chapter
overflow diagnostics are unchanged apart from source-line offsets; the new
paragraph adds none. The tracked user PDF is untouched by this batch.

All **180 model/config/harness fingerprints** match the preceding evidence.
The existing models do not project SCRAM byte storage or these Rust types, so
no transition or proof claim changes and no fresh TLC run is claimed. Inventory
remains **34 modules, 36 safe configurations and 109 expected counterexamples**.

Neither phase guarantees that destructors run on panic-abort, forced domain
termination or power loss. Base64/parser scratch, identity metadata, the
configured/caller credentials, scalar/compiler copies and TLS/HPKE/library
state remain in the wider review. This batch changes no Kafka/SCRAM wire
grammar, cryptographic algorithm, transport admission, persistent format,
production trust or custody boundary.
