# SEC-15: SASL authentication frame ownership

Date: 2026-10-19. Reviewed baseline: `d1e92b6`.

SASL reply parsing now consumes and retains one complete zeroizing frame,
lending its authentication range instead of allocating a separate copy.
Requests validate the complete frame bound before copying and encode into a
reserved erasing owner. The [tracked evidence](evidence/2026-10-19-security-sasl-frames.json)
contains baseline failures, source/artifact hashes and execution results.
SEC-15 remains **partial**; counts remain **29 corrected / 6 partial / 3 open**.

## Finding and correction

The previous parser copied `auth_bytes` into a plain `Vec` before reading the
session-lifetime trailer and checking for trailing bytes. Success returned that
independent vector without an erasing owner. A missing trailer or extra data
rejected after copying, releasing the copy without erasure. The connector's
original transport frame had a separate lifetime too.

`parse_sasl_authenticate` now takes `Zeroizing<Vec<u8>>` by value. It validates
framing, correlation, metadata, the authentication range and the complete
trailer while borrowing that owner. Success transfers the same frame into
`SaslAuthenticate`; `auth_bytes()` lends its validated range. Rejection drops
the input owner. The reply's copied public vector and derived Clone/Debug
implementations are removed. Borrowing error-message metadata was already
allocation-free and remains unchanged.

The previous request helper used the ordinary growable encoder and checked
the complete payload limit at finalization. An authentication field could fit
its individual limit while the complete frame exceeded the limit after header
and client-ID overhead. That case copied the field, then rejected before the
caller's successful-result wrapper existed. The current connector's bounded
SCRAM messages do not reach this composite-limit case; it is a library
admission/cleanup gap. Its successful request caller already wrapped the final
buffer, so the bare-output probe alone demonstrates no production request
leak.

The request helper now checks client-ID and complete payload bounds first.
It reserves the full frame, constructs `Zeroizing<Encoder>` before writing,
and transfers the same allocation as `Zeroizing<Vec<u8>>`. Header writing and
final-length validation remain shared with the ordinary encoder; other Kafka
request helpers keep their existing API. There is no compatibility parser or
plain SASL request wrapper.

The connector passes response frames into their consuming parser and keeps
returned request owners directly. The first request/reply and client-first
buffers live in the challenge-handling scope, ending before the final exchange.
The final SCRAM phase still owns proof and expected signature through terminal
verification. Existing configured credentials and transport owners keep their
separate contracts.

C18's [cleanup/recovery row](../../reference/cleanup-recovery.md) records these
local byte owners. Their ordinary Drop erases/frees buffers without capability
calls, guard acquisition, recovery custody or physical completion authority.
No wire fields, nullable-field behavior, broker-error metadata, version numbers
or persistent records change.

## Reproduction and validation

Four baseline controls fail. They use public fixture bytes and a host-only
allocator observer, checking eight initialized authentication bytes immediately
before actual release. The observer never reads freed memory, padding or spare
capacity. Thread-local state leaves other tests inactive.

| Baseline control | Fresh allocations | Growths | Authentication prefix erased |
|---|---:|---:|---|
| Bare request output | 1 | 3 | No |
| Oversized complete request | 1 | 3 | No |
| Accepted reply's independent copy | 1 | 0 | No |
| Missing reply trailer after the copy | 1 | 0 | No |

The baseline parser test bridge adds an erasing input owner around the original
borrowed-input parser. That strengthens its input cleanup for the experiment;
it leaves the independent authentication copy and its rejection behavior
unchanged. Both accepted and rejected copies still release un-erased bytes.
Growth counts do not establish whether the host allocator moved an allocation
or retained specific old bytes.

The corrected request probe observes one allocation, no growth and an erased
prefix at release. Complete-size rejection allocates nothing. Reply success,
missing/extra trailer and early rejection allocate no authentication copy and
erase the original input on release. Additional cases verify exact request
bytes, the largest admitted complete frame, client-ID rejection, variable
error-message lengths, empty/nullable authentication fields, broker errors,
correlation rejection and malformed lengths.

Debug Kafka checks pass **28 runtime tests and three compile-fail doctests**.
Optimized launch/S3/Kafka suites pass **96 runtime tests and four compile-fail
doctests**. These retain the existing Base64 allocation probes, SCRAM vectors
and authentication rejection coverage. Two independent Cargo-managed fixtures
reject with `E0382` for input reuse and `E0505` for dropping a reply while its
authentication borrow remains in use. Warning-denied Kafka Clippy and workspace
formatting pass. Both service bundles build/sign; AArch64 source/signature
freshness passes.

An initial probe build failed with `E0502` in its own truncation expression.
Computing the new length before the mutable call corrects that harness issue.
It is retained separately and supplies no runtime ownership evidence.

The native AArch64 connector runs against three Apache Kafka 4.1.1 brokers with
verified TLS, required P-256 mTLS and SCRAM-SHA-256. Its isolated Compose project
is `charlotte-sec15-sasl`. The host kills route leader `kafka2` in the required
fault window. The guest completes **20 passed / zero failed / zero pending**,
bitmap `0x503ffff`:

```text
low-level offset 1, generic step commits=4 produced=4 retries=2 dlq=1
timeouts=1 group_generation=3 heartbeats=2 metadata_refreshes=2
coordinator_refreshes=1 retry_attempts=6 terminal_errors=2
output_route_produced=5 lag=1
```

One `TLS handshake transport failed (6)` diagnostic occurs between the fault
window and successful completion. It remains alongside the passing result;
no stronger causal conclusion or extra authentication success is inferred.
The run covers leader failure and generic transactional steps. It does not
qualify native malformed-frame erasure, coordinator-movement/producer-fencing
scenarios or native x86 Kafka execution.

Separate project-filtered inventories confirm that containers and volumes are
removed. Per-container image inspection ran after cleanup and rejected all
three missing objects. Its status/stderr are retained separately. The configured
image reference and locally inspected repository digest are recorded; immutable
image IDs for the running containers were not captured.

## Remaining boundaries

The connector's transport accumulator and TLS receive buffers still precede
this owned frame boundary. Constructing the response owner after transport
return does not erase earlier growth or plaintext transport copies. TLS key
schedules, HPKE/library scratch, host-tool copies and physical heap/page
teardown remain separate. This batch adds no replay permission or authentication
bypass result. Production provisioning/custody remains open and production
mode stays disabled.

These observations cover selected initialized host storage, not every compiler
move, expression, register, spill, cache or allocator byte. Panic-abort, forced
domain termination and power loss do not promise ordinary Drop.

All **180 model/config/harness fingerprints** match the preceding evidence.
The existing models do not project SASL frame bytes or these Rust containers;
no transition change or fresh TLC result is claimed. Inventory remains **34
modules, 36 safe configurations and 109 expected counterexamples**. Existing
manual wire/phase descriptions remain valid; the tracked user PDF is preserved
byte-for-byte.
