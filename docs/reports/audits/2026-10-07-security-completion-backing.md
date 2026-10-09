# Completion allocation lifetime admission

Date: 2026-10-07. SEC-07 continuation after terminal kernel-range release at
`2bf05003`. This closes the completion record's weak-only backing gap; broader
metadata and heap admission remain partial.

## Finding and correction

The completion record kept its admission charge in its payload. Dropping the
last strong reference destroyed that charge, while producer or captured weak
references could retain the allocation and control block. Record counts could
therefore return to zero before record backing was freed. Source review and a
controlled regression confirmed the lifetime mismatch; no hostile application
exploit or whole-node exhaustion was demonstrated.

`CompletionRef` and `CompletionWeak` now use an allocation owner carrying the
original record reservation. Strong references retain the payload; weak
references retain its allocation admission after payload destruction. The last
allocator owner refunds only after record backing and its private charge holder
are freed. Every holder clone uses `Arc::into_inner`, including concurrent final
destruction; no weak or bare holder reference escapes the adapter.

Allocator clones share one allocation allowance. They cannot multiply record
allocations or revive a freed allocation using its old reservation. Construction
uses `ChargedAllocator::try_arc`, retaining a preparation owner until fallible
Arc construction and rejected-payload destruction have finished. Allocation
rejection returns unused admission; physical allocation exhaustion is not
manufactured by the tests. The adapter forwards unchanged allocation layouts
and pointers to the global allocator and is restricted to fixed allocations.

Namespace identity, platform classification and original domain/node counters
remain captured in the existing charge. Stale weak destruction cannot refund a
replacement namespace with the same numeric ASID or capability. Submission-slot
and detached-CQ semantics, configured limits and syscall backpressure remain
unchanged. A cancelled timer's producer weak reference can now keep record
admission occupied until event reclamation. This is the actual backing lifetime.
There is no new userspace ABI or protocol format.

Contract: [completion-record admission](../../reference/completion-record-budgets.md).

## Regression evidence

- Guest fixtures close a capability while retaining its strong owner, clone 128
  weak references, destroy the payload, reject weak upgrade and reject another
  submission until the last weak reference is released. Admission then recovers.
- Exact ASID/capability reuse keeps both old weak backing and the successor's
  live record charged. Final old weak destruction refunds only the old account;
  successor state remains usable and its charge unchanged.
- Six host tests execute the actual generic kernel allocation adapter. They
  cover strong/weak lifetime, charge-holder allocation rejection, rejected
  payload destruction before refund, allocation multiplication/revival
  rejection, 64 rounds of concurrent final weak destruction and concurrent
  cloned-allocator admission with exactly one successful allocation.
- Existing producer cancellation, retained results, platform reserve progress,
  CQ replacement, callback and event-watch tests remain enabled. Neither host
  nor guest fixtures establish general heap byte accounting or pressure fairness.

## Validation

| Check | Result |
| --- | --- |
| Host allocation-owner regressions | **6 passed, 0 failed**. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,788 requests. |

Run the host test from outside the repository to avoid the kernel's root Cargo
build-std configuration (or use the existing host-test runner):

```sh
cargo +nightly-2026-07-27 test --locked \
  --manifest-path /path/to/charlotte-os/crates/charlotte-lifecycle/Cargo.toml \
  --test charged_allocator
```

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance completion-backing-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance completion-backing-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance completion-backing-arm-20261007 --fresh-storage --timeout 180
```

Runners rebuild kernels and check assembly section permissions; this kernel-only
change reuses validated embedded services. Arm required permission to bind its
local forwarding ports after sandbox rejection.

## Remaining scope

Record counts cover the completion allocation/control block and its auxiliary
charge holder, not aggregate heap bytes. Independent observer-list control
blocks, arbitrary callback captures, registry storage and other weak-only Arc
allocations still need admission/lifetime review. Existing node/domain limits
do not provide per-principal aggregates or a hostile-workload containment claim.
SEC-07 remains partial.

SEC-18 remains partial for general allocation/destruction under subsystem
guards, physical-platform quiescence, broader reset support and abandoned-owner
recovery. Authentication, production provisioning and security-time findings
remain separate work.
