# Application SDK and userspace ergonomics qualification

14 November 2026. Source base: CharlotteOS `327a50a7`, plus the application
foundation changes accompanying this report. Toolchain: `nightly-2026-07-27`.
This checkpoint qualifies the application helpers and their integration; it does
not close kernel cleanup gates or qualify every connector operation.

## Result and scope

- `charlotte-app` has no dependency on platform service implementations.
  `catten-services` reexports its single grant/socket/Kafka/S3 client
  implementations. Grant wire data lives in `charlotte-protocol-grant`.
- Runtime helpers retain existing owners: bounded queue-pressure admission,
  deadline/wait-rejection observations, bounded payload preparation and exact
  generation-bound native worker custody. The entry wrapper runs its terminal
  action after a returning application scope. Synchronous serving retains the
  authenticated request envelope and permits one publication attempt.
- `charlotte-app-sitas` supplies owned call futures using the existing executor
  and sole backend CQ consumer. Backend wait rejection is terminal, retaining
  executor owners instead of permitting older executors' error retry loop.
- The SDK exports an exact hashed source closure, target tools, pinned Sitas
  sources, signer and project CLI. The CLI creates separate host business logic
  and an EL0 adapter, builds and explicitly signs artifacts. It never deploys.
  Its hashes are consistency checks, not signed provenance. External Cargo and
  toolchain dependencies still resolve normally.

Current contracts and remaining extensions are in the
[application SDK guide](../../guides/application-sdk.md) and
[architecture page](../../architecture/userspace-ergonomics.md).

## Passing qualification

`bash scripts/run-host-tests.sh` completed successfully against the final Rust
sources, including 66 runtime tests, 14 application tests, five async-adapter
tests, three reconnecting-example tests and SDK filesystem/integrity checks.
The tests exercise preparation/submission/mapping errors, retained loans on
deadline or rejected wait, returned-capability disposal, stale worker/interest
generations and cancellation without replay. The bridge's backend-rejection
test verifies terminal handling without releasing its registered interest.
These host seams do not qualify native capability revocation or CQ delivery.

`cargo fmt --all -- --check` and Clippy with all features/targets and denied
warnings passed for `catten-rt`, `charlotte-app` and `charlotte-app-sitas`.
The bundled AArch64 services built and signed through
`scripts/build-catten-services.sh`. The reconnecting-client and echo-service
examples compiled for AArch64 and x86_64. The echo helper itself was compiled,
not independently deployed in this checkpoint.

The exported SDK created and host-tested an independent starter, built
layout-verified ELFs for both targets, and signed artifacts with an explicit
restricted development key. The signer lockfile resolved `hpke 0.14.0`.
An additional exported AArch64 application compiled `WakeReactor`,
`CharlotteReactor` and `ShardExecutor::run` against CI's immutable Sitas revision
`d27f7e7b7a6e3657c9b39e53da02cf0cd8f91a78`. The working sibling checkout used by
repository builds was `6d63266eb52492c1958c45f3ed71c8c7a451489a`; it additionally exposes `try_run`, which the
new probe deliberately does not require. This is source compatibility evidence
for the exported pin; the SDK-specific binary was not booted.

The final native campaign used:

```sh
scripts/run-aarch64.sh debug --tests --smp 4 --security-test \
  --timeout 180 --instance ergonomics-final
```

The TCG guest completed at 85.856 seconds with both security probe publication
generations retired. Relevant diagnostics were:

```text
[13.009158] [sitas] owned IPC future verified
[13.011118] [sitas] SUCCESS: basic_kv total_len=3, mailbox index verified 1024 records, join probe sum 81, SDK/Sitas CQ probe verified 64 records
[51.651461] [security] probe passed; retired policy fenced, publication generation=1
[85.833230] [security] probe passed; retired policy fenced, publication generation=2
[85.855949] SELFTEST COMPLETE: passed=19 failed=0 pending=0 passed_bitmap=0x2003ffff failed_bitmap=0x0 pending_bitmap=0x0
```

The first line requires an actual accepted moved-memory call, backend dispatch,
future re-poll and checked returned memory. It is included in the
existing Sitas verifier result rather than changing its status ABI. Native
wait-rejection and arbitrary cancellation timing are not exhaustively covered.
This probe does not isolate reply notification from stale executor wake hints;
the separate existing CQ probe exercises native queue notification delivery.

Relevant artifact identities (SHA-256):

| Input or artifact | Digest |
| --- | --- |
| Final AArch64 test kernel | `8f4dae592203476bbfaafee5a8b8dc7d48a34bb7c6ab54c35863ad20f3a93f2c` |
| Final embedded, signed Sitas ELF | `2fcdf75148512fb5abcfaa06f92c479afc5322e30f04bda9418979bd71c19109` |
| Security probe artifact reported by signer | `df5a81817fc28c12b04cf1accea2e304e68bd47aa980c11b8e7442ad83d86415` |
| Root Cargo.lock | `83ffe1f2b6a6c33f8abb3bba4b24b1c20fcf983c2342cd6bdee01754d8298c29` |
| Exported SDK sdk.json fingerprint (dirty source snapshot) | `5b0ee4cc0d3a43db5399fb606230f158aebb6f2b9da4a2672dc87c20eef19650` |
| Exported pinned-Sitas AArch64 compile-check ELF | `a5191855826f13e8a2c25cc481a1cd70f4c62aa376ff0d03bd7691af17afa805` |

The manual rebuilt twice without unresolved references; changed application
prose and included example pages were rendered for visual inspection. A PDF
build supplies no additional runtime evidence.

## Failed and incomplete runs

The initial `debug --hvf --no-network --timeout 40` invocation failed before
boot. The kernel compiler denied 24 dead-code diagnostics, including
`device/smmu.rs::test_initialization`,
`device/mapping/tests/readiness.rs::real`, device recovery/unit-initialization
fixtures, PCI reset fixtures and `service/store/tests.rs::live`:

```text
error: function `test_initialization` is never used
error: function `live` is never used
error: could not compile `catten` (bin "catten") due to 24 previous errors
```

Those kernel files were unchanged by the application foundation. At that
checkpoint the HVF configuration remained unresolved; the build follow-up below
records its separate correction. A separate sandboxed TCG
attempt failed to create its QMP socket with `Operation not permitted`; the
authorized repeat booted and passed 17 non-security self-tests.

The first security invocation used a 60-second runner window. It completed the
first primary probe and retired publication generation 1 at 51.366 seconds,
then ended during the second generation:

```text
[51.366399] [security] probe passed; retired policy fenced, publication generation=1
[52.593261] [security] reply CQ: 32 controller denials drained through owned multi-source waits
error: guest failure/deadline captured
```

That run is incomplete, not passing. The fixture's existing documented/CI
180-second runner window was then used; no per-operation budget was increased.
The first documented campaign passed 19/0/0 at 86.248 seconds (kernel
`b2a430e6206672a6bb599129a47dbf378c6acc6be8048628362d031f7e4f368b`).
After adapting to pinned Sitas' common `run` API and terminal wait-error policy,
the final campaign above passed independently.

During development, a worker Drop test initially invoked native logging before
its mock abort and terminated with SIGSYS. The mock now records/panics without
issuing a native syscall. The first full host repeat after adding the echo
example rejected its unconditional `no_std` test configuration; its test harness
now follows the reconnecting example's conditional entry configuration. The
final full host suite passed after both corrections.

The diagnostics and identities needed to interpret these outcomes are recorded
inline here; temporary logs/build outputs are not durable evidence dependencies.

## HVF build follow-up

The subsequent build investigation traced the 24 denied diagnostics to
hardware-only fixture helpers compiled under `kernel_tests` while their boot
callers were excluded by `hvf_compat`. The helper entrypoints, worker state and
imports now share those exclusions. The unit-initialization reexport and x86
IOMMU dispatcher match their backends. Synthetic admission/ownership tests and
shared private-release probes remain available; no warning policy was weakened
and no production resource operation changed.

Both image-only commands completed, including signed service bundles, kernel
compilation, boot-profile verification and native assembly-section checks:

```sh
scripts/run-aarch64.sh debug --tests --hvf --no-network --build-only --instance hvf-build
scripts/run-x86_64.sh debug --tests --no-network --build-only --instance intel-build
```

| Build artifact | SHA-256 |
| --- | --- |
| AArch64 HVF compatibility kernel | `645131e752ff7c2bc55dce9790ff2ceffabd7febb2e2b9b116fdca9c8b3d5e13` |
| AArch64 HVF boot image | `9e63aec706e816f2e3f093339f83917a524a6386e546b298599f3a83400680e0` |
| x86-64 ordinary test kernel | `abdd818944ace6a8d1ae091c1e2f14cd0ab6549bb07cd37ccf67f1042f614203` |
| x86-64 test boot image | `bdcfb124c6952b29af959940ae6adb3944e64366d170727d7c93cd33db698e78` |

The HVF image verified one native assembly entry and the x86 image verified
249. The workflow now includes an AArch64 HVF compile/profile/section check on
Linux, preventing this feature combination from depending on access to a Mac.
Local CI-input, boot-profile, tooling-contract and boot-image tests passed.
The ordinary AArch64 test configuration also passed `cargo check`; the final
x86 test configuration rebuilt with the same kernel digest above. Workspace
formatting and diff checks passed.
This follow-up is build evidence only: neither image was booted under HVF,
Fusion or KVM here. The earlier passing TCG campaigns remain separate.

For an Intel iMac the relevant guest is x86-64. The repository's
[VMware guide](../../platforms/vmware-x86_64.md) records prior Intel Fusion
qualification with a guest-visible VT-d unit, NVMe and E1000E; that is different
from QEMU's reduced AArch64 HVF profile and was not repeated in this follow-up.
Native Intel Linux with the existing x86 runner's `--kvm` option is the next
accelerated QEMU validation path; it requires accessible `/dev/kvm`.
