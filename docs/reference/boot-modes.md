# Boot validation and regression modes

Optimization (`debug`/`release`) and test selection are independent. Development
runners keep the registered native suite enabled by default. Use `--minimal` to
build an operational node without native regression fixtures or bundled probe
applications; use `--tests` to select the development default explicitly.

```sh
# Fast operational boot, with a bounded host observation window.
scripts/run-aarch64.sh release --minimal --timeout 120
scripts/run-x86_64.sh release --minimal --timeout 120

# Build boot + data images without starting a VM.
scripts/run-aarch64.sh release --minimal --build-only
scripts/run-x86_64.sh release --minimal --build-only

# Normal development and extended native qualification.
scripts/run-aarch64.sh debug --tests --timeout 180
scripts/run-x86_64.sh release --tests --security-test --http-test --timeout 300
```

`--help` lists runner options. Networking, storage, naming, grant control,
observation and the deployment plane retain their normal operational launch
policy. `--no-network` remains a runtime option. Minimal does not mean a
kernel-only image, or a production trust configuration: protected trust/key
provisioning remains disabled, independently of test selection. Management
listeners still require an explicit valid TLS resource; missing credentials
leave them disabled. [Management startup observation](management-tls.md#execution-qualification-and-remaining-gates)
can also qualify a minimal boot when its resource and reader credentials are
provided. Positive startup expectations reject missing input files before build.

Raw telemetry disk archiving is disabled by default in both modes. Live
observability, bounded memory history and fresh placement reports remain active.
Set `CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS=60000` to opt into diagnostic archiving
when building; changing it requires rebuild/restart and cached kernels must
match. See [telemetry policy](../guides/configuration.md#diagnostic-telemetry-persistence).

## Network verifier boundaries

`--dhcp-test` observes the operational TCP/IP service's status reply and requires
both its expected status magic and a nonzero interface address. Serving stage 6
alone does not prove a lease. The registered DHCP verdict concerns that address;
it does not require management credentials or establish listener readiness.
S3 and Kafka fixtures can therefore select DHCP verification with management
ingress intentionally disabled.

DHCP, listener readiness and management admission use separate observers:

| Assertion | Retained observer |
| --- | --- |
| DHCP lease acquired | `el0_dhcp` / `--dhcp-test`, actual `OP_STATUS` address and magic |
| Provisioned HTTP service reaches listening stage | `el0_http` / `--http-test`, then authenticated host requests and response checks |
| Provisioned management endpoints authenticate and enforce reader permissions | `--management-startup-test ready`, real synchronized time, verified observations and mutation denial |
| Missing management credentials disable both ingress services | `--management-startup-test disabled`, explicit service-disabled events and ordered single trust/service publication |
| Invalid signed management resource rejects before publication | `--management-startup-test WrongPolicy`, `Signature` or `Authentication` |

The disabled probe also attempts plaintext and TLS requests. It records a
transport timeout as uncertain; the disabled assertion comes from the explicit
service events, never from silence. A DHCP pass cannot substitute for any of
these management checks. The existing DHCP startup/lease-observation deadlines
and polling remain unchanged; there is no kernel-visible address-change
subscription in the current socket protocol. This does not qualify the raw
fixture's accepted status-call wait as an end-to-end deadline.

The [credential-free campaign evidence](../reports/audits/evidence/2026-11-01-dhcp-verifier.json)
records the affected S3/Kafka repeats and their separate management observations.

## What runs during every boot

Mandatory startup validation lives in `catten::boot_validation`, outside
`self_test` and without a disabling Cargo feature. Before AP admission it:

- Checks that physical memory has available frames and coherent usable/free counts.
- Allocates one unpublished frame, performs volatile readback at both ends,
  releases it and checks the original free-frame count.
- Checks the configured heap reserve and performs one aligned 8 KiB heap
  allocation, volatile write/readback and release.

Hardware discovery, actual IOMMU initialization, signed artifact verification,
resource admission, driver initialization and service readiness/error checks are
operational requirements. They remain active. The operational launcher also
seeds missing/invalid signed bootstrap artifacts before publishing its service
set, retaining valid installed images. A denied, malformed or unreadable result
stops installation rather than authorizing replacement. First-boot installation never depends on
the NVMe verifier. Regression tests that deliberately
reject IOMMU initialization or retain abandoned domains are optional fixtures,
not boot prerequisites. Minimal keeps production assertions, panic diagnostics
and resource owners; it does not replace checks with assumed success.

The minimal completion event is:

```text
BOOT COMPLETE: validation passed; local storage ready; services submitted
```

The existing local-ready publisher emits it after observing storage readiness
and registering the node's boot-done marker. It establishes that boundary, not
that every optional deployment or network peer is healthy. Timed minimal runs
require the complete event plus memory-validation, signed bootstrap seeding and
minimal-mode records;
a panic, truncated event or self-test verdict cannot qualify the run. Test runs
still require the registered coordinator's `SELFTEST COMPLETE` verdict, with no
failed or pending results. Minimal runs never synthesize a zero-test success.

## Compile-time and bundle selection

| Selection | Kernel | Signed bundle |
| --- | --- | --- |
| Runner default / `--tests` | `acpi,kernel_tests` plus selected integration/display/trace features | Operational services plus development applications |
| `--minimal` | `acpi` plus selected display/trace features | Operational services; no demo/client/smoke/security/shutdown probes |
| Raw Cargo defaults | `acpi,display,kernel_tests` | Caller supplies the corresponding architecture bundle |

All integration-test features imply `kernel_tests`. That feature gates the
self-test registry, native test modules and adapters, failure-injection hooks
and state, test-only metrics, IOMMU rejection/recovery fixtures and the topology
printing worker. Host `#[cfg(test)]` logic tests retain their ordinary Cargo
harness independently. Explicit scheduler tracing remains available for runtime
diagnosis; it is not a regression suite.

Build bundles separately with:

```sh
scripts/build-catten-services.sh --embed --mode minimal
scripts/build-catten-services-x86_64.sh --mode minimal
# --mode test is the default for both builders.
```

`scripts/lib/service-bundle.sh` owns the architecture-specific operational and
probe inventories. Builders select explicit bins and replace the staged bundle
from a temporary directory, so an earlier test ELF cannot survive minimal
staging. Minimal bundles use `target/embedded-services/<arch>-unknown-none-minimal`;
development bundles retain their existing paths. The AArch64 Sitas demonstration
is built only for test bundles. AArch64 still needs its data image preseeded
with non-embedded runtime services; blank-disk self-seeding is the x86 path. Both architectures gate embedded test images in
the kernel as well as excluding them from the preseeded data disk.

The standalone in-memory Raft server and echo-upgrade service manager are
development fixture servers and belong to test bundles. DNS retains its durable
Raft runtime in minimal builds. Batch signing builds the pinned host signer once,
then starts a separate signing process for each ELF, preserving per-process key
ownership while avoiding repeated Cargo startup.

For a direct headless kernel build, export `CATTEN_AARCH64_SERVICE_BUNDLE` or
`CATTEN_X86_64_SERVICE_BUNDLE` to the matching absolute bundle path and use
`--no-default-features --features acpi`. Add `kernel_tests` for native regressions.
The runners handle these details, signatures and image creation automatically.

Minimal instances receive a `-minimal` suffix, separating their images/logs
from development runs. `--reuse-storage` is rejected for minimal runs: use its
isolated disk, or regenerate it with `--fresh-storage`. A minimal build does not
scrub an arbitrary previously installed disk. The bundle inventory validator
rejects missing runtime services and unexpected probe files even when embedding
builds are skipped. `check-boot-profile.py` checks the compiled mode and scans
minimal ELF symbols/fixture markers before image creation, including cached
kernel reuse. This mode check does not replace rebuilding after source or other
configuration changes. Build mode is not inferred from an optimization profile or filename.

The supported minimal native paths are AArch64 QEMU with SMMUv3 and x86 QEMU
with VT-d/AMD-Vi. HVF remains a reduced development fixture and rejects
`--minimal`; it cannot supply the protected-DMA storage prerequisites. Physical
hardware qualification remains separate. CI builds/lints both kernel modes and
boots minimal and regression configurations on both architectures.

## Retired development exercises

| Retired exercise | Retained coverage |
| --- | --- |
| Unregistered kernel async/cross-LP demo and unused infinite logging thread | Registered EL0 cross-LP completion, CQ wait/wake, scheduler lifecycle and synchronous mailbox/queue checks. These have asserted outcomes and coordinator deadlines; the old demo's log-only success was not a reliable verdict. |
| Separate physical-frame readback test | Mandatory startup frame/readback/accounting validation, also present in minimal builds. |
| Verbose 1050-byte and 8 KiB allocator exercises | One mandatory aligned, multi-page heap readback; native retirement/failure controls remain in the regression suite. |
| Native copies of elementary adaptive-stack policy assertions | The broader `charlotte-lifecycle` host boundary tests and native stack/admission/history tests. |
| First-boot seeding inside the NVMe verifier | Always-on operational launcher seeding, before steady-state publication; the verifier observes that initialized storage stack. |
| Unused combined `run_self_tests` wrapper | Explicit synchronous and scheduler-owned deferred phases, preserving their distinct execution contexts. |

Low-level admission, cancellation, ownership-retention and unsafe-control tests
remain even when a broader integration test also succeeds: their rejection and
abandonment discriminators are distinct. Further retirement needs a concrete
coverage mapping, including failure outcomes, rather than overlapping success
messages or similar filenames. Historical audit evidence remains unchanged.

The [artifact and execution record](../reports/audits/evidence/2026-10-31-boot-modes.json)
includes both native modes, blank-disk installation/reboot, the real pre-seeding
negative control, and separate failed host-fixture/signing attempts.
