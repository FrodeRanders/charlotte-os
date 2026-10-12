# Thread metadata fallback retention — 2026-10-12

Scope: SEC-18, existing cleanup family C16 and gap G1. Source baseline:
`488e032b3b8a0c4bc275cda851e97721ff4fa5eb`. This is scoped progress, not
closure of SEC-18 or the full call-chain audit. Current contracts are in
[cleanup/recovery](../../reference/cleanup-recovery.md) and
[thread retirement](../../reference/thread-retirement.md).

## Finding and correction

`Thread::drop` still called `finish_retirement_metadata`: it canceled blocked
waker registrations, updated user-thread usage and invoked exit observers
under whichever enclosing guards held the owner. Implicit field destruction
then freed the stable context Box and other metadata. Earlier stack/context
changes prevented physical cleanup there, but did not qualify this metadata
fallback. Source inspection establishes that path; no production deadlock is
attributed to it here.

The complete thread payload is now inline behind `ManuallyDrop`. Unexpected
Drop only increments an atomic diagnostic and retains the context, exact
stack/root admission, waiter/waker state, exit subscriptions, sponsors and
unused retirement node. It performs no callbacks, cancellation, accounting,
guard acquisition, heap/physical cleanup or logging. Stable context boxing and
assembly context offsets remain intact; this adds no allocation or registry.

Explicit metadata notification still precedes stack/root completion, retaining
the root through numeric-ASID usage accounting. Consuming metadata destruction
requires notification completion and successful release of the exact stack
pair. It disarms its fallback before field destruction. The shared typed
`RetiredEntry::release_with` retains the existing node until disposal returns;
only then is node backing freed. Never-admitted ordinary rejection uses the
same disposal after successful pair release and local guard exit.

Physical rejection retains the existing complete node and one-shot fence;
later scans never retry it. Interruption/abandonment is terminal retention,
not Pending or an owner-preserving retry. The abandonment counter carries no
address, ticket or authority, and no recovery path re-adopts backing by address.

## Targeted evidence

- Host success verifies payload disposal before node deallocation, exactly one
  payload destruction and no allocation during release.
- Host interruption uses `catch_unwind` with a deliberately retaining typed
  fixture payload. A node-specific deallocation tracer confirms that its exact
  node remains allocated. This tests node custody, not kernel panic recovery.
- Serialized guest fixtures abandon never-admitted kernel and user threads
  under simultaneous master-thread, retirement, address-space and physical
  allocator guards. Exit observers remain uncalled; waiter/source and watch
  charges remain live; usage and free-frame counts remain unchanged. External
  watch/source cancellation is checked separately and cannot recover the
  thread. The user root rejects close with `OperationsInFlight` and retains its
  one-thread admission. Two complete payloads and that root are intentionally
  retained. The atomic counter advances by two.
- A selected-generation success probe checks metadata/pair completion and
  availability of those four guards immediately before explicit disposal.
  Existing fixtures verify complete frame recovery, current-SP/Arm `on_cpu`
  deferral, unlocked exit callbacks, masked-entry rejection and terminal
  failed-pair retention through two scans without physical retry.

All four guests printed this assertion-backed diagnostic:

```text
[thread metadata retention] kernel/user payload Drop under table/retirement/root/allocator guards: no exit, waiter cancellation, usage refund or frame return; exact user root remains busy
```

The structural `ManuallyDrop` contract prevents implicit field destruction;
free-frame measurements alone do not measure all kernel heap deallocations.
The selected-generation guard probe is confined to serialized boot fixtures;
it must not interpret unrelated concurrent-LP lock ownership as a production
disposal failure.

## Validation and artifact identity

Both bundled architectures were rebuilt before guest execution using
`scripts/build-catten-services.sh --embed` and
`scripts/build-catten-services-x86_64.sh`. Host tests:
`scripts/run-host-tests.sh`: **384 passed, 0 failed, 3 ignored**.
`cargo fmt --all -- --check`, `git diff --check`, and strict `-D warnings`
Clippy passed for bundled services, ARM security/HTTP/DHCP kernel and x86
HTTP/DHCP kernel.

Initial kernel Clippy invocations incorrectly pointed both bundle environment
variables at nonexistent `target/catten-services/.../catten-services.bundle`
directories. They failed on missing embedded ELF files (ARM 11 compiler errors,
x86 33). Corrected runs used the rebuilt `target/embedded-services` directories
and ARM's generated `CATTEN_SECURITY_TEST_DIR`; both passed. These are
validation-command errors, not observed guest failures. No failed guest run
occurred in this change's validation.

Fresh-storage QEMU runs used a 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Guest completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 57.509763 s |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 19.810850 s |
| x86 AMD-Vi, same kernel and features | 18 / 0 / 0 | `0x123bbff` | 22.885587 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.795868 s |

Every runner returned success and zero failed/pending bitmaps. Boot fixtures
are synchronous and additional to these registered deferred-test counts.
Existing remote-abort, scheduler-lifecycle and exit-watch checks remained
enabled. Shutdown also reached PSCI power-off after the drain checks.

Runner commands used `CATTEN_SKIP_EMBED_BUILD=1` after rebuilding both bundles:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance thread-metadata-arm
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance thread-metadata-intel
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance thread-metadata-amd
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance thread-metadata-shutdown
```

HTTP/deployment host ports were respectively 19280/18580, 19281/18581,
19282/18582 and 19283/18583. AMD reused the exact successfully booted Intel
kernel with `CATTEN_SKIP_KERNEL_BUILD=1`. Immutable per-boot kernel hashes,
captured before later profile builds, were:

| Artifact | SHA-256 |
| --- | --- |
| ARM combined kernel | `49c86d68228ae053b55477f5e15208987802e7b3f19597e58d2db937a0f389a6` |
| Intel and AMD kernel | `9833fe09c2450f88edc00b9c040fa502f2673f11648e538ccdf23c6c55df015b` |
| ARM shutdown kernel | `7a090adaa7440d76ecf1c344ad8157799987fc6604a902b863eefe8b432fc120` |
| ARM bundled service ELFs | `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621` |
| x86 bundled service ELFs | `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999` |

Bundle digests hash sorted `basename:ELF-sha256` lines, each newline terminated,
using the runner's `catten_boot_bundle_sha256` convention. Services are
unchanged from the preceding CQ follow-up.

## Remaining boundaries

Outer constructor/syscall masks, growth's containing thread-table guard,
arbitrary explicit exit callbacks, general scheduler metadata admission and
the full R18-1 caller/implicit-destructor inventory remain open. Retained
payloads intentionally lose capacity; there is no failed-pair or abandoned
thread recovery adapter. Notification interruption does not establish a
retryable cleanup phase. This fixture does not simulate real allocator
corruption, kernel unwinding or the full cross-LP pressure/concurrency matrix.

The per-LP reaper still polls every millisecond; the
[weekly wait sweep](2026-10-12-wait-readiness-sweep.md) keeps that readiness gap
open. Historical Intel object-store, AMD startup and prior teardown timeout
captures retain their original verification limits and unresolved causation.
Passing this matrix does not retrospectively resolve those failures or close
C16/G1/G2/G7, SEC-07 or SEC-18.
