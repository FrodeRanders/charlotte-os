# SEC-15: consumed response metadata owner

Date: 2026-10-20. Reviewed baseline: `8ae075c`.

S3 response parsing now consumes its input and lends metadata ranges from that
same erasing owner. Independent ETag, version-ID and request-ID strings are
removed. The [tracked evidence](evidence/2026-10-20-security-s3-metadata-owners.json)
contains controls, diagnostics, source/artifact hashes and execution results.
SEC-15 remains **partial**; overall counts remain
**29 corrected / 6 partial / 3 open**.

## Finding and correction

The former parser borrowed a response and allocated plain metadata strings.
Their final release, replacement by a later metadata field, or release after a
later header rejection did not erase those independent copies. The existing
transport's erasing header buffer had a separate lifetime.

`ResponseHead::parse` now consumes `Zeroizing<Vec<u8>>`, returning one parsed
owner on success and dropping that owner on every rejection. ETag, version-ID
and request-ID fields are private validated byte ranges. `etag()`,
`version_id()` and `request_id()` lend UTF-8 views; `body_prefix()` lends any
bytes following the header, which can be binary. Range offsets follow byte
lengths through the validated header. No raw pointer reconstruction or unsafe
code is added.

`ResponseHeadBuffer` transfers its existing reserved header allocation into
the parser. It keeps returning the body suffix in its existing separate owner.
The service borrows metadata views for its unchanged wire encoder. No metadata
allocation or plaintext copy is needed for parsing. Independent string fields,
the borrowed-input/returned-offset API, and aggregate Clone/Debug/comparison
derives are removed. No compatibility parser remains.

Trimming, quoted/empty ETags, case-insensitive names, the request-ID alias and
last-value-wins behavior remain. Header errors retain their existing statuses,
including Incomplete for an unterminated direct input exactly at the limit;
the accumulator retains its separate terminal admission rule at that limit.
This is an ownership change, without broader HTTP grammar or ambiguity claims.

The whole consumed buffer remains until parsed-response Drop. Ignored and
superseded bytes are retained inside it until then; replacement does not erase
those ranges immediately. In production this extends the bounded header's
lifetime through metadata use. A direct caller passing header plus body retains
that whole input, with body views borrowing its owner. Existing caller/input
bounds apply; no aggregate heap-admission guarantee is added.

The existing [C18 cleanup row](../../reference/cleanup-recovery.md) records these
local owners. Ordinary Drop erases/frees byte storage without capability calls,
waits, guard acquisition or recovery custody. Exporting metadata into the
existing owned IPC memory intentionally creates a separately governed copy;
this batch does not qualify client/kernel memory erasure or change its mapping,
publication and close contracts.

## Controls and host validation

Five baseline controls fail:

| Control | Baseline observation |
|---|---|
| ETag on success | One independent allocation; observed metadata prefix un-erased |
| Version ID on success | One independent allocation; observed metadata prefix un-erased |
| Request ID on success | One independent allocation; observed metadata prefix un-erased |
| ETag before late rejection | One independent allocation; observed metadata prefix un-erased |
| ETag replacement | Two independent allocations; the first observed prefix un-erased on replacement |

The controls use the original borrowed parser with a host bridge that retains
an erasing input owner through parsed-result Drop. That strengthens input
cleanup and changes its lifetime for the experiment; it leaves independent
metadata construction/release unchanged. Input release already erases in these
controls. The observed failure is the independent copy. Accessor spelling and
body-view bridges let the same fixtures compile; no baseline guest run or
production consuming-parser claim is made.

The allocator observer checks eight initialized public fixture bytes with
volatile accesses immediately before actual release. It reads no freed
storage, padding or uninitialized capacity. Corrected parsing observes zero
allocations/copy releases and erasure of the original input. Replacement checks
the final selected value; late-rejection coverage includes malformed headers,
conflicting lengths and unsupported transfer encoding. These are bounded
observations, without a complete allocator/physical erasure claim.

Eight new tests also cover Unicode byte offsets, quote/whitespace trimming,
empty/absent fields, request-ID aliases, binary body prefixes, every header
split, metadata after accumulator/chunk Drop and parser-error parity. The
Unicode/alias/body fixture passes against baseline and corrected parsers.
Independent Cargo-managed trait controls accept baseline Clone/Debug bounds
and reject both corrected bounds with `E0277`; two compile-fail doctests retain
that API policy.

Debug S3/transport suites pass **46 runtime tests and three compile-fail
doctests**. Optimized launch/Kafka/S3/transport suites pass **136 runtime tests
and seven compile-fail doctests**. Strict Clippy, workspace formatting and
whitespace checks pass. Existing request/signing/transport controls remain;
Cargo.lock and third-party dependencies are unchanged. Expected baseline/trait
rejections are retained separately from passing runs. No incidental corrected
build/test failure was observed.

## Native validation and remaining scope

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes. A fresh-storage RustFS
TLS guest run passes **20/0/0**, bitmap `0x303ffff`, including the 33-byte
PUT/HEAD/GET/DELETE round trip. Kernel SHA-256:
`93e8cbd7600e990551f62aeece65faaba62246224b23b2e24e1c9c23bdf20061`.
The guest checks the actual service's metadata encoder and object protocol.
Malformed/Unicode/duplicate-field cases and byte observations are host fixtures;
no exhaustive server or native mapping/publication-failure coverage is inferred.

Both fixture image references are pinned to locally verified repository
digests. The running server inspection also captures image ID
`sha256:67f06d4b3479fd9d323d8a99c86aac411e665ac0981d06b52a2b790c66db359e`.
The removed initialization container has its configured immutable reference
and local inspection, without a separate running-instance ID capture.
Project-scoped post-run container and volume inspections are empty.

Original request/profile/raw-path data, exported metadata, TLS record/key-schedule
state, kernel backing, compiler spills/registers/cache and physical memory remain
separate SEC-15 boundaries. Ordinary Drop requires execution; forced termination,
panic-abort and power loss are outside these observations. Protected enrollment,
rollback-resistant state and recipient-key custody remain unimplemented;
production trust remains disabled.

All **180 formal inputs** match preceding evidence (34 modules, 36 safe and
109 negative configurations plus the runner). No fresh TLC cases were run;
these local byte owners and Rust trait bounds are outside existing protocol
projections. The manual PDF remains untouched, with its start/final hash in
the evidence. No LaTeX or PDF rendering change is included.
