# SEC-15: Kafka wire-profile encoding

Date: 2026-10-20. Reviewed baseline: `2c728b5`.

The Kafka profile encoder now returns its credential-bearing frame under
erasing ownership. Aggregate Debug formatting is removed from the wire profile
and its authentication enum. The [tracked evidence](evidence/2026-10-20-security-kafka-profile-encoding.json)
records controls, diagnostics, source/artifact hashes and execution results.
SEC-15 remains **partial**, with **29 corrected / 6 partial / 3 open** overall.

## Finding and correction

`charlotte_protocol_kafka::Profile::encode` returned a plain vector containing
SCRAM passwords and/or client private keys. The kernel's `launch_kafka_profile`
immediately wrapped that result in `Zeroizing`, so its ordinary release already
erased the buffer. The encoder API left that obligation with every caller.
Both `Profile` and `Authentication` derived Debug, allowing aggregate credential
field dumps. No actual production log of such a dump was identified here.

The encoder now returns `Result<Zeroizing<Vec<u8>>, ProfileError>`. It keeps
its existing full validation and checked complete-frame size before allocation.
The same single zero-filled allocation gets its owner before credential writes
and transfers directly to the caller. The kernel removes its redundant wrapper
and borrows the returned owner through its original synchronous launch-memory
preparation and release scope. There is no new output allocation, credential
section copy or plain-output compatibility encoder.

The profile and authentication enum lose aggregate Debug while retaining their
existing Clone/equality behavior. Authentication Copy and credential slices stay
borrowed. Profile Clone still allocates metadata vectors and borrows credential
fields. Fixtures compare profiles without automatically formatting credentials.
Individual fields and the encoded byte owner can still be explicitly copied or
formatted. Removing these two formatters does not prohibit all secret logging.

CHKAFP7 version 7, its 112-byte header, integrity digest, field ordering,
authentication choices, limits, error statuses and retired-format rejection
remain unchanged. The protocol crate adds one edge to the repository's existing
pinned zeroize 1.9.0 dependency; no third-party version changes. It still owns
no live capabilities. The manual's networking chapter and living references
now describe the returned owner. The PDF is left untouched.

The existing [C18 row](../../reference/cleanup-recovery.md) records the byte
owner. The kernel already had its erasing wrapper and retains its containing
launch context. Ordinary byte Drop adds no guards, waits, capability/root
cleanup, remote teardown, recovery custody or physical completion proof. The
reservation avoids growth without establishing aggregate heap admission or
fallible-allocation/OOM guarantees.

## Controls and host validation

The exact baseline protocol source is built in an isolated Cargo package.
Three controls fail: bare encoded password release, bare client-private-key
release, and password release after ordinary early-return abandonment. Each
observes one allocation, no growth and one release with the initialized public
fixture prefix un-erased. The original kernel wrapper pattern passes for both
password and private-key ranges. Another fixture confirms the baseline profile
and authentication Debug formatters include both credential fields. No baseline
guest run or existing production launch-buffer leak is claimed.

The observer reads eight initialized public fixture bytes with volatile reads
immediately before System deallocation. It reads no freed memory, padding or
uninitialized spare capacity. Profiles and reference decoding are prepared
outside the observation window: Kafka's metadata vectors have their own
allocations, which must not be mistaken for the output frame.

Seven new runtime tests cover thirteen variable-field ranges in independent
release runs, password abandonment, a typed same-allocation ownership transfer,
all four authentication choices, invalid profiles before allocation, exact
64 KiB admission/oversize rejection, and the independent full wire fixture.
Corrected release assertions observe the selected initialized range erased,
one frame allocation, no growth and one release. These checks do not inspect
every byte or prove physical erasure; growth counts alone establish neither
movement nor contents of retired storage.

A Python struct/hashlib generator supplies the complete **344-byte** public
frame, including its integrity digest, routes, broker and authority rows. The
[tracked fixture](../../../tests/transport/fixtures/kafka-profile-v7.bin) matches
both baseline and corrected encoders. Independent Debug-bound Cargo controls
compile against the baseline and reject each corrected type with `E0277`.
Two compile-fail doctests retain those trait policies. Isolated controls pin
zeroize 1.9.0 to match the repository.

Debug Kafka/protocol/S3/transport suites pass **128 runtime tests and 12
compile-fail doctests**. Optimized launch/Kafka/protocol/S3/transport suites
pass **190 runtime tests and 13 compile-fail doctests**. Strict Clippy, workspace
formatting and whitespace checks pass. An earlier passing suite preceded the
abandonment probe's change to observe the password range rather than username;
final suites identify the completed probe. The initial passing suite preceded
the seven new controls. Expected baseline and trait failures remain separate
from corrected passing runs. No incidental corrected build/test failure was
observed.

## Native validation and remaining scope

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes.

A fresh-storage three-broker Kafka run passes **20/0/0**, bitmap `0x503ffff`,
with TLS, required P-256 mTLS, SCRAM, route-leader interruption and a generic
transactional step. Kernel SHA-256:
`479408e3f81e053a41fd20da2b02241c9e7b5bca9521c55e3cfa37cd344f2580`.
The host kills kafka2 in the guest's declared fault window and later restarts
it. Two TLS transport-failure diagnostics occur during that window; the later
low-level transaction and generic step succeed. The final step reports four
commits/four produced, two retries, one DLQ result and one timeout. Those
counters are fixture outcomes, without a broader causal conclusion about
unrelated cleanup failures.

The pinned immutable broker reference was inspected locally before launch.
All three running container inspections capture image ID
`sha256:aee4e663253e674c1aa42ddc0485ff8c98ee6885997f303924085a59f8b46c7a`.
Project-scoped post-run container and volume inspections are empty.

This guest exercises the actual kernel encoder/launch caller and connector's
TLS/mTLS/SCRAM path. Malformed/maximum-size and byte-release observations come
from host fixtures. Native x86 Kafka, separate coordinator/fencing scenarios,
native mapping/submission failures and exhaustive broker behavior remain
unqualified.

Source credential buffers, explicit field/output copies, connector-local
profile data, TLS/library state, digest/scalar intermediates, original launch
memory and its kernel teardown, compiler spills/registers/cache and physical
memory remain separate SEC-15 boundaries. Ordinary Drop requires execution;
forced termination, panic-abort and power loss are outside these observations.
Protected enrollment, rollback-resistant state and recipient-key custody remain
missing; production trust stays disabled.

All **180 formal inputs** match preceding evidence: 34 modules, 36 safe and
109 negative configurations plus the runner. No fresh TLC cases were run;
these local byte owners and trait bounds are outside existing projections.
The manual PDF remains untouched, with its start/final hash in the evidence.
