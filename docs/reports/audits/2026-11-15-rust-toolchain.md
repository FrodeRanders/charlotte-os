# Rust nightly upgrade qualification

15 November 2026. Source base: `f12f39ccf6ec6c34a985c22ebbb2749b881986e1`,
plus the migration accompanying this report. Sitas source and CI pin:
`6d63266eb52492c1958c45f3ed71c8c7a451489a` (clean sibling checkout).

## Change and safety boundary

The repository, CI workflows and exported application SDK now select
`nightly-2026-11-14`, `rustc 1.102.0-nightly (5f4effd57 2026-11-13)`, replacing
`nightly-2026-07-27`, `rustc 1.99.0-nightly (dc3f85158 2026-07-26)`.
The host was Apple Silicon macOS. Both bare-metal architectures were rebuilt;
the installed RISC-V standard library was verified but no RISC-V guest was run.
Cargo was `1.102.0-nightly (ede6cefea 2026-11-11)`, LLVM `23.1.5` and both
QEMU system emulators `11.1.2`. Native qualification below uses TCG with four LPs.

The unstable allocator extension gate is now `allocator_ext`. Shared Arc backing
requires the unsafe [`AllocatorClone`](https://doc.rust-lang.org/nightly/std/alloc/trait.AllocatorClone.html)
contract: interchangeable allocation domains, no unwind on clone, and no
invalidation through movement/drop while equivalent clones survive.
`ChargedAllocator` retains the existing private holder and requires that contract
of its underlying allocator. `ByteSponsorClone` separately requires immutable
original funding across clones; only unit and captured principal sponsors opt in
in production. The frame tracker's mutable initial-binding sponsor stays excluded.
Its vector uses the separate safe `AllocatorNightly` opt-in. No blanket promise
is inferred from ordinary `Clone`.

Nonzero-block controls allocate with the sponsored/byte allocator, drop the
original and free through its clone, checking original charges through actual
free. Existing charged-Arc controls cover allocation rejection, rollback,
concurrent final clones and final weak release. A negative compilation control
rejects clone-equivalent shared backing for the mutable frame-tracker sponsor.
The 20 charged-allocator tests and 50 real frame-tracker tests pass, including
the existing eager-free rejection. These changes add no representation fields,
pool, physical-cleanup authority or earlier refund. Q4 and finding-wide
acceptance gates remain open.

Other compiler adaptations remove the now-stable `abi_custom` feature gate,
use atomic `try_update` in host fixtures, iterate existing bounded register/scope
arrays and simplify two service expressions without changing short-circuiting.
The vendored TLS copy adds only missing deprecation-version metadata; its
provenance is updated. Unused runtime/graft/filesystem dependency edges are
removed and fixture-only kernel DNS/discovery dependencies are enabled by their
actual features. Cargo.lock package versions and source revisions do not change.
No warning policy, resource ceiling, signature policy or crypto backend is relaxed.

## Development failures and corrections

These observations preceded the passing qualification; they are not passing runs:

- `rustup toolchain install` installed the requested compiler, Rust source,
  rustfmt, Clippy, LLVM tools and three bare-metal standard libraries, then
  exited 1 during optional self-update: `failed to set permissions for
  '/Users/froran/.cargo/bin/rustup-init': No such file or directory`.
  Component inventory and the actual compiler were independently verified.
- The first host repeat failed at standalone `Box::try_new` after removal of
  the old allocator gate. Restoring the renamed `allocator_ext` gate fixed it.
- ARM checking rejected `Vec::drain` without `AllocatorNightly`; the explicit
  safe collection marker resolved it without granting mutable sponsors the
  unsafe clone-equivalence contract.
- x86 checking denied the now-stable `abi_custom` feature attribute. Removing
  the attribute retains the existing ABI use.
- Strict Clippy initially rejected three TLS deprecation attributes lacking
  `since`, the ARM register loop, two service library expressions and the
  clusterctl final boolean. The source/provenance fixes retain the existing
  bounds, volatile writes, parser rejection and conditional reply observation.
  Cargo also identified unused dependency edges, handled as described above.
- An additional deployment-feature compile check first stopped at the build
  script's required signed-descriptor guard (`deploy fixture requires signed
  descriptor; use the boot runner: NotPresent`). Preparing the normal signed
  fixture and supplying its bundle made the check pass. No guard was bypassed.
- The first ARM security campaign passed 19/0/0 at 106.128766 seconds, with
  kernel SHA-256 `4f045eea3fda10099e3b53e6cd176ab3dbd05a575a101b80c71d64109ab2d93c`.
  Its service bundle preceded the final service lint/dependency edits, so a
  final-source repeat was required. That repeat first failed before guest boot:
  `Could not set up host forwarding rule 'tcp::8081-:7444'`. It overlapped the
  Intel guest on the same host port; the diagnostic result was `guest-exited`,
  with no serial execution. The subsequent ARM run uses a distinct forwarding
  port (18811). This failed launch supplies no guest-runtime qualification.

## Future compiler boundary

Current x86 builds pass, but Cargo reports future incompatibilities from `core`,
`aes 0.8.4`, `chacha20 0.10.2`, `curve25519-dalek 4.1.3/5.0.0`,
`poly1305 0.9.1` and `sha2 0.10.9/0.11.0` across the kernel and service
graphs. The diagnostic category is:

```text
warning: enabling the `sse` target feature on the current target is unsupported due to LLVM backend issues
warning: this was previously accepted by the compiler but is being phased out; it will become a hard error in a future release!
```

This is emitted for SIMD target-feature declarations with the current soft-float
x86 custom target; it also occurs inside the compiler's own `core` source.
See [Rust issue 117938](https://github.com/rust-lang/rust/issues/117938).
It is an unresolved future-toolchain boundary, not evidence of native SIMD
execution. The kernel target remains soft-float/no-SIMD, and the software GCM
authentication-state erasure controls remain mandatory. Enabling SIMD would
require separate ABI/context-save qualification. No warning suppression or
unreviewed dependency/backend upgrade is used here.

## Qualification

`scripts/run-host-tests.sh` completed with its default `-D warnings` Cargo
policy on the final Rust sources, through the signer self-test marker. It
includes the production allocator/ownership harnesses, crypto backend rejection
controls, TLS parser and interop tests, application/runtime/Sitas tests and SDK
integrity checks. The new CI-pin test passes five cases and SDK tests pass two.
`cargo fmt --all -- --check`, `git diff --check` and the capacity census pass;
the selected 24-pool policy sources and numerical limits are unchanged.

The complete CI Clippy matrix passes with `--locked` and `-D warnings`: ARM
workspace/default and kernel/minimal; x86 workspace/minimal and kernel/tests;
ARM release services with `-Z build-std=core,alloc`; and all host signer targets.
The additional `acpi,clusterctl_test` compile check passes with its normal signed
deployment fixture, covering the newly optional DNS/discovery dependencies.
These are local checks, not a claim that hosted GitHub jobs have run.

The exported SDK verifies its source inventory, creates and host-tests an
independent starter, builds layout-checked ARM and x86 ELFs, and signs both with
an explicit restricted copy of the public development key. It selects the new
compiler and the same immutable Sitas pin. The SDK-specific binaries were built
and signed, not independently deployed. Its source snapshot is marked dirty;
its exact fingerprint identifies that export, not signed release provenance.

| SDK artifact | SHA-256 |
| --- | --- |
| Exported `sdk.json` fingerprint | `a4e66d1674894485ab84b752405e3ae22c91267eecf0422b85897ba8b9eaec9e` |
| Signed starter ARM ELF | `9b237f0a1aedf090dd4a25cfa7ec76d57e788c3f17313c71eb65329d6ba980de` |
| Signed starter x86 ELF | `e66398b1256d1928a6893a4361408f36d6b305e80978969605fe5fdb44102322` |
| Final root Cargo.lock | `c516b44bd41c5aa77f4df60e3e6249f2019df65ae19675e83911b2a85f08524a` |

The native campaigns rebuild/sign their service bundles (ARM through
`scripts/build-catten-services.sh`) and enforce ELF assembly permissions and
test/minimal bundle selection. Commands use fresh, named instances:

```sh
CATTEN_DEPLOY_HOST_PORT=18811 scripts/run-aarch64.sh debug --tests --smp 4 \
  --security-test --timeout 240 --instance rust-upgrade-arm-repeat
scripts/run-x86_64.sh debug --tests --smp 4 --security-test --iommu intel \
  --timeout 240 --instance rust-upgrade-intel
CATTEN_DEPLOY_HOST_PORT=18812 scripts/run-x86_64.sh debug --tests --smp 4 \
  --security-test --iommu amd --timeout 240 --instance rust-upgrade-amd
CATTEN_DEPLOY_HOST_PORT=18813 scripts/run-aarch64.sh debug --minimal --smp 4 \
  --timeout 120 --instance rust-upgrade-arm-minimal
CATTEN_DEPLOY_HOST_PORT=18814 scripts/run-x86_64.sh debug --minimal --smp 4 \
  --iommu intel --timeout 120 --instance rust-upgrade-x86-minimal
```

The final ARM campaign completes 19/0/0 at 86.724190 seconds, bitmap
`0x2003ffff`; Intel completes 17/0/0 at 108.576814 seconds and AMD 17/0/0 at
107.789269 seconds, both with bitmap `0x2003bbff`. All failed and pending bitmaps
are zero. Both publication generations pass and retire on all three targets,
with native GCM known-answer/tamper/decrypt and selected SHA/HMAC checks.
Representative final ARM backing diagnostics include:

```text
[namespace account principal] actual cap/endpoint/record/mailbox bundles=72/80/104/104 bytes; sibling sharing, 256 heap-held denials, independent sponsor, three root-prefix rollbacks, promotion, four final-weak refunds after ASID reuse and 14560 real bundles at 1048576-byte principal ceiling passed
[reactor account principal] waiter/scheduler/timer/record/CQ/watch bundles=72/72/72/64/80/64 bytes; sibling sharing, 384 heap-held denials, four root and eight namespace prefixes, healthy sponsor, immutable promotion and 16381 real bundles at 1048576-byte ceiling passed
```

Both minimal profiles pass actual boot validation with local storage ready and
operational services submitted (ARM at 4.014280 seconds; x86 at 4.502921).
Both subsequently report DHCP `10.0.2.15/24`. Their bundles omit the security
probe and regression fixtures; they report boot completion without synthesizing
regression-suite results.

| Native artifact | SHA-256 |
| --- | --- |
| Final ARM security kernel | `03a7e23e787314d68bae22b0f1678fa84a9bc3f902af306d6dd5059bb09c6bca` |
| Final ARM deployed security artifact (signer digest) | `4de01243b49be659fcf489fdab4a49c60b4a35c9af13fe2c57cf3ece437010fc` |
| Intel security kernel | `25308fec0ef468971ea0d32859a526198283d9e9f88a4ce257e87a9b495a279f` |
| Intel deployed security artifact (signer digest) | `72fe5c17d01c06a43c16f9517388e331997406873189e31daccdd6f7ba6ca188` |
| AMD security kernel | `7bf55527494cef77ac09949ac00d4b78ec346a5a89fa85d2527933820e0c3d2b` |
| AMD deployed security artifact (signer digest) | `a4f6473d9033ee5d47f5ea6f87a8bd5fb77b2855fc0cb6f38dd5f0a86c7d2f93` |
| ARM minimal kernel | `82367aca568d7f608c15ae6181a629f36613575beb0921b80f2a88c3cc92799e` |
| x86 minimal kernel | `02e15a1d3c2e4b6c9642f0bdce22ed456b8aff97f4c1a669a280f95302c11349` |

These results qualify the selected emulated configurations. They do not qualify
physical hardware, HVF/KVM, every optional network campaign or all possible
cancellation schedules, nor close the audit's remaining cleanup/heap-fit gates.
