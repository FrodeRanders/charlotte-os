# SEC-15: S3 response-buffer reservations and framing scratch

Date: 2026-10-19. Reviewed baseline: `e4060b7`.

S3 now reserves response-header and decoded-body storage before copying and
erases consumed or abandoned chunk-framing scratch. The
[tracked evidence](evidence/2026-10-19-security-s3-response-buffers.json) contains
baseline controls, validation, diagnostics and source/artifact hashes.
SEC-15 remains **partial**; overall counts remain
**29 corrected / 6 partial / 3 open**.

## Finding and correction

The previous header accumulator and decoded-body vector already had erasing
owners. Their final release erased their buffers, but both grew while receiving
response bytes. Erasure at final release did not cover earlier reallocations.
The chunk decoder's separate 128-byte line array retained size/extension and
trailer bytes after consumption and had no erasing destructor.

`ResponseHeadBuffer` reserves the 16 KiB header limit once. It copies only the
header prefix, recognizes the delimiter across transport chunks, and returns
the current chunk's body suffix in its own reserved erasing owner. Completion
or rejection fences further pushes. A header ending exactly at the limit is
accepted; a full unterminated header rejects immediately because no further
byte can make it valid within the bound. The service retains its existing
16-read bound and socket/TLS wait policy. Each active header parse now reserves
16 KiB even for a short header; this is a local bound, without an aggregate
heap-admission guarantee. The caller still bounds its transport chunks.

`ChunkedDecoder::decode` now returns `Zeroizing<Vec<u8>>`. It reserves the current
input length before writing; removing framing cannot produce more output than
that input. The service replaces its drained buffer with this result, including
initial body decoding. It no longer appends into a caller's growing vector.
The old public append API is removed and all callers/fixtures are updated.

Framing scratch has a zeroizing owner and erases after a consumed size/extension
line, data delimiter or trailer line. Rejection drops the current output and
fences that decoder. The existing service removes its containing GET operation
on read failure; it does not resume malformed framing or acquire replay
permission. These changes preserve the existing HTTP grammar, including the
parser's treatment of bytes after a complete chunked body; this batch does not
qualify broader HTTP ambiguity, connection reuse or metadata policy.

The existing [C18 cleanup row](../../reference/cleanup-recovery.md) records these
local owners. Drop erases/frees byte storage without capability calls, waits,
guard acquisition or recovery custody. No new registry, owner family, persistent
format, protocol tag or physical cleanup authority is introduced.

## Controls and host validation

Four controls fail against the baseline:

| Control | Baseline observation |
|---|---|
| Fragmented header | One allocation, two growths; final prefix erased |
| Multiple decoded chunks in one input | One allocation, one growth; final prefix erased |
| Consumed framing line | Initialized line bytes remain after consumption |
| Rejected framing line followed by Drop | Initialized fixture prefix fails the erasure check |

The growth controls use explicit adapters. The header adapter reproduces the
former private service loop's append/parse/body-copy byte operations with
injected chunks. The decoder bridge keeps its original append implementation
and the caller's existing zeroizing output; it adds no reservation. This
preserves final-buffer cleanup in the controls. Growth counts do not establish
whether the allocator moved storage or retained particular retired bytes.

The host allocator observer checks eight initialized public fixture bytes
immediately before actual release. It reads no freed memory, padding or
uninitialized capacity. The scratch Drop control constructs one decoder in a
`MaybeUninit` slot, captures its initialized line address, destroys it once,
and inspects those named bytes while the slot retains its backing. It adopts
no second owner. The corrected debug/optimized builds pass these controls.

Eight S3 transport tests cover no-growth output/header release, body-suffix
ownership, malformed/abandoned cleanup, terminal fencing, every header/body
split, byte-at-a-time chunk framing, overlong framing and the exact header
limit. Two library tests cover consumed and rejected scratch; existing protocol
and signing tests remain. The release suites pass **115 runtime tests and four
compile-fail doctests**. Debug S3/transport tests pass **25 runtime tests**.
Strict Clippy, workspace formatting and whitespace checks pass. The existing
release observer is shared between the two host test binaries, each with its
own allocator instance; the original nine receive-owner tests still pass.
Cargo.lock adds only the host harness's S3 dependency edge, with no third-party
version change.

The first strict Clippy run rejected the nested header-result type with
`type_complexity`. Naming the head/body pair `ResponseStart` resolves it;
the original diagnostic and passing repeat are retained separately. Earlier
passing builds/tests precede that type alias or the final split-point test;
the final evidence identifies the completed-source runs.

## Native validation and limits

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The final AArch64 freshness/signature check passes.

The fresh-storage RustFS TLS guest fixture passes **20/0/0**, bitmap
`0x303ffff`, including the 33-byte PUT/HEAD/GET/DELETE round trip. Kernel SHA-256:
`7311cfc37a44c39582926c49ee6b7b82b67b3fa11bbc23380868ff7b84f624b3`.
The running server image was captured as
`sha256:67f06d4b3479fd9d323d8a99c86aac411e665ac0981d06b52a2b790c66db359e`.
The initialization container was removed before inspection; its configured
reference and local image digest are recorded with that verification limit.
Project-scoped post-run container and volume inspections are empty.

The guest run checks the actual S3 caller and object protocol. Chunked split,
rejection and scratch-erasure observations come from host fixtures; this guest
run does not independently establish coverage of every HTTP framing case.
Request-formatting and response-metadata copies, TLS record consumption/key
schedules, kernel socket backing, compiler spills/registers and physical memory
remain separate SEC-15 boundaries. Ordinary Rust Drop also requires execution;
forced termination, panic-abort and power loss are outside these observations.
Protected enrollment, rollback-resistant state and recipient-key custody remain
unimplemented; production trust remains disabled.

All **180 formal inputs** match preceding evidence (34 modules, 36 safe and
109 negative configurations plus the runner). No fresh TLC cases were run;
these local byte lifetimes are outside the existing protocol projections.
The user's pre-existing manual PDF is preserved unchanged, with its hash in
the tracked evidence. No manual LaTeX or PDF rendering change is included.
