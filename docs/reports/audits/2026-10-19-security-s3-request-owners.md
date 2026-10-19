# SEC-15: S3 authorization and request-header owners

Date: 2026-10-19. Reviewed baseline: `754e866`.

SigV4 authorization and hexadecimal signature text now have erasing owners
from construction. The S3 sender receives a reserved, erasing HTTP request
header directly. The [tracked evidence](evidence/2026-10-19-security-s3-request-owners.json)
contains controls, diagnostics, source/artifact hashes and execution results.
SEC-15 remains **partial**; overall counts remain
**29 corrected / 6 partial / 3 open**.

## Finding and correction

The former signer returned authorization and hexadecimal signature text as
ordinary strings. Both released their bytes without erasure. Its aggregate
Debug implementation also made those fields available to ordinary debug
formatting. The S3 service copied authorization into a growing plain string
and transferred that allocation into a plain byte vector for sending.

`Signature.authorization` and `.signature` now retain `Zeroizing<String>`.
Hex encoding reserves its fixed 64-byte output before writing. Authorization
checks the complete component-length sum, reserves once and concatenates into
its existing owner. The aggregate Debug implementation is removed. HMAC keys
and intermediate derivations keep their existing erasing owners. Canonical
request, string-to-sign, signed-header, path/query and other metadata buffers
retain their separate plain-string lifetimes; this batch does not qualify the
entire signer or its compiler copies.

The private HTTP formatter is replaced by the library's borrowed `RequestHead`
and owned `request_head` encoder. It validates field boundaries and checks the
complete output length before reserving one `Zeroizing<Vec<u8>>`. It copies
directly into that owner, without a staging string or growth. Content-Length
uses a local decimal buffer and Connection is emitted by the encoder. The
service retains the returned owner through its existing sender; ordinary
success, rejection or abandonment drops that buffer. Native send-failure
injection was not run in this batch; local release behavior is observed by the
host controls, with the sender's existing borrow/wait contract unchanged.

The serializer rejects control characters, malformed method/name/encoded-target
fields, duplicate names, absent/empty Host, and caller-supplied Connection,
Content-Length or Transfer-Encoding. This keeps framing fields under one
encoder. Existing launch-profile validation already rejects malformed host,
namespace and access-key values; the control that bypasses that profile is a
stronger serializer-boundary check. It does **not** demonstrate production
request injection. URI/canonicalization policy and broader HTTP validation
remain separate. Valid S3 header bytes, optional field order, Content-Length
formatting and deterministic SigV4 signatures are preserved.

The existing [C18 cleanup row](../../reference/cleanup-recovery.md) records these
local owners. Byte Drop adds no capability release, guard acquisition,
recovery custody, replay permission or physical cleanup proof. No new registry,
owner family, wire tag, persistent format or wait loop is introduced.

## Controls and host validation

Three baseline release controls fail:

| Control | Baseline observation |
|---|---|
| Authorization field after signing | Its initialized signature range is un-erased at release |
| Hexadecimal signature field after signing | Its initialized prefix is un-erased at release |
| Serialized request header | Its initialized authorization range is un-erased; one observed allocation and five growths |

The first two controls call the baseline public signer. Their observer begins
after signing, so their zero observed allocations describe the Drop window;
they make no claim about construction allocation counts. The header control
uses an explicit adapter of the former private formatting block with prepared
borrowed fields, preserving its write/append/into-bytes operations. It verifies
the fixture bytes before release. This is a host byte-operation control, with
no baseline guest-execution claim. Growth counts do not establish allocator
movement or particular retired-storage contents.

The observer reads eight known initialized public fixture bytes with volatile
accesses immediately before actual host release. It never reads freed storage,
padding or uninitialized spare capacity. The corrected controls observe erasure;
header construction observes one allocation, no growth and one release.
A fourth baseline control fails because the formatter accepts a supplied CRLF
value; the corrected serializer rejects all tested invalid fields with no
output allocation, subject to the profile-validation limitation above.

Five new host tests also cover exact optional-header bytes, zero and maximum
`u64` lengths, duplicate/reserved fields and invalid method/target/Host values.
The debug S3/transport suites pass **30 runtime tests and one compile-fail
doctest**. Optimized launch/Kafka/S3/transport suites pass **120 runtime tests
and five compile-fail doctests**. Existing digest, HMAC and deterministic S3
signature vectors pass. Strict Clippy, workspace formatting and whitespace
checks pass. Cargo.lock and third-party dependencies are unchanged.

An independent Cargo-managed Debug-bound fixture compiles against the baseline
and fails against the corrected type with `E0277: Signature doesn't implement
Debug`. The same bound is retained as a compile-fail doctest. These are expected
negative controls, retained separately from passing runtime results. No
incidental build/test failure was observed in the corrected validation runs.

## Native validation and remaining scope

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes. A fresh-storage RustFS TLS
guest run passes **20/0/0**, bitmap `0x303ffff`, including the 33-byte
PUT/HEAD/GET/DELETE round trip. Its kernel SHA-256 is
`26076b93457805ee5c2243beb878df4ce0187131da30d261c749cd1734714efc`.
The guest checks the actual signer/serializer caller and protocol round trip;
local byte observations and malformed-field checks come from host fixtures.

Running-container inspection returned `no such object` after cleanup. The
configured RustFS references and local pre-run image identities are retained,
with no independent identification of the removed running server/init images.
Project-scoped post-run container and volume inspections are empty.

Canonical signing/path/query buffers, response metadata, TLS record/key-schedule
state, kernel socket backing, compiler spills/registers and physical memory
remain separate SEC-15 boundaries. Ordinary Rust Drop requires execution;
forced termination, panic-abort and power loss are outside these observations.
Protected enrollment, rollback-resistant state and recipient-key custody remain
unimplemented; production trust remains disabled.

All **180 formal inputs** match preceding evidence (34 modules, 36 safe and
109 negative configurations plus the runner). No fresh TLC cases were run;
these byte owners and Debug formatting are outside existing protocol
projections. The user's pre-existing manual PDF was unchanged during the
implementation validation, with that preserved hash in the evidence. At commit
verification it matched HEAD and differed from that earlier modified copy;
the commit-time hash is recorded separately. This batch did not edit or stage
the PDF. No LaTeX or PDF rendering change is included.
