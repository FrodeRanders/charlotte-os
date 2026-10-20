# SEC-15: SCRAM initial-phase owners

Date: 2026-10-20. Reviewed baseline: `f1f234a`.

The initial SCRAM phase now owns its nonce under erasing storage and escapes
usernames directly into its existing transcript. The
[tracked evidence](evidence/2026-10-20-security-scram-initial-owners.json)
records controls, diagnostics, source/artifact hashes and execution results.
SEC-15 remains **partial**, with **29 corrected / 6 partial / 3 open** overall.

## Finding and correction

`Client::new` previously created a plain escaped-username string, reserving only
the unescaped byte length. Commas and equals signs expanded that string and
could trigger growth. It copied the escaped name into an already-erasing
transcript, then released the scratch without erasure. The phase also retained
a plain nonce string; ordinary Drop, challenge rejection and successful challenge
processing released that copy without erasure. The password copy already had
an erasing owner.

The constructor now checks the escaped byte length and complete initial
transcript size before allocation. It reserves the transcript once and writes
`,` as `=2C` and `=` as `=3D` directly into it. All other UTF-8 bytes are
preserved. The private scratch formatter and its allocation are removed. The
nonce changes to `Zeroizing<String>`; password and nonce owners are installed
before copying their borrowed inputs. Successful construction now uses three
allocations: transcript, password and nonce. None grows during construction.

The same initial phase retains all three fields. Ordinary Drop or consuming
challenge processing releases them, including rejection, before the caller
uses the final phase. Password release and the final proof/verifier policy
are preserved. The public constructor and consuming phase API are unchanged.
Length overflow rejects as `InvalidCredential`; existing empty-input and nonce
syntax checks retain their outcomes. No new username normalization or admission
policy is added. Checked sizing does not establish aggregate heap admission,
fallible allocation or OOM recovery.

Usernames and nonces are SCRAM wire metadata. Their local lifetime protection
does not make them confidential on the wire or grant authentication/replay
rights. The actual connector's configured credentials and original generated
nonce retain their separate existing owners and lifetimes.

The existing [C18 row](../../reference/cleanup-recovery.md) records this local
phase. Its ordinary byte Drop adds no capability calls, guards, waits, remote
teardown, recovery custody or physical completion proof. No new registry,
owner family, persistent/wire format, dependency or cryptographic change is
introduced. The manual source and living references describe the phase's nonce
and direct transcript construction; the PDF is left untouched.

## Controls and host validation

The exact baseline Kafka core sources are built in an isolated Cargo package
with zeroize 1.9.0. Four release controls fail:

| Baseline control | Observation |
|---|---|
| Escaped username | Four constructor allocations; one scratch growth; one release with its initialized prefix un-erased |
| Nonce, initial Drop | Four allocations; nonce releases once, un-erased |
| Nonce, challenge rejection | Four allocations; nonce releases once, un-erased |
| Nonce, successful challenge transition | Nonce releases once, un-erased, while the returned final phase remains live |

The baseline password control passes, preserving its existing erasing policy.
Successful challenge processing has additional existing salt/proof/transcript
allocations; the three-allocation claim applies to initial construction only.

The shared host observer can now select an allocation by reviewed construction
order. Its existing `observe` selects ordinal one; SCRAM tests select the nonce
at ordinal four in the baseline and three after removing scratch. Password
moves from ordinal three to two. The observer clears a selected address when
that allocation releases, so later address reuse cannot become a new target.
All existing transport release suites run against this helper too.

The selected ranges contain eight initialized public fixture bytes, read with
volatile accesses immediately before host deallocation. No Rust object layout,
freed storage, padding or uninitialized spare capacity is inspected. Allocation
order and field offsets follow the reviewed fixture/source. General allocator
behavior remains outside these observations. Release counts alone do not prove physical erasure; growth
counts do not prove movement or contents of retired allocations.

Eight new tests cover transcript construction without scratch, nonce release
on Drop/rejection/success, existing password erasure, Unicode and adjacent/repeated
escapes, invalid initial inputs before allocation, and a complete independent
exchange. The successful-transition probe keeps the final owner outside its
observation window, confirming initial nonce release before final-phase use.
Corrected nonce assertions observe one erased release and no nonce growth.
Constructor checks observe three allocations and no transcript growth, including
a 2,048-byte escape-heavy username.

A Python hashlib/HMAC/Base64 reference supplies the
[tracked messages](../../../tests/transport/fixtures/scram-initial-reference.txt)
for an escaped username: client first, client proof and server verifier. Both
baseline and corrected implementations match all three and accept the verifier.
This extends the existing SCRAM vectors without changing the cryptography.

Debug Kafka/protocol/S3/transport suites pass **136 runtime tests and 12
compile-fail doctests**. Optimized launch/Kafka/protocol/S3/transport suites
pass **198 runtime tests and 13 compile-fail doctests**. Strict Clippy, workspace
formatting and whitespace checks pass. Earlier passing runs predate the eighth
reference test or the observer's address-retirement refinement; final source is
validated separately, and the baseline controls repeat with the final observer.
Expected baseline failures remain separate from passing corrected runs. No
incidental corrected build/test failure was observed.

## Native validation and remaining scope

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes.

A fresh-storage three-broker Kafka run passes **20/0/0**, bitmap `0x503ffff`,
with TLS, required P-256 mTLS, SCRAM, route-leader interruption and the generic
transactional step. Kernel SHA-256:
`21585fa884cd498d5fe61018919f6e5d25bd14fc150f9c372976bf5c802f2a24`.
The host kills kafka2 in the guest's declared fault window and later restarts
it. One TLS transport-failure diagnostic occurs during that window; the later
low-level transaction and generic step succeed. Final step counters include four
commits/four produced, two retries, one DLQ result and one timeout. These are
fixture outcomes, without a broader causal conclusion about unrelated failures.

The pinned immutable broker image was inspected locally before launch. All
three running container inspections capture image ID
`sha256:aee4e663253e674c1aa42ddc0485ff8c98ee6885997f303924085a59f8b46c7a`.
Project-scoped post-run container and volume inspections are empty.

The guest exercises actual connector authentication and reconnect paths with
its configured ASCII username. Escaped/Unicode usernames and initialized-byte
release observations come from host fixtures. Native x86 Kafka, separate
coordinator/fencing scenarios, malformed broker input, native mapping/submission
failures and exhaustive broker behavior remain unqualified.

Original inputs, configured credentials and caller nonce, explicit message
copies, connector-local profile data, third-party TLS state, compiler
spills/registers/cache and physical memory remain separate SEC-15 boundaries.
Ordinary Drop requires execution; forced termination, panic-abort and power loss
are outside these observations. Protected enrollment, rollback-resistant state
and recipient-key custody remain missing; production trust stays disabled.

All **180 formal inputs** match preceding evidence: 34 modules, 36 safe and
109 negative configurations plus the runner. No fresh TLC cases were run;
these local byte owners are outside existing projections. The manual PDF remains
untouched, with its start/final hash in the evidence.
