# SEC-15: HPKE recipient state erasure

Date: 2026-10-19. Reviewed baseline: `44bb3b1`.

This follow-up corrects missing Drop erasure in the selected X25519 dependency
and keeps generated recipient private bytes in a zeroizing return owner.
SEC-15 remains **partial**; the audit remains **29 corrected / 6 partial /
3 open**. The [tracked evidence](evidence/2026-10-19-security-hpke-erasure.json)
contains source/dependency hashes, baseline diagnostics, compile controls and
execution results. All key material used in retained-storage controls is a
fixed public fixture.

## Finding and correction

The locked `hpke 0.14.0` dependency selects `x25519-dalek 3.0.0` with default
features disabled and only `static_secrets` enabled. In that configuration,
`StaticSecret` and `SharedSecret` have destructors, but their erasing statements
are compiled out. HPKE's wrapper comments assume the underlying types erase
on Drop. The baseline feature graph and two retained-storage tests reproduce
the discrepancy: both owners retain nonzero bytes after destruction.

`charlotte-launch` now depends directly on X25519 with `zeroize` enabled.
That feature also enables Curve25519's erasure support. Production constant
trait checks require `ZeroizeOnDrop` for both underlying owners. This adds no
runtime call or allocation. An isolated build with the feature removed rejects
both checks, independently of any other workspace member enabling the feature.
The retained-storage tests also check actual named-field behavior; a marker
trait by itself would not measure erasure.

`operations::generate_recipient_keypair` formerly serialized private bytes to
an ordinary array. The signing tool wrapped that returned array later. The
helper now constructs `Zeroizing<[u8; 32]>` before serialization and transfers
it directly to its caller. Public bytes remain an array. Moving the private
owner prevents implicit reuse of its old binding; explicit copies or clones
still require the caller's lifetime policy. The tool retains this owner through
file writing and error returns.

After failed envelope authentication, `operations::open` now uses `Zeroize`
for the attempted plaintext prefix, replacing an ordinary fill. Earlier
validation rejection leaves output untouched. Successful plaintext and unused
output capacity remain the caller's responsibility. The existing host and
kernel callers already keep their output in zeroizing buffers.

These are local erasure changes in existing owners, recorded in C18's
[cleanup/recovery row](../../reference/cleanup-recovery.md). They add no locks,
capability cleanup, recovery registry, physical-release proof or replay right.
The HPKE suite, envelope bytes and stored key format remain unchanged.

## Reproduction and validation

The baseline Drop tests retain each owner's allocated `MaybeUninit` slot,
destroy its one initialized owner, and read only its exposed 32-byte field
through volatile raw reads. They do not inspect padding or freed memory, or
adopt another owner. The final probes compute the field offset while borrowed,
then derive post-Drop pointers from the retained storage. The exploratory probe
kept an `AsRef`-derived pointer instead; its result is retained separately, and
an isolated repeat against committed baseline source confirms both failures
with the final probes. The test-only direct dependency enables only
`static_secrets`, matching HPKE's baseline feature selection. Both tests fail:

```text
x25519_private_owner_erases_on_drop: retained byte 0; left 90, right 0
x25519_shared_owner_erases_on_drop: retained byte 0; left 188, right 0
```

With erasure enabled, both debug and optimized repeats pass. The debug launch
suite passes **62 runtime tests and one compile-fail doctest**. The optimized
launch/S3/Kafka suite passes **84 runtime tests and three compile-fail
doctests**; the final signing-tool suite passes **19 runtime tests**. These
include envelope context/recipient/expiry checks, authenticated tampering,
owned key generation, and the existing hash/HMAC and connector vectors.
Signing-tool coverage checks matching public/private files, private mode 0600,
existing-output preservation, public-write failure rollback and private-write
rejection before public output. These file assertions do not measure heap
erasure after return.

An independent Cargo-managed control compiles against the baseline array API,
with a warning that dropping the copied array does nothing. Against the new
API it rejects reuse with `E0382`. The isolated feature-removal control rejects
with two `E0277` diagnostics for the required Drop marker. Both are expected
negative controls, retained separately from passing checks. Warning-denied
Clippy and workspace formatting pass.

Both AArch64 and x86 service bundles build and sign; AArch64 source/signature
freshness passes. A normal AArch64 development boot completes **17 passed /
zero failed / zero pending**, bitmap `0x1ffff`, including recipient binding,
shared manifest/gate policy and one-shot trust publication. This is native
functional evidence, without a target-memory erasure measurement.

The first ARM runner attempt is retained separately: the sandbox rejected its
local QEMU diagnostic socket before any guest execution. The permitted rerun
passes. A denied process-inventory command supplies no test evidence.

The x86 QEMU Secure Boot fixture passes **all 16 cases**: valid Intel/AMD
boots, loader/config/kernel/policy substitutions, invalid policy signature,
rollback/conflict/uninstalled successor, missing/duplicate policy modules,
and unauthorized platform NS/driver signing roots. It retains exact kernel,
policy, firmware, loader and bundle hashes, plus each case's diagnostic.
This is the existing public-key development fixture; it does not qualify
production enrollment, protected state or recipient custody. Its private test
keys and VM artifacts are removed by the fixture after completion.

## Dependency observations and remaining scope

The pinned P-256 `SecretKey` and ECDSA `SigningKey` source implementations
already erase their named secret scalar on Drop. This source observation does
not qualify TLS parsing, the full TLS key schedule, or every cipher context.
HPKE's X25519 deserializer still uses a plain temporary array, and dependency
scalar intermediates and compiler-generated copies remain outside this batch.
No cached dependency source was edited.

The [bounded lifetime inventory](../../reference/secret-state-lifetimes.md)
keeps these remaining boundaries explicit. Retained-storage tests establish
observed named-field behavior on the tested host; they do not establish erasure
of every move, register, spill, cache, allocator copy or crash dump. Panic-abort,
forced domain termination and power loss do not promise ordinary Drop.
Recipient-key custody and protected production provisioning remain separate
SEC-04 work; production mode stays disabled.

All **180 model/config/harness fingerprints** match the preceding report.
These models do not project X25519 field storage or Cargo features, so there
is no transition change and no fresh TLC run is claimed. Inventory remains
**34 modules, 36 safe configurations and 109 expected counterexamples**.
No manual wire-contract change is needed. The concurrently edited tracked
manual PDF is preserved byte-for-byte.
