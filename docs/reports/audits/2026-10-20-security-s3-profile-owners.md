# SEC-15: service-local S3 profile owners

Date: 2026-10-20. Reviewed baseline: `50c5abf`.

The S3 service now retains its local launch-profile snapshot in one erasing
allocation and owns the derived HTTP authority string before copying. The
[tracked evidence](evidence/2026-10-20-security-s3-profile-owners.json) records
controls, diagnostics, source/artifact hashes and execution results. SEC-15
remains **partial**, with **29 corrected / 6 partial / 3 open** overall.

## Finding and correction

The private service profile previously copied its host, region, bucket, prefix,
access key and namespace into plain strings, and the public CA into a plain
vector. Its secret-key vector already had an erasing owner. HTTP authority
formatting produced another plain string. Ordinary Drop released those plain
copies without erasure. The existing wire decoder already validated the whole
record, including string constraints, before these copies; no late malformed
UTF-8 allocation leak is claimed.

`charlotte_s3::profile::OwnedProfile::decode` reuses that current decoder and
validates the typed fields before allocating. It reserves the complete frame,
constructs `Zeroizing<Vec<u8>>` before copying, and retains one private immutable
snapshot. `view()` reuses the same decoder to borrow typed fields without
allocation or a self-referential representation. This bounded revalidation
avoids another parser or a second set of copied fields. The service borrows a
view for connection setup, signing, status and its existing startup log.

The owner has no Clone or Debug implementation; its view has no aggregate Debug
implementation. Individual fields remain explicitly accessible for use and can
still be copied or logged. The borrowed wire protocol type retains its existing
API; this change does not qualify every provisioning/diagnostic caller.

`ProfileView::authority` reserves host length plus room for a colon and any u16
decimal port when needed. Its `Zeroizing<String>` exists before writing; formatting
requires no growth. Default-port omission, nondefault ports and signed HTTP bytes
are preserved. These byte helpers grant no new launch, object, signing or replay
authority. Actual service scope still comes from its immutable launch profile.

The runtime adapter continues to borrow launch-owned memory through `Context`.
It copies before releasing the same read-only mapping and retains no mapping
through networking. Launch ownership, TLS inputs, rights checks and status/log
fields are preserved. Current CHS3PF1 grammar, bounds and rejection behavior are
unchanged. The former plain profile and authority implementations are removed
without a compatibility path or dependency change.

The existing [C18 row](../../reference/cleanup-recovery.md) records the local
owners. Their ordinary Drop erases/frees bytes without capability calls, waits,
guard acquisition, remote teardown or recovery custody. No new registry, owner
family, persistent format, physical cleanup proof or retry permission is added.
Reservations prevent local growth; they do not establish aggregate heap admission
or fallible-allocation/OOM guarantees.

## Controls and host checks

Baseline byte-operation controls produce three expected failures: host and
access-key allocations each release with their initialized fixture prefix
un-erased; the authority string also releases with its prefix un-erased. Each
observed allocation releases once. A separate baseline secret-key control passes,
preserving the distinction between its existing owner and the other field copies.
The adapter retains the old profile fields, decode/copy and authority operations;
it removes the live Context mapping wrapper and exposes three fields for the
host observer. No baseline guest or mapping-failure result is inferred.

The observer reads eight initialized public fixture bytes using volatile reads
immediately before host deallocation. It does not read freed memory, padding or
uninitialized capacity. Seven new tests cover release ranges in all eight variable
profile fields, allocation-free borrowed views and rights, six authority/port
combinations, invalid profiles before allocation, snapshot independence, absent
namespace and the 64 KiB profile limit. Corrected release assertions observe one
snapshot/authority allocation, no growth, erasure of the selected initialized
range and one release. These observations do not inspect every byte or prove
physical erasure. Reallocation counts alone establish neither movement nor
contents of retired storage.

Two compile-fail doctests retain the new owner's Clone/Debug policy; this does
not remove any baseline trait implementation, since the former private profile
also lacked those derives. Debug S3/transport suites pass **61 runtime tests and
six compile-fail doctests**. Optimized launch/Kafka/S3/transport suites pass
**151 runtime tests and ten compile-fail doctests**; the shared S3 wire suite
separately passes **10 tests**. Strict Clippy, workspace formatting and whitespace
checks pass. Expected baseline failures are recorded separately from passing
corrected runs.

## Native validation and limits

Both service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64
ELFs**. The AArch64 freshness/signature check passes.

A fresh-storage RustFS TLS guest run passes **20/0/0**, bitmap `0x303ffff`,
including the 33-byte PUT/HEAD/GET/DELETE round trip. Kernel SHA-256:
`2bfa5b29a7e97cf802de9ffd2112d7612d5aad9b50c314c1f747491db6f40070`.
This exercises actual launch-profile decoding, TLS configuration and signed
HTTP calls. Invalid-profile, maximum-size and byte-release observations come
from host fixtures; native mapping/submission failures and exhaustive server
behavior are unqualified.

Both fixture images use locally verified immutable repository digests. Running
server inspection captures image ID
`sha256:67f06d4b3479fd9d323d8a99c86aac411e665ac0981d06b52a2b790c66db359e`.
The initialization container has its configured immutable reference and local
inspection, without a separate running-instance ID capture. Project-scoped
post-run container and volume inspections are empty.

Original launch-memory/kernel teardown, host provisioning buffers, deliberately
emitted endpoint/scope logs, caller/IPC and exported metadata copies, third-party
TLS record/key-schedule state, compiler spills/registers/cache and physical
memory remain separate boundaries. Ordinary Drop requires execution; forced
termination, panic-abort and power loss are outside the observations. Protected
enrollment, rollback-resistant state and recipient-key custody remain missing;
production trust remains disabled.

All **180 formal inputs** match preceding evidence: 34 modules, 36 safe and
109 negative configurations plus the runner. No fresh TLC cases were run;
these local byte owners are outside the existing protocol projections. The
manual PDF remains untouched, with its start/final hash in the evidence.
