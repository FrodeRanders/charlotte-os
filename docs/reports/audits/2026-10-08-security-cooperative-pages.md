# Cooperative shutdown runtime-page ownership — 2026-10-08

This batch advances **SEC-18 / R18-2** from the reconciled
[remediation ledger](../../reference/security-remediation.md). It does not close
SEC-18 or change the finding totals: **20 corrected in scope, six partial and
four open**. Protected persistent policy state and recipient-key custody remain
unimplemented; production admission remains disabled.

## Finding and implementation

Cooperative node/device/deployment drain requests and acknowledgement reads used
copied config/status physical frames without retaining their captured root.
Deployment cooperative access also ran beneath its registry guard. A copied
`ServiceDomain` identifies an occupancy but does not itself keep image backing
alive, so delayed access could outlive retirement or touch reused physical pages.
The node's local-readiness publisher had the same status-pointer lifetime gap.

`bootstrap::with_service_pages` now admits the exact root outside subsystem
serialization. A borrowed `ServicePages` validates the handle/ASID and both fixed
runtime leaf mappings against the loader's captured frames before exposing
bounded request/status methods. No pointer or physical frame escapes that view.
These pages belong to the root's image backing and cannot be unmapped through
application memory/MMIO operations. Its admitted operation keeps the root and
image backing alive until explicit completion. Mapping-validation rejection
also finishes that operation; abandoned access retains it and its charges.

Deployment retirement retains its admitted entry's `Polling` claim and releases
the registry guard before cooperative access, force publication or close. A
competing retire caller stays pending. Deadline shortening remains bounded by
the existing signed grace and enclosing node deadline; an immediately forced
request publishes force directly. Status is cached before staged teardown.
Cooperative page rejection becomes a cached `ServicePagesUnavailable` result,
retaining the entry/phase and withholding acknowledgement/force counters, device
transfer and poweroff progress. Node Drop does not retry a cached page failure.

Force publication receives and borrows the abort sweep's existing root lease,
qualifies its runtime pages, and may reject. It does not reopen operation
admission after a concurrent staged close. Ordinary rejection releases the sweep
lease but keeps its terminal thread-admission fence; abandonment retains both.
Actual thread quiescence and physical reclamation remain separate operations.
The readiness publisher takes one admitted object-store status snapshot before
yielding. Raw post-spawn lifecycle frame helpers are private to the bootstrap
adapter; the existing ELF/runtime initialization boundary remains unchanged.

Contract: [supervisor runtime-page access](../../reference/live-address-space-operations.md#supervisor-runtime-page-access).

## Ownership probes

The serialized guest admission suite adds real roots with two admitted image
pages and checks:

- Requests/status run after lifecycle/table admission guards leave. Immediate
  close rejects with a live access, and ordinary completion permits teardown.
- Staged close rejects new access before its callback. An older borrowed lease
  still accesses its own pages, keeps close pending and permits final release
  only after explicit completion. Successful teardown restores frame/image totals.
- Wrong config/status frames, ASID mismatch and a foreign borrowed root reject
  without invoking the callback or leaking an ordinary operation lease.
- Force-publication rejection writes no request, releases its operation lease
  and retains its terminal thread-admission fence.
- A stale handle paired with the successor's actual frame addresses still
  rejects. Repeated node/device/deployment polls cache failure without changing
  successor request/status bytes, counters, registry admission or device gating.
  Existing thread-abort probes repeat cooperative rejection while a successor
  thread remains live at the reused ASID; closing-root probes reject too.
- Abandoning a validated root operation retains its two image charges and keeps
  close busy. Later bounded access does not discharge the abandoned count.
  This fixture deliberately retains backing; it supplies no recovery bypass.

These are controlled real-root interleavings, not physical-platform qualification
or an exhaustive adversarial concurrency proof. Existing EL0 shutdown tests
exercise cooperative acknowledgement, signed/enclosing deadlines, forced abort,
reverse dependency phases and device-quiescence gating.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, default and `shutdown_test`, `--locked`, `-D warnings`: passed. |
| Host regression suite | `scripts/run-host-tests.sh`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d default QEMU | **15 passed, 0 failed, 0 pending**; new page-owner marker present. |
| Four-LP AMD-Vi default QEMU | **15 passed, 0 failed, 0 pending**; new page-owner marker present. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; new marker, scoped probes `0xffff`, generations 1/2, 4,804 cancellation requests. |
| Focused shutdown QEMU | **Five final runs passed**: Intel/AMD without networking, Intel/AMD with networking, Arm with networking. Each authoritative result is **1 passed, 0 failed, 0 pending**. |

The initial x86 `--no-network --shutdown-test` runs reached cooperative/forced
cleanup and storage retirement, then failed the fixture's unconditional
all-network-phase assertion; neither run was accepted as a pass. That fixture
previously assumed all eleven service phases and at least two device adapters.
It now captures the actually launched service set before shutdown consumes it,
requires exactly one acknowledgement for each present phase, zero for absent
phases, no unacknowledged/forced outcome, and the exact retained device count.
Durable storage remains mandatory. Final storage-only x86 runs retain one device;
full-network x86 runs exercise all eleven phases and two adapters; Arm exercises
all eleven phases and three adapters, followed by the PSCI terminal poweroff
marker. Thus optional-platform coverage does not weaken present-phase checks.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance service-pages-intel-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance service-pages-amd-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18513 \
CATTEN_DEPLOY_HOST_PORT=17813 scripts/run-aarch64.sh --security-test \
  --instance service-pages-arm-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --shutdown-test \
  --instance service-pages-shutdown-intel-final-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --shutdown-test \
  --instance service-pages-shutdown-amd-final-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18516 CATTEN_DEPLOY_HOST_PORT=17816 \
scripts/run-x86_64.sh --shutdown-test \
  --instance service-pages-shutdown-intel-network-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18517 CATTEN_DEPLOY_HOST_PORT=17817 \
scripts/run-x86_64.sh --iommu amd --shutdown-test \
  --instance service-pages-shutdown-amd-network-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18515 CATTEN_DEPLOY_HOST_PORT=17815 \
scripts/run-aarch64.sh --shutdown-test \
  --instance service-pages-shutdown-arm-final-20261008 --fresh-storage --timeout 180
```

Runners rebuilt kernels and enforced executable/read-only assembly sections.
Existing validated embedded service bundles were reused for this kernel-only
change. Networked QEMU required local forwarding-port permission.

## Remaining scope

R18-1's full locking/destructor/IRQ-state inventory, R18-3's bounded retained-owner
recovery registry, broader active-I/O/reset coverage, pressure/stress evidence and
physical qualification remain open. This adapter neither clears abandoned
counts nor authorizes reuse after uncertain physical cleanup. Shared control-page
wire semantics and application status diagnostics are unchanged; lifetime
retention does not turn a service acknowledgement into hardware completion.
