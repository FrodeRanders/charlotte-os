# Freestanding x86 soft-float future-incompatibility warning

Investigated 15 November 2026 at `fd2cc8725fc7450b33bd125fbc9ae49ea5eff2b1`.
The warning comes from one target/compiler conflict: the x86 service target
selects the soft-float ABI, while dependencies and rebuilt `core` define
SSE/AVX functions. An isolated compile comparison confirms that changing the
target resolves this diagnostic without changing the tested crypto versions.
The repository target and runtime ABI remain unchanged; runtime compatibility
and any production target migration are unqualified by this investigation.

## Build identity

| Input | Identity |
| --- | --- |
| Toolchain channel | `nightly-2026-11-14` |
| rustc | `1.102.0-nightly`, commit `5f4effd579500175af844241eb179e756190f1e7`, dated 13 November 2026 |
| LLVM | `23.1.5` |
| Host | `aarch64-apple-darwin` |
| Cargo.lock SHA-256 | `c516b44bd41c5aa77f4df60e3e6249f2019df65ae19675e83911b2a85f08524a` |
| Service x86 target SHA-256 | `2b0725bd2ac5562f68b110a420e5f780df3e3f5f407abecb02ea7f86b82f7a59` |
| Kernel x86 target SHA-256 | `201448bf16914c3832c246efb3affabcb4247317add134c757b6ffe3dcd8907f` |

The service target is `crates/catten-services/x86_64-unknown-none.json`;
the independent kernel target is `target_specs/x86_64-unknown-none-catten.json`.
Both specify `rustc-abi: softfloat` and disable baseline SIMD while enabling
`+soft-float`. The AArch64 service target has a different ABI/feature selection.

## Diagnostic and dependency paths

Cargo's service-target report, ID 1, contains only this warning category:

```text
enabling the `sse` target feature on the current target is unsupported due to LLVM backend issues
```

| Reported package | Diagnostic entries in the captured report |
| --- | ---: |
| aes 0.8.4 | 29 |
| chacha20 0.10.2 | 12 |
| core 0.0.0 | 8747 |
| curve25519-dalek 4.1.3 | 148 |
| curve25519-dalek 5.0.0 | 70 |
| poly1305 0.9.1 | 13 |
| sha2 0.10.9 | 5 |
| sha2 0.11.0 | 9 |

These 9033 entries include intrinsic and macro expansions; they do not represent
9033 distinct defects. Examples include `aes-0.8.4/src/autodetect.rs:137`
(`target_feature(enable = "aes")`),
`chacha20-0.10.2/src/backends/avx2.rs:35`, and rebuilt `core`'s
`portable-simd/crates/core_simd/src/swizzle_dyn.rs:259`. AES/AVX feature
implications also enable SSE, explaining the common diagnostic text.

AES is both a direct service/management-TLS dependency and the backend of
AES-GCM 0.10.3. ChaCha20 0.10.2 enters through
`charlotte-launch -> hpke 0.14.0 -> chacha20poly1305 0.11.0`.
`core v0.0.0` is Rust's own library rebuilt for the custom target by
`-Z build-std`; its source path names the host toolchain installation, not the
guest architecture. The report is retained in the shared service target
directory and can reappear on subsequent cached builds.

[Rust PR 160302](https://github.com/rust-lang/rust/pull/160302), merged
23 August 2026, adds the x86 soft-float/SSE compatibility lint. Its rationale
links [LLVM backend failures](https://github.com/rust-lang/rust/issues/117938)
for the mixed AVX2/soft-float configuration. This is a substantive compatibility
constraint, not evidence by itself of a dependency vulnerability or an observed
runtime cryptographic failure. Updating packages alone does not resolve the
same intrinsic definitions in rebuilt `core`.

## Isolated compile comparison

The private probe used this complete library source:

```rust
#![no_std]
#[target_feature(enable = "sse2")]
pub unsafe fn probe() {}
```

Its standalone Cargo manifest used edition 2024, an empty `[workspace]`, and
exact dependencies `aes = 0.8.4` and `chacha20 = 0.10.2`, both with default
features disabled. Each target had its own fresh target directory. Both checks
used the pinned compiler, offline dependency resolution and
`cargo check -Z build-std=core,alloc` with an explicit JSON target.

The soft-float probe copied the repository service target, removing only
`pre-link-args`, since this check does not link a service. The comparison copied
that probe target and removed `rustc-abi`, selecting Rust's default x86 ABI,
then replaced `features` with:

```text
-mmx,+sse,+sse2,-sse3,-ssse3,-sse4.1,-sse4.2,-avx,-avx2,-soft-float
```

| Check | Observation |
| --- | --- |
| Soft-float target | Exit 101: the local SSE2 function is rejected by `x86_softfloat_sse`, denied by default as part of `future_incompatible`. |
| Default x86 ABI with SSE2 | Exit 0; the same AES/ChaCha versions and rebuilt core compile without the incompatibility diagnostic or a future-incompatibility report. |

The expected failing control reported:

```text
error: enabling the `sse` target feature on the current target is unsupported due to LLVM backend issues
 --> src/lib.rs:2:18
  |
2 | #[target_feature(enable = "sse2")]
  |                  ^^^^^^^^^^^^^^^
  = note: `#[deny(x86_softfloat_sse)]` (part of `#[deny(future_incompatible)]`) on by default
```

Both probe manifests also emitted incidental unused-dependency warnings: the
probe compiles the dependencies but its empty function does not call them.
No warning suppression was applied. The successful comparison is a type-check
experiment, not service linking, crypto execution, or native ABI qualification.

## Resolution boundary

The current kernel already requires FXSR/SSE2, initializes CR0/CR4, saves/restores
512-byte x87/SSE state with FXSAVE64/FXRSTOR64 during context switches, and clears
OSXSAVE to withhold unsaved AVX state. These source observations make a separate
normal-ABI userspace target a plausible resolution. They do not prove every
entry, interrupt, preemption or boundary safe under changed code generation.

A target change must retain the kernel's separate code-generation policy and
qualify syscall/interrupt/entry assembly, stack alignment, cross-thread and
cross-domain FP/SIMD preservation, CPU feature dispatch and signed execution
target compatibility. Review whether any externally visible ABI changes require
a runtime-ABI increment. Keep the existing `polyval_force_soft` erasure policy;
it addresses an independent destructor gap and must not be lost while changing
target flags. Rebuild/sign all affected x86 services and exercise native crypto,
TLS and scheduler/preemption checks before claiming a supported migration.

Keeping soft-float instead would need an upstream-supported exclusion of
incompatible vector definitions throughout dependencies and rebuilt `core`.
Selecting portable crypto backends alone cannot remove the core definitions.
This investigation makes neither runtime changes nor a claim that suppressing
the lint repairs the unsupported combination.
