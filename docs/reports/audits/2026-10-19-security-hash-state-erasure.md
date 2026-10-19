# SEC-15 hash-state erasure — 19 October 2026

## Result and scope

Baseline `710b8bb` commits early platform artifact-authority selection. This
batch continues SEC-15's secret-copy review. S3/SCRAM already owned their pads
and many derived keys in zeroizing storage, but the shared SHA implementation
left its state, partial block and compression schedule in ordinary memory. Its
finalizer also copied the complete hash state before padding.

A retained-storage test reproduces the unfinished-state gap on that baseline.
The correction erases the owned SHA state and scratch, removes the explicit
full-state/block copies, and shares one HMAC helper whose output retains a
zeroizing owner through SigV4 and SCRAM derivation. SHA, HMAC, PBKDF2 and protocol
outputs are unchanged.

SEC-15 remains partial. This is local memory hardening, with no demonstrated
cross-domain secret read. It does not establish complete erasure of compiler,
CPU or third-party crypto state. The [inventory](../../reference/secret-state-lifetimes.md)
and [tracked evidence](evidence/2026-10-19-security-hash-state-erasure.json)
record the exact scope and validation limits. Finding counts remain
**29 corrected in scope, six partial and three open**.

## Reproduction and correction

The test retains one `MaybeUninit<Sha256>` storage slot after destroying its
initialized owner. It reads only the initialized scalar/array fields using raw
volatile reads, with no padding read, second adoption or freed-memory access.
Input is the public byte `0x36`, repeated 65 times: one block is compressed and
one byte remains buffered. On the optimized baseline, the first state check
fails with these retained words:

```text
left: [4099202733, 2535792975, 2430268064, 3742509290,
       1692759311, 2744271233, 2185925419, 4175449650]
right: [0, 0, 0, 0, 0, 0, 0, 0]
```

An initial form started with an empty hash and observed retained public initial
constants. The refined control starts after compression; its separate capture
is the relevant evidence for derived-state retention. Neither fixture uses or
prints a real credential.

`Sha256` now implements `Zeroize` and calls it from Drop for its state words,
partial block and counters. Finalization pads the same owner. Compression
borrows a complete caller/buffer block rather than copying it; a completed
buffered block is erased before further input. The expanded message schedule
and eight working words have local zeroizing owners. This adds no allocation,
locking, capability cleanup, physical-release proof or recovery custody.

One shared `charlotte_launch::sha256::hmac_sha256` owns key normalization,
including the long-key digest, both pads, hash contexts and inner digest. It
returns `Zeroizing<[u8; 32]>`. The two old S3/SCRAM helper implementations and
plain-array HMAC return paths are removed. SigV4 retains each returned key owner;
SCRAM PBKDF2 replaces its current owner and transfers its accumulated result
owner directly to the exchange. Public SHA digests still return arrays, with
secret-derived consumers responsible for their result owners.

`zeroize` 1.9 was already in the locked dependency graph. `charlotte-launch`
adds a direct dependency with default features disabled; the lockfile adds only
that edge. No package version, cryptographic format or persistent record changes.

## Validation

The affected host suites pass **79 tests** in optimized release mode:
**59 shared launch**, **six S3** and **14 Kafka**. Existing S3 deterministic
signing, HMAC RFC 4231 and SCRAM RFC 7677 checks retain their expected outputs.
The signer separately passes **18 tests**. Debug host suites and warning-denied
Clippy across all targets of the three affected libraries also pass.

New checks include:

- Unfinished Drop erases the named fields at eight empty/partial/full-block
  lengths in retained storage, including the reproduced 65-byte case.
- Sixteen fixed SHA outputs match independent Python `hashlib` results. Every
  possible two-piece split and seven chunk sizes exercise block and padding
  boundaries through 1,024 bytes.
- Seven HMAC outputs match Python `hmac`, including empty, 64-byte and hashed
  65/131-byte keys. The return is required to have the zeroizing owner type.

Both AArch64 and x86 service bundles build and sign. The final ARM bundle
passes source/signature freshness checks. Normal ARM boot reports
**17 passed / zero failed / zero pending**, bitmap `0x1ffff`. The full x86
disposable Secure Boot matrix passes **16/16**; valid Intel and AMD guests each
complete **15/0/0**, bitmap `0x1bbff`, and the policy/loader/platform-image
substitution controls retain their required rejections. These runs exercise
the changed hash path through signer, boot policy and service-image verification.
They do not constitute a fresh external S3/Kafka/TLS interoperability run or a
performance qualification.

The first host-test command accidentally repeated `--manifest-path`; Cargo
rejected its arguments before compilation or test execution. The corrected
command passed. That invocation is separate from the intentional baseline
erasure failures and passing implementation tests.

Formatting, diff whitespace and local documentation-target checks pass. The
manual's existing connector-owner claims remain accurate, so its source and
the concurrent user PDF are left untouched. All 180 model/config/harness
fingerprints match the preceding evidence. No model transition describes these
byte-level erasure details; no fresh TLC or full-model-suite run is claimed.

## Remaining review

The returned public digest, caller-owned inputs/outputs and deliberately sent
protocol bytes have their own lifetimes. Scalar compression temporaries,
compiler moves/spills, CPU registers/caches and crash/debug captures remain
outside this observed named-storage guarantee. Panic-abort, forced domain
termination and power loss also cannot promise Rust Drop execution.

TLS session/key schedules, TLS identity parsing, HPKE/library scratch, host-tool
copies and the broader connector secret inventory remain open. The SCRAM
password still lives until its client owner drops; this batch does not replace
the exchange with consuming phase types. Production trust/custody and the
SEC-04 platform-storage work remain separate requirements.
