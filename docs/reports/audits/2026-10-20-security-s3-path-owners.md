# SEC-15: decoded request-key and raw-path owners

Date: 2026-10-20. Reviewed baseline: `0abce15`.

The decoded S3 key and constructed raw object path now have erasing owners
before copying. The [tracked evidence](evidence/2026-10-20-security-s3-path-owners.json)
contains controls, diagnostics, source/artifact hashes and execution results.
SEC-15 remains **partial**; overall counts remain
**29 corrected / 6 partial / 3 open**.

## Finding and correction

The service's former private request copied a decoded key into a plain vector
and derived Clone. Its profile path helper built a growing plain string from
bucket, prefix and key. Ordinary success, cancellation or rejection released
those local key/path bytes without erasure. Canonical URI encoding already had
its separate owner; the original raw path and key had different lifetimes.

The byte-only request now lives in `charlotte_s3::request::OwnedRequest`.
`decode` uses the existing protocol decoder to validate the complete current
record before allocating. It reserves the key length, constructs
`Zeroizing<Vec<u8>>` before copying, and preserves the decoded flags, lengths,
range and payload digest. `key()` lends a slice and the unused Clone derive is
removed. The request remains an independent snapshot of its borrowed input.

`object_path` borrows that request and the service's validated launch-profile
bucket/prefix fields. It keeps the existing UTF-8, leading-slash, NUL and
literal dot-segment key checks. It computes the complete byte-length sum and
reserves `Zeroizing<String>` before concatenating. Prefix trimming, raw path
bytes and subsequent percent encoding are preserved. This helper formats data;
it grants no object/signing authority or permission to choose a caller-supplied
service scope. Actual service calls still use their immutable profile fields.

Wire-valid opaque keys can still reject at path construction. That rejection
leaves the key with its request owner until ordinary Drop; path construction
does not consume the request. Invalid wire records reject before key allocation.
Original caller/IPC buffers and profile/configuration fields keep their own
lifetimes. Checked sums/reservations prevent local growth without establishing
aggregate heap admission or fallible-allocation/OOM guarantees.

The service's IPC memory/mapping lifetime, operation ordering and permission
checks are unchanged. The byte helper copies the key before the existing
mapping drops; it does not retain a mapping through networking. The private
plain request/path implementations are removed. The core library adds one
internal dependency edge to the shared wire crate; Cargo.lock changes only
that edge, with no third-party version change.

The existing [C18 cleanup row](../../reference/cleanup-recovery.md) records these
local owners. Their ordinary Drop erases/frees bytes without capability calls,
waits, guard acquisition or remote teardown. No new registry, owner family,
wire tag, persistent format, cryptographic algorithm, physical cleanup proof
or business replay authority is introduced.

## Controls and host validation

Three baseline controls fail:

| Control | Baseline observation |
|---|---|
| Decoded key, normal Drop | One allocation; observed key prefix un-erased |
| Raw path, normal Drop | One observed allocation, three growths; observed key range un-erased |
| Decoded key after path rejection | One allocation; observed key prefix un-erased |

They use explicit byte-operation adapters of the former private request decoder
and profile path formatter. Live mapping/profile access is replaced by borrowed
wire bytes and profile fields; original validation/copy/format/append operations
remain. No baseline guest run or mapping-failure observation is claimed. The
prefix/Unicode/percent-encoding fixture also passes against that adapter.

The observer reads eight known initialized public fixture bytes with volatile
accesses immediately before actual host release. It never reads freed memory,
padding or uninitialized capacity. The corrected controls observe erasure,
one allocation, no growth and one release. Growth counts do not prove allocator
movement or contents of retired storage. The input wire fixture is separately
owned; no erasure claim is made for that original caller buffer.

Eight new host tests cover those releases, invalid wire admission before
allocation, forbidden keys before path allocation, Unicode/percent encoding,
prefix trimming, maximum key/scalar preservation and snapshot independence
from mutated/dropped input bytes. Wire validation and existing path-policy
results are preserved. Independent Cargo-managed Clone controls accept the
baseline adapter and reject the corrected type with `E0277`; a compile-fail
doctest retains that API policy. Explicit caller copies remain possible.

Debug S3/transport suites pass **54 runtime tests and four compile-fail
doctests**. Optimized launch/Kafka/S3/transport suites pass **144 runtime tests
and eight compile-fail doctests**. Strict Clippy, workspace formatting and
whitespace checks pass. Expected baseline/trait rejections are retained
separately from passing runs. No incidental corrected build/test failure was
observed.

## Native validation and remaining scope

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes. A fresh-storage RustFS
TLS guest run passes **20/0/0**, bitmap `0x303ffff`, including the 33-byte
PUT/HEAD/GET/DELETE round trip. Kernel SHA-256:
`bd61dc6749135c727f61be8209f36fa53ae63b3939f222360037dc22c572462d`.
The guest checks actual request decoding, path construction and signed HTTP
calls. Rejection/Unicode/maximum-key byte observations come from host fixtures;
no exhaustive server or native mapping/submission-failure coverage is inferred.

Both fixture images use locally verified immutable repository digests. Running
server inspection captures image ID
`sha256:67f06d4b3479fd9d323d8a99c86aac411e665ac0981d06b52a2b790c66db359e`.
The initialization container has its configured immutable reference and local
inspection, without a separate running-instance ID capture. Project-scoped
post-run container and volume inspections are empty.

Original IPC/caller inputs, profile/configuration data, exported metadata, TLS
record/key-schedule state, kernel backing, compiler spills/registers/cache and
physical memory remain separate SEC-15 boundaries. Ordinary Drop requires
execution; forced termination, panic-abort and power loss are outside these
observations. Protected enrollment, rollback-resistant state and recipient-key
custody remain unimplemented; production trust remains disabled.

All **180 formal inputs** match preceding evidence (34 modules, 36 safe and
109 negative configurations plus the runner). No fresh TLC cases were run;
these local byte owners/trait bounds are outside existing protocol projections.
The manual PDF remains untouched, with its start/final hash in the evidence.
No LaTeX or PDF rendering change is included.
