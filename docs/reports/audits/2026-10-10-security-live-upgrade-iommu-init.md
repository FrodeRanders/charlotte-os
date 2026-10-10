# Live-upgrade IOMMU boot initialization regression (2026-10-10)

Found while validating the
[SEC-36–38 remediation](2026-10-10-security-audit-remediation-2.md): the
`live_upgrade_test` kernel panicked during boot at
`device/smmu.rs:1113` (`test_reject_sparse_map`), before any upgrade request.
This report records the root cause, the correction and the validation. It is a
functional/availability regression, not a new audit finding.

## Root cause

Commit `b25a570a` ("Claim IOMMU boot initialization before unlocked
preparation") removed lazy IOMMU/SMMU unit initialization from ordinary device
operations: `with_smmu`/`with_vtd` now require an already installed unit, and
only the boot path may initialize a vacant one through `initialize_early`.

The boot call was still gated `not(feature = "live_upgrade_test")`
([main.rs:226](../../../crates/catten/src/main.rs#L226) and
[main.rs:241](../../../crates/catten/src/main.rs#L241)). Previously that gate
was harmless because `initialize_early` lazily initialized the unit on first
use. After `b25a570a`, nothing initialized the unit in that build, so
`recovery_tests::run()`
([main.rs:314](../../../crates/catten/src/main.rs#L314)) failed its first
registry probe with `Unsupported` and panicked the kernel. The same build
would later have failed the protected-DMA NVMe/object-store stack.

Evidence from the failing runs (kernel
`a9ede00d56e10c6d942004c74e743e8d2321e1b696bb979721474ab21521813d`,
identical on two consecutive boots):

```text
[device/smmu.rs:1113] called `Result::unwrap()` on an `Err` value: Unsupported
[security-probe] (no SMMU early-init line in either log)
```

The `live_upgrade_test` mode is documented as requiring the protected-DMA
object store ([aarch64.md](../../platforms/aarch64.md)), so initializing the
unit is required for that mode, not optional.

## Correction

The boot initialization blocks now run under `not(hvf_compat)` only, so both
the default and `live_upgrade_test` builds initialize the platform IOMMU at
boot; `hvf_compat` remains excluded (and is mutually exclusive with
`live_upgrade_test` by `compile_error!`). The real-unit preparation probe
(`test_initialization`) runs in both modes, matching the default boot, and
keeps the fixture live under that feature set.

- AArch64 SMMU: [main.rs:226](../../../crates/catten/src/main.rs#L226).
- x86_64 VT-d/AMD-Vi: [main.rs:241](../../../crates/catten/src/main.rs#L241).

No ownership contract changed: the fix restores the single boot-time unit
initialization that `b25a570a` made mandatory.

## Validation

- Fresh-storage AArch64 `--live-upgrade-test` guest: **SELFTEST COMPLETE:
  passed=2 failed=0 pending=0** (`0x10800`), with
  `[smmu] early initialization complete.`, `[device recovery] ... QEMU NVMe
  reset/reassignment passed`, `[service] SUCCESS: bootstrap delivery, name
  lookup, stale-connection failure, and restart generation all verified`,
  `[service] persistent upgrade completed`, `[nvme] SUCCESS: storage stack
  verified`, and the 12 KiB PRP-list and 2 MiB + 4 KiB persistent object
  round trips plus MSI-X and DMA-domain checks. Kernel SHA-256:
  `11305986d6a2a0a740f2a33d79dc305e4cdf502492a28fa37547e1bdce4f413b`.
- Clippy with `-D warnings` passed for four configurations: AArch64 and
  x86_64 kernels in both default and `live_upgrade_test` feature builds.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `scripts/run-host-tests.sh`: passed, **342 tests, zero failures**.
- Fresh-storage AArch64 security guest rerun after the change: **19 passed,
  0 failed, 0 pending** (`0x2003ffff`), scoped probe `checks=0xffff`, kernel
  SHA-256
  `c96cf4e705ce07240500526246793b5f4825e0ec8381a8eea44d994b479ac5a1`.

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18847 CATTEN_DEPLOY_HOST_PORT=18147 \
  scripts/run-aarch64.sh --live-upgrade-test \
  --instance smmu-fix-upgrade --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18849 CATTEN_DEPLOY_HOST_PORT=18149 \
  scripts/run-aarch64.sh --security-test \
  --instance smmu-fix-sec --fresh-storage --timeout 180
```

The x86_64 `live_upgrade_test` configuration is compile- and Clippy-checked
but was not guest-executed in this pass (no x86 QEMU run). The fix is
symmetric, and the aarch64 failure and pass are both directly observed.

## Limits

This restores the intended boot invariant; it does not qualify production
IOMMU reset/drain behaviour (SEC-18 G3/G7 and R18-6 remain open). The focused
mode still skips the topology probe, async demo, scheduler lifecycle and
boot rebalance by design; the device topology is forced lazily by the first
fixture that needs it, as it was before the regression.
