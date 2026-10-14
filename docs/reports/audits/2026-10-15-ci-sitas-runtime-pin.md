# CI Sitas runtime pin and panic diagnostics

Date: 2026-10-15. CharlotteOS baseline `04b45974`.

## Reported failure and source mismatch

The reported GitHub AArch64 security job expired at
`self_test/el0_sitas.rs:491:18`, waiting for the SITAS result. At 24.349440 s:
17 passed, 1 failed, 1 pending; passed bitmap `0x3fbff`, failed `0x400`, pending
`0x20000000`. The remaining scoped-launch-security test was pending, not a
successful or failed completed execution. The panic also reported no frame
chain and the runner could not symbolize raw backtrace addresses. The supplied
excerpt has no run URL or payload hashes, so it does not identify every original
boot input or independently establish other pending-test behavior.

All six CI Sitas checkout sites used the immutable but obsolete
`94eca68554e88f06721df4ccf3a69222a963c2c9` pin. That backend calls unchecked CQ
syscalls 12/42, retired by CharlotteOS runtime ABI 3. It ignores the new rejection
contract; compilation and current ELF signing alone do not establish syscall
compatibility. Local native validations instead used sibling Sitas commit
`d27f7e7b7a6e3657c9b39e53da02cf0cd8f91a78`, which uses checked syscall 88 and
retains monotonic deadlines.

Initially the published Sitas head was `2c0f8009b02b0976886c9a01f766d695ee0c59fd`.
After the owner pushed Sitas, remote HEAD/main verified as `d27f7e7`. CI now pins
that published commit directly in every detached checkout. No patch application,
older-decoder fallback, kernel compatibility syscall or timeout increase is added.
The local `../../gautelis/sitas` working tree remains unchanged. Old-source
validation uses an isolated archived CharlotteOS workspace and Sitas checkout.

## Frame-pointer correction and regression checks

CI's global `RUSTFLAGS=-D warnings` took precedence over target-specific Cargo
flags, removing the kernel's forced frame pointers. CI now uses
`CARGO_BUILD_RUSTFLAGS` for its default warning policy; both custom kernel targets
explicitly retain denied warnings and `force-frame-pointers=yes`. Userspace code
generation keeps its existing settings. This preserves diagnostics without
changing the allocation-free unwinder, test assertions or monotonic deadlines.

Four host CI-input checks verify the pinned Sitas backend's wait number against
the current SDK, reject the actual earlier pinned backend, require every CI
checkout to use the shared immutable pin and preserve denied warnings/frame
pointers on both kernel targets. The checks run in the ordinary host suite.
They read pinned Git objects, not dirty dependency worktree content, and require
no network access or mutation of the sibling repository.

## Negative execution

The old pinned source was compiled and signed in an isolated workspace. Only
its staged Sitas ELF replaced the saved current fixture for the negative run;
the original ELF was restored before the corrected build. A fresh v4 store,
four LPs and the AArch64 security profile reproduced the reported timeout with
the original 10-second verifier deadline and a 180-second runner window:

```text
17.851550 s: SELFTEST COMPLETE: passed=17 failed=1 pending=1
passed_bitmap=0x3fbff failed_bitmap=0x400 pending_bitmap=0x20000000
panicked at crates/catten/src/self_test/el0_sitas.rs:491:18:
self-test deadline expired while waiting for SITAS result
```

Runner exit 1. The corrected compiler configuration preserved four raw panic
frames and symbolized `verify_el0_sitas+0x64c` and `kernel_thread_trampoline+0x8`.
That demonstrates the diagnostic repair in a real failing run. It is separate
from a passing repeat and is not a physical quiescence or cleanup receipt.

Negative signed Sitas ELF: 152,776 bytes, SHA-256
`516fc02803646daa6ea95dd784a2d0fba2a36c1bd0062a8887a911621604d06a`.
Executed negative kernel:
`633fbc61de1d7e2c06d7305c0c11d5d9408c0595b821894bb8be24882d135d7f`.
Saved pre-negative Sitas ELF:
`a01b596eab01adc9f8bf7ac45228b1b21da9dcdb00b9e38799850358d01ed564`.

## Corrected execution

The ordinary builder rebuilt/signed the service bundle and Sitas image from the
published local `d27f7e7` checkout before guest launch, using CI's new warning
environment. A fresh v4 store, four LPs, AArch64 security profile and 180-second
runner window passed **19/0/0** at 55.575567 s, bitmap `0x2003ffff`; runner exit 0.
The SITAS fixture completed at 6.931773 s with basic_kv length 3, 1024 verified
mailbox records and joined sum 81. The 10-second verifier deadline is unchanged.
The scoped-launch-security test completed both configured generations at
33.400671/55.563970 s; it is no longer pending in this passing execution.
All 64 scheduled IPC claim/endpoint close episodes passed with their original
exact exit watches at 8.598144 s. This does not attribute historical IPC failures.

The full host suite passed **483 Rust tests, 0 failed, 4 ignored**, plus the four
new CI-input checks and existing signing, boot/input and object-store controls.
Strict kernel Clippy passed on ARM's CI security profile and x86's acpi profile
with the new warning configuration. Workspace rustfmt, shell syntax and diff
checks passed. The ordinary native runner verified kernel assembly permissions.
The sibling Sitas working tree remained clean and unchanged throughout.

| Corrected payload | SHA-256 |
| --- | --- |
| Executed ARM security kernel | `7f840b011349dae609640de1f13c1acf913c416ba111028ab2b20f3139a5bfdf` |
| Signed Sitas image, 152,776 bytes | `a01b596eab01adc9f8bf7ac45228b1b21da9dcdb00b9e38799850358d01ed564` |
| Signed NS image, 178,008 bytes | `6c473655f837bc398f1108ef6300038f5635a0b8d66286345a02506a04712eaa` |
| Bootstrap security probe, 280,400 bytes | `261d5ebb3af8b28ae970aad70e6118cecb0b774b8ba1c23f6e50a251cbd77d6c` |
| Independent-root probe ELF | `953e8bb42811da628d09173b97ed4cb4d6e1fb935b23d59673351931cc704891` |
| Admitted descriptor | `e7926e4ed812d4466eebe027aec09a0932106248159cdd127887c80d87d19bba` |
| Alternate descriptor | `a73c70a937e7e2e6e9a714616b55c74e2937a004fb40a2c7c9b7d588e453690e` |
| Artifact public-key file | `7d2ddeb51550782ad70bfb104853726da66f7b1a7839f93fc936d3e0a63bbedc` |
| Deployment public-key file | `1807eb46a2e38bd72eddea560afdb35c4c13e7967f2acd1c3ef3cf1ec9963a3c` |

Kernel identities are the captured executed runner payloads. NS/Sitas/bootstrap
probe signatures independently verified after the run; no private key bytes
are printed or published. Negative and positive security profiles generate fresh
independent fixture roots, so their kernel inputs differ beyond the Sitas binary;
this is a reproduced failure/corrected execution pair, not an identical-input
single-variable kernel comparison. The source ABI mismatch and negative failure
are retained rather than replaced by the passing result.

## Qualification

This corrects CI build inputs and diagnostic compilation. It changes no CQ
readiness producer, ownership/retirement policy, physical cleanup proof or
accepted-request replay rule. Historical IPC exact-exit and Raft persistence
attribution remain separate. The GitHub pipeline itself must run after the
CharlotteOS changes are published; local equivalent-profile validation does
not claim an unobserved GitHub job passed. Results and artifact identities are
retained inline, without ignored build files or temporary-log evidence links.
