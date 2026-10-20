# SEC-15: S3 wire-profile encoding

Date: 2026-10-20. Reviewed baseline: `50c5abf`, with the preceding
[service-local profile work](2026-10-20-security-s3-profile-owners.md) retained
in the working tree. The [tracked evidence](evidence/2026-10-20-security-s3-profile-encoding.json)
records diagnostics, controls, source/artifact hashes and execution results.
SEC-15 remains **partial**; overall counts remain
**29 corrected / 6 partial / 3 open**.

## Finding and scope

`charlotte_protocol_s3::Profile::encode` returned a plain vector containing
credentials. The kernel's `launch_s3_profile` immediately wrapped that result
in `Zeroizing`, so its ordinary release already erased the buffer. The helper
API left that obligation with each caller. The borrowed `Profile` also derived
Debug, permitting aggregate credential-field dumps; no actual production log
of such a dump was identified in this review.

The encoder now returns `Option<Zeroizing<Vec<u8>>>`. Complete validation and
size calculation still precede allocation. It allocates the same single
zero-filled frame, installs the owner before credential writes, and returns
that owner directly. The kernel caller removes its redundant wrapper and
borrows the owner through the same synchronous launch-memory preparation.
There is no plain-output compatibility encoder or new allocation/copy at
that boundary.

The wire `Profile` drops its aggregate Debug derive. Borrowed Copy/Clone and
equality remain; they duplicate references without owning credential storage.
Fixtures now compare profiles without an automatic credential formatter. The
encoded owner and individual fields can still be explicitly formatted or
copied. This change does not prohibit all secret logging or qualify every
provisioning caller.

CHS3PF1 header/field bytes, size bounds, validation, kernel admission and the
read-only launch-memory contract are unchanged. The protocol crate adds a
dependency edge to the repository's existing pinned zeroize 1.9.0; no third-party
version changes. It still owns no live capabilities. The manual's storage chapter now distinguishes the encoded/service-local owners from
the original read-only launch memory. Its PDF is left untouched.

The existing [C18 row](../../reference/cleanup-recovery.md) records this byte
owner. The kernel caller already had its erasing wrapper and keeps its original
release scope; this introduces no new teardown context. Ordinary byte Drop adds
no guard acquisition, waits, capability/root cleanup, remote teardown, recovery
custody or physical completion proof. Complete local allocation does not
establish aggregate heap admission or fallible-allocation/OOM guarantees.

## Controls and host validation

The exact baseline protocol sources are built in an isolated Cargo package.
Two release controls fail: dropping a bare encoded frame, and abandoning it
through an ordinary early return, both release the initialized public fixture
secret prefix un-erased. Each observes one allocation, no growth and one release.
The original kernel wrapper pattern passes with erasure. A separate baseline
fixture confirms aggregate Debug contains both credential fields.

The observer reads eight known initialized public fixture bytes with volatile
reads immediately before System deallocation. It reads no freed memory,
padding or uninitialized spare capacity. Corrected tests check each of eight
field ranges in independent runs, abandonment, same-allocation ownership
transfer, invalid/oversized profiles before allocation and the 64 KiB limit.
Observed releases erase the selected range with one allocation and no growth.
These checks do not inspect every byte or prove physical erasure; growth
counts alone establish neither movement nor retired-storage contents.

A Python standard-library struct fixture independently supplies the complete
188-byte public frame. Both baseline and corrected encoders match it. An
independent Debug-bound Cargo control compiles against the baseline and rejects
the corrected type with `E0277`; the protocol crate retains a compile-fail
doctest for that bound. Six new runtime tests cover the encoder contract.

Debug protocol/S3/transport suites pass **77 runtime tests and seven compile-fail
doctests**. Optimized launch/Kafka/protocol/S3/transport suites pass
**167 runtime tests and 11 compile-fail doctests**. Strict Clippy, workspace
formatting and whitespace checks pass.

The first integration build rejected three malformed-profile fixture insertions
with `E0308`: their collection still held plain vectors after the encoder's
return type changed. The collection now retains erasing owners; final builds
pass. That failed run is preserved separately. Isolated controls initially
resolved cached zeroize 1.9.1; their original outcomes are retained, and all
final baseline/parity/trait controls repeat with exact 1.9.0 to match the
repository. Expected negative controls and this provisional dependency mismatch
are separate from corrected passing validation.

## Native validation and remaining work

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes.

A fresh-storage RustFS TLS run passes **20/0/0**, bitmap `0x303ffff`, including
the 33-byte PUT/HEAD/GET/DELETE round trip. Kernel SHA-256:
`6172b6b82f52c117ca0fb35498cfc6ea0b5175f31f6fbfeab2e65e8abbbedc5c`.
This covers the actual kernel encoder/launch caller and service-local profile,
TLS and signing paths. Byte-release, malformed and maximum-size observations
come from host fixtures. Native mapping/submission failures and exhaustive
server behavior remain unqualified.

Fixture images use locally verified immutable repository digests. Running
server inspection captures image ID
`sha256:67f06d4b3479fd9d323d8a99c86aac411e665ac0981d06b52a2b790c66db359e`.
The initialization container has its configured immutable reference and local
inspection, without a separate running-instance ID capture. Project-scoped
post-run container and volume inspections are empty.

Caller credential inputs, explicit encoded/field copies, provisioning/logging
buffers, read-only launch-memory backing, third-party TLS state, compiler
spills/registers/cache and physical memory remain separate SEC-15 boundaries.
Ordinary Drop requires execution; forced termination, panic-abort and power
loss are outside these observations. Protected enrollment, rollback-resistant
state and recipient-key custody remain missing; production trust stays disabled.

All **180 formal inputs** match preceding evidence: 34 modules, 36 safe and
109 negative configurations plus the runner. No fresh TLC cases were run;
these local byte owners and trait bounds are outside existing projections.
The manual PDF remains untouched, with its start/final hash in the evidence.
