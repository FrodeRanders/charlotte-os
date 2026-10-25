# SEC-15: erasing x86 GCM authentication-state owner

Work dates: 2026-10-25 to 2026-10-26. Reviewed baseline: `4158ff6a`.

Freestanding x86 services now select POLYVAL's existing erasing software owner
instead of its pinned non-dropping dispatcher. Their production authentication-
state guard rejects removal of the backend flag or erasure feature. The TLS
cipher remains AES-128-GCM-SHA-256; dependency versions, source and lockfile are
unchanged. This is a concrete SEC-15 correction. The ledger stays **29 corrected /
6 partial / 3 open**, with SEC-15 partial and production trust disabled.

The [tracked evidence](evidence/2026-10-25-security-x86-gcm-owner.json) records
source/configuration/dependency/artifact hashes, expected failing controls,
native receipts and relevant failure diagnostics.

## Source-confirmed gap and correction

The [earlier GCM review](2026-10-22-security-gcm-erasure.md) enabled separate
erasure features in AES-GCM, GHASH and POLYVAL. Its AArch64 software backend
clears named state fields. POLYVAL 0.6.2's x86 dispatcher, however, wraps its
active software or PCLMUL backend in a `ManuallyDrop` union and has no Drop
delegation. Dropping the dispatcher therefore does not drop that original
authentication state, regardless of which backend CPU detection selects.
Consuming finalization of an extracted backend is a different path. No
cross-domain read or retained-memory extraction was demonstrated here.

The `x86_64-unknown-none` Cargo target now supplies `--cfg polyval_force_soft`,
retaining denied warnings. This selects the pinned software implementation;
with the already-enabled `zeroize` feature, its ordinary destructor clears
the `h` key and `s` accumulator fields. `GHash` and the existing record cipher
carry that same owner. No new buffer, allocation or capability owner is added.

The production guard now requires `needs_drop` for both selected authentication
types on AArch64 and freestanding x86. The type-equality link to the TLS-selected
cipher and existing AES `ZeroizeOnDrop` checks remain. Destructor presence is
necessary; pinned-source inspection establishes its field-wiping semantics.
This is not a post-Drop target-memory measurement or proof of every secret copy.

Explicit `RUSTFLAGS`/encoded flag overrides can shadow Cargo target flags.
They now reject a deployed TLS build if they restore the non-erasing dispatcher;
disabling the erasure feature while retaining software selection rejects too.
The independent kernel target keeps its original frame-pointer/warning flags;
CI's `CARGO_BUILD_RUSTFLAGS` cannot silently replace this target policy.

This choice disables PCLMUL authentication acceleration for freestanding x86.
It does not disable AES acceleration, change the cipher/wire format, enable a
CPU feature or alter transport/reconnect policy. Throughput impact is unmeasured.
Restoring acceleration requires a separately qualified erasing backend.
Hosted x86 outside this target configuration remains unqualified. The optional
ARM PMULL implementation still fails the existing AArch64 destructor guard.

## Controls and qualification

`scripts/tests/test-tls-gcm-backend.py` compiles the exact production guard for
the actual freestanding x86 target, with versions checked against the lockfile:

| Control | Result |
| --- | --- |
| Repository target flags, erasure enabled | Compiles |
| Erasure enabled, encoded flags override software selection | Expected `E0080` on `needs_drop` |
| Software selected, erasure features disabled | Expected `E0080` on `needs_drop` |

The two expected failures are kept separately from passing builds. The control
uses cached exact-version dependencies and the actual Cargo target configuration,
not a duplicate backend model. It is also invoked by `run-host-tests.sh`; that
hook requires the repository's existing nightly toolchain and `rust-src`.

The existing public all-zero GCM vector now lives in a test-only source shared
by its host test and the signed opt-in `security_probe`. The probe checks the
actual TLS-selected cipher on the guest CPU: expected ciphertext/tag, changed-tag
rejection without ciphertext mutation, then authentic decryption. Its explicit
log receipt precedes the existing probe modes. No status grammar, authority or
check bitmap changes. Ordinary service code does not include the vector helper.

Targeted service-library, reconnecting-client, Kafka and S3 suites pass **263
runtime tests and nine doctests in each of debug and release**. Strict service
Clippy passes for AArch64 library/binaries and the x86 library/security probe.
The CI-input checks still pass all four cases, including kernel frame-pointer
preservation. Formatting and whitespace checks pass. No incidental source-build
or test failure occurred; negative erasure controls are the intended rejections.

Both service builders and signing steps completed from the frozen source; the
staged inventories contain **41 AArch64 and 37 x86 ELFs**, including the existing
external Sitas fixture in the AArch64 input set. Dependency source, checksums and the workspace lockfile
are unchanged. Original capability/root ownership and hardware/physical proofs
are unchanged; the existing C18 record cipher only gains its local named-field
erasure at ordinary Drop. No allocator, guard, logging, wait, remote teardown,
registry, custody or physical/retry authority is added to that destructor.

## Native execution and artifacts

Fresh-storage four-LP runs executed sequentially, each completing its first
guest attempt with `reason: complete`. Both security publication generations
(1 and 2) passed on every backend:

| Backend | Deferred passed / failed / pending | HTTP checks passed | Native GCM receipts |
| --- | --- | --- | --- |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 5 |
| x86_64 / AMD-Vi | 18 / 0 / 0 | 14 | 5 |
| AArch64 / SMMUv3, with DHCP and S3 TLS | 22 / 0 / 0 | 14 | 5 |

The GCM receipts cover the actual selected cipher in signed EL0 probe instances,
including the probe's existing alternate modes. They are functional cipher
checks, not fifteen independent post-Drop memory measurements. x86 has native
cipher and ordinary network integration coverage here, not an x86 TLS server
round trip.

The AArch64 fixture verifies a **33-byte S3 PUT/HEAD/GET/DELETE round trip** over
TLS. Its server and CLI image IDs/digests were inspected before launch and
selected by immutable image ID. Running-container inspection confirmed the exact
server image and isolated `charlotte-gcm-owner-arm` project. Runner cleanup left
that project's container and volume inventories empty. Public development
credentials and generated ephemeral CA/server keys are the fixture inputs; no
production credential or private key bytes are recorded.

Host attempts remain separate from guest results. Every run records four
`ConnectionResetError` retries: one after EOF, one after abortive peer closes and
two during the idle-client probe. AMD and AArch64 each additionally record one
initial-metrics `TimeoutError`. All affected probes subsequently pass; the
evidence preserves all **fourteen retry diagnostics** and their eventual results.
No guest failed or was repeated. No timeout, attempt bound or resource limit was
widened. These results do not attribute historical scheduler, IPC, root-close,
metrics or NIC failures, nor measure this backend's throughput impact.

The manual was rebuilt twice and affected PDF pages 117–118 rendered and
visually checked. The full 249-page build still emits 190 overfull-box warnings;
this scoped check does not re-audit every page. No undefined references remain.
Frozen code/configuration hashes, local Markdown targets, workspace formatting
and whitespace checks pass. The new compile-control hook was executed directly
and the host driver syntax checked; the complete cross-workspace host wrapper
was not rerun. All formal inputs/transitions remain unchanged; no TLC or new
refinement result is claimed for these local cryptographic lifetimes.

## Remaining SEC-15 scope

TLS traffic-key/IV arrays and HKDF state, explicit/compiler/register/cache copies,
caller/launch/socket/kernel backing and physical memory have separate lifetimes.
Hosted x86 outside the freestanding target policy and optional ARM PMULL remain
unqualified. Ordinary Drop requires execution; forced termination, panic-abort
and power loss remain outside this correction. Protected production enrollment,
rollback-resistant policy state and recipient-key custody are still missing
boundaries. SEC-15 stays partial; the other finding dispositions are unchanged.
