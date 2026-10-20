# SEC-15: owned canonical signing buffers

Date: 2026-10-20. Reviewed baseline: `d18cf9b9`.

Canonical SigV4 URI/query/header copies and returned signing metadata now
retain erasing owners from construction. The
[tracked evidence](evidence/2026-10-20-security-s3-canonical-owners.json) contains
baseline controls, independent reference values, diagnostics and artifact/source
hashes. SEC-15 remains **partial**; overall counts remain
**29 corrected / 6 partial / 3 open**.

## Finding and correction

The previous signer copied paths, query values and normalized headers into plain
strings. It returned the canonical request, signing transcript and signed-header
names as ordinary strings too. URI encoding reserved only the input byte count,
allowing growth when percent encoding expanded it. Query/header joining and
formatting also grew buffers. Header validation could reject after earlier
values had already been copied; those copies released without erasure.
These are source observations. The direct byte controls below cover final
public outputs, with no exhaustive observation of private helper allocations.

`canonical_uri` and `canonical_query` now return
`Result<Zeroizing<String>, Error>`. Encoding computes its complete byte length
before reserving. Each encoded query entry has its own erasing owner while
sorting; the joined output reserves the full name/value/separator sum. The
former bare-string query API is removed. Existing callers compile against the
current interface; the S3 service borrows its owned encoded path directly.

Header-name syntax, duplicate names and required fields are checked before
normalization copies. Lowercase names and whitespace-folded values each have
erasing owners. Sorted canonical headers and joined names reserve their exact
complete lengths before writing. ASCII whitespace folding, encoded byte order,
duplicate query-value sorting and canonical newlines are preserved.

`Signature.canonical_request`, `.string_to_sign` and `.signed_headers` now retain
erasing owners. Canonical request, scope and transcript construction share a
checked concatenation helper with authorization construction. The canonical
request digest and its hexadecimal text have erasing owners too. Existing
signature/authorization and HMAC derivation owners retain their contracts.
The reservations prevent growth in these construction paths; checked length
arithmetic is no aggregate heap budget or fallible-allocation/OOM guarantee.

Original caller inputs, raw service paths/profile/request data, response
metadata and public timestamp/payload-hex helpers keep their separate
lifetimes. The original caller can also make explicit copies. This batch does
not establish whole-application or physical-memory erasure.

The existing [C18 cleanup row](../../reference/cleanup-recovery.md) records these
local byte owners. Ordinary Drop erases/frees them without capability calls,
waits, guard acquisition or recovery custody. No new registry, owner family,
wire tag, persistent format, crypto algorithm or replay authority is introduced.

## Controls and host validation

Five baseline controls fail:

| Control | Baseline observation |
|---|---|
| Canonical URI | Initialized fixture range un-erased at release; one observed allocation and one growth |
| Joined canonical query | Initialized fixture range un-erased at release |
| Returned canonical request | Initialized path range un-erased at release |
| Returned signing transcript | Initialized canonical-hash range un-erased at release |
| Returned signed-header names | Initialized name range un-erased at release |

These controls call the baseline public implementation. A host query bridge
changes only its result shape to `Result<String, Error>` for the shared test
source; it adds no owner, reservation or byte transformation. The baseline
signer and its original callers use the unchanged original query function.
No baseline guest run is claimed.

The allocator observer reads eight known initialized public fixture bytes with
volatile accesses immediately before actual release. It never reads freed
storage, padding or uninitialized spare capacity. For query and returned
metadata, observation begins after construction: zero observed allocations
there describe the Drop window, without construction-count claims. Growth
counts do not prove allocator movement or particular retired-storage contents.
The corrected controls observe erasure; URI encoding observes one allocation,
no growth and one release. Private normalization/scope/intermediate cleanup is
qualified by source ownership and the functional/rejection tests, without a
claim that each allocation was individually observed.

Eight new host tests also compare complete canonical and signing bytes, reject
invalid/missing/duplicate headers and relative paths, and cover empty query
values, duplicate names, Unicode percent encoding and ASCII-only whitespace
folding. Python `urllib.parse`, `hashlib` and `hmac` independently calculate the
tracked fixture. That full fixture passes against both baseline and corrected
implementations, including the final signature and authorization.

Debug S3/transport suites pass **38 runtime tests and one compile-fail doctest**.
Optimized launch/Kafka/S3/transport suites pass **128 runtime tests and five
compile-fail doctests**. Existing request/response cleanup controls, SHA/HMAC
vectors and aggregate Debug rejection remain. Strict Clippy, workspace formatting
and whitespace checks pass. Cargo.lock and third-party dependencies are
unchanged. No incidental corrected build/test failure was observed; expected
baseline failures are retained separately from passing results.

## Native validation and remaining scope

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The final AArch64 freshness/signature check passes. A fresh-storage
RustFS TLS guest run passes **20/0/0**, bitmap `0x303ffff`, including the 33-byte
PUT/HEAD/GET/DELETE round trip. Its kernel SHA-256 is
`09f3a5c3c1baa1ad1b57a4bfd69b76d061c666f71332c8f1bc5ac67534f311b8`.
This checks the actual signer caller and protocol round trip. The guest fixture
uses an empty query; populated-query and Unicode observations come from host
fixtures, without a claim of exhaustive broker/server interoperability.

Running-container inspection occurred after cleanup and returned `no such
object`. Configured image references and local pre-run identities are retained,
without independent identification of the removed running server/init images.
Project-scoped post-run container and volume inspections are empty.

Caller data and response metadata, TLS record/key-schedule state, kernel socket
backing, compiler spills/registers/cache and physical memory remain separate
SEC-15 boundaries. Ordinary Drop requires execution; forced termination,
panic-abort and power loss are outside these observations. Protected enrollment,
rollback-resistant state and recipient-key custody remain unimplemented;
production trust remains disabled.

All **180 formal inputs** match preceding evidence (34 modules, 36 safe and
109 negative configurations plus the runner). No fresh TLC cases were run;
these local byte owners are outside existing protocol projections. The manual
PDF remains untouched, with its start/final hash in the evidence. No LaTeX or
PDF rendering change is included.
