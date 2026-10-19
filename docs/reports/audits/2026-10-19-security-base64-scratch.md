# SEC-15: SCRAM Base64 scratch ownership

Date: 2026-10-19. Reviewed baseline: `9be5546`.

The SCRAM Base64 helpers now construct zeroizing output owners before writing.
The decoder reserves the complete output bound once and erases any decoded
prefix on rejection. This continues the [secret-state inventory](../../reference/secret-state-lifetimes.md);
SEC-15 remains **partial**, with **29 corrected / 6 partial / 3 open** findings.
The [tracked evidence](evidence/2026-10-19-security-base64-scratch.json) retains
baseline failures, probe scope, artifact hashes and execution results.

## Finding and correction

The previous decoder built a plain `Vec` by pushing bytes into an initially
empty allocation. Its successful verifier caller wrapped the result in
`Zeroizing` after return. A later invalid Base64 quantum could return an error
before that wrapper existed, releasing the initialized prefix without erasure.
A successful decode also grew twice in the fixture. Late wrapping protects
the final allocation; it cannot cover the earlier error path or possible copies
left by growth.

The decoder now creates `Zeroizing<Vec<u8>>` with capacity for at most three
bytes per four-character quantum. Its output never exceeds that bound. Success
transfers the owner, while every error drops the same partial-output owner.
The existing SCRAM message bound still limits production decode input to 4096
bytes, with a separate 1024-byte decoded salt limit.

The encoder already reserved its full output before writing, and production
proof/nonce callers already wrapped its successful result. Returning
`Zeroizing<String>` from construction strengthens that helper boundary; its
baseline probe does not demonstrate an existing production encoder leak.
Nonce, proof and verifier callers now retain their returned owners directly.
The decoded salt has the same owner policy. Local quantum digits and packed
words also use zeroizing owners; scalar expression/compiler copies remain
outside that named-storage contract.

C18's [cleanup/recovery row](../../reference/cleanup-recovery.md) records the
same local userspace scratch family. Ordinary Drop erases/frees these buffers
without guards, capability calls or a recovery registry. There is no new
physical-release proof or replay authority. Wire bytes, accepted padding and
grammar, Kafka framing and persistent records are unchanged.

## Reproduction and validation

The host-only allocator probe delegates operations to `System`. Thread-local
state arms only around three public fixtures, whose first allocation is the
helper output. Each fixture initializes at least eight bytes before release,
including late rejection. The probe checks only that initialized prefix,
immediately before actual deallocation. It never reads freed memory, padding
or uninitialized capacity. Other tests leave the probe inactive.

Four baseline controls fail. Three use the helper's bare result or late error;
a fourth reproduces the caller's existing successful-result wrapper:

| Control | Growths | Prefix erased before final release |
|---|---:|---|
| Encoded output, bare helper result | 0 | No |
| Decoded output, bare helper result | 2 | No |
| Rejected suffix after a decoded prefix | 2 | No |
| Successful decoder with the caller's late wrapper | 2 | Yes |

The last control rejects on growth alone. These observations do not prove that
the host allocator moved a particular allocation or retained old bytes after
a reallocation. Removing growth closes that possible copy boundary; the late
error probe separately demonstrates an actual un-erased release.

The final debug and optimized probes observe one output allocation, no growth,
one release and an erased prefix. They cover successful encoding/decoding and
late invalid digits, nonterminal padding and mixed padding. The debug Kafka
suite passes **21 runtime tests and two compile-fail doctests**. Optimized
launch/S3/Kafka suites pass **89 runtime tests and three compile-fail doctests**.
They retain the RFC 7677 exchange, fixed Base64 encoding examples, padded and
unpadded round trips across every length through 1024 bytes, and authentication
error cases. Warning-denied Kafka Clippy and workspace formatting pass.

Both AArch64 and x86 service bundles build and sign. AArch64 source/signature
freshness passes. The actual AArch64 connector then runs against three Apache
Kafka 4.1.1 brokers with verified TLS, required P-256 mTLS and SCRAM-SHA-256.
The fixture uses its own Compose project, `charlotte-sec15-base64`. The host
kills route leader `kafka2` during the required fault window. The guest completes
**20 passed / zero failed / zero pending**, bitmap `0x503ffff`:

```text
low-level offset 1, generic step commits=4 produced=4 retries=2 dlq=1
timeouts=1 group_generation=3 heartbeats=2 metadata_refreshes=2
coordinator_refreshes=1 retry_attempts=5 terminal_errors=2
output_route_produced=5 lag=1
```

Two `TLS handshake transport failed (6)` diagnostics occur between the fault
window and successful completion. They remain in the evidence alongside the
passing result; no stronger causal conclusion or extra authentication success
is inferred from them. This run covers leader interruption and generic
transactional steps. Coordinator-movement and producer-fencing scenarios were
not rerun, and native x86 Kafka execution remains unqualified.

The fixture removes its containers and volumes, confirmed by separate empty
project-filtered inventories. Per-container image inspection ran after removal
and returned three `no such object` errors. That metadata failure is retained
separately. The configured image reference and locally inspected repository
digest are recorded; immutable image IDs for the running containers were not
captured. An earlier three-probe baseline run is also retained separately from
the final isolated four-control reproduction.

## Remaining boundaries

These observations cover the selected output allocation's initialized prefix
on the tested host. They do not qualify every allocator byte, move, scalar
intermediate, register, spill, crash dump or physical page. Panic-abort, forced
domain termination and power loss do not promise ordinary Drop.

Kafka SASL frame encoding/parsing and transport buffers retain their separate
copy review. In particular, `SaslAuthenticate::auth_bytes` still owns a plain
`Vec`; this batch does not extend the Base64 owner contract to that parsed
frame. TLS key schedules, HPKE/library scratch, host-tool copies and recipient
custody remain open boundaries. Production mode remains disabled.

All **180 model/config/harness fingerprints** match the preceding evidence.
The existing models do not project Base64 byte storage or allocator growth, so
no transition change or fresh TLC run is claimed. Inventory remains **34
modules, 36 safe configurations and 109 expected counterexamples**. No manual
wire-contract change is needed, and the concurrently edited tracked PDF is
preserved byte-for-byte.
