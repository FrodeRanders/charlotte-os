# Private translation-table admission

Date: 2026-10-06. SEC-07 follow-up after staged-copy/QEMU recovery at `2876ce7d`.
This corrects the renewed audit's concrete sparse private-table pressure path;
SEC-07 remains partial for broader kernel allocation and metadata admission.

## Change

Both owning architecture address spaces embed a separate table account.
Actual private roots and intermediate frames are admitted before physical
allocation: 1,024 table frames per domain, a node pool of one sixteenth of
usable RAM, and ordinary admission limited to three quarters of that pool.
Heap/image/object backing remains independent. Empty linked branches and
partial mapping construction retain their charges until physical teardown.
Mapping the same cached region needs no new admission, even at the ceiling.

`PreparingTable` retains the exclusive original account borrow alongside
its physical owner. Allocation rejection refunds unused admission. Failed
unpublished release and interrupted publication retain the original domain/node
charge; release is armed nonrefundable before the consuming allocator call.
No account block, per-frame ledger allocation, reusable-ASID lookup, scalar
restoration or timeout-based refund is introduced. Borrowed current-root
snapshots reject private branch allocation.

`FrameRelease` retires the table account with heap/image accounts before the
physical walk. Only a completely successful walk permits aggregate refund;
any rejected table/root/data release retains the whole table charge. Successful
root teardown excludes prior quarantined provisional tables. Failed final
invalidation and abandoned root owners retain their exact account and slot.

The trusted ambient loader classifies table admission before initial x86 root
allocation. Late namespace promotion would reject essential root construction
once ordinary table admission was exhausted. Arm classifies before lazy root
creation. Only this kernel policy selects platform reserve/floor access;
artifact roles, manifests, application names and syscalls cannot select it.
Platform private tables may use the physical progress floor within the same
domain and total node ceilings. Shared higher-half kernel preparation retains
its existing scope and is outside this private pool.

Current contracts are in [translation admission](../../reference/translation-admission.md),
[table preparation](../../reference/page-table-preparation.md) and
[table lifetime](../../reference/page-table-lifetime.md).

## Regression evidence

- Real walkers fill a five-table ceiling with one foreign data page, retain a
  sparse partial prefix, reject repeated fresh allocation, perform sixteen
  cached unmap/remap rounds, then complete the prefix after raising a kernel-only
  test limit. Another live root maps the same data independently; dropping the
  first preserves the second alias and exact node charges.
- The public memory-object path repeatedly rejects a sparse map at the ceiling,
  preserves the source object's backing/authority, releases mapping pins/root
  leases, remaps a cached region and permits ordinary object/root close.
- Ordinary-admission pressure leaves real counters and physical backing intact.
  Ordinary root/mapping preparation rejects; trusted platform root and mapping
  succeed from reserved admission, with no ordinary charge. Ordinary admission
  recovers after the adapter restores the original limit. This is counter
  pressure, not a node-wide physical-exhaustion attack.
- Existing construction-prefix fixtures now check real table charges and
  complete successful teardown refunds. Existing root-release failure fixtures
  retain 28 table charges across seven failed walks without adding new physical
  leaks to those fixtures.
- Two additional rejected/abandoned provisional fixtures retain two frames and
  two charges through successful destruction of their original roots. They
  check the original domain ceiling and no refund/retry bypass. Interrupted
  ownership is simulated; actual kernel panic unwinding is not exercised.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**, including all table-account and public-object rejection fixtures. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**, including the same fixtures and operational storage. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; table-account fixtures passed, probes reported `0xffff`, publication generations 1/2, concurrent cancellation retired 4,772 requests. Pressure clients retired 1,429/1,372 requests with time/cycle progress. |

Commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance table-admission-intel-20261006 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance table-admission-amd-20261006 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance table-admission-arm-20261006 --fresh-storage --timeout 180
```

The runners rebuild kernels and validate native assembly section permissions.
Existing validated embedded bundles are reused; no userspace or wire-format
change is included. Arm localhost forwarding uses approved isolated ports.

## Remaining scope

SEC-07 remains partial for shared kernel tables, IOMMU tables, stacks, kernel
heap and general metadata/callback storage. The translation pool is not a
complete RAM ledger, live table compactor or worst-case latency guarantee.
QEMU fixtures do not prove physical-platform races, true many-domain pressure
soak or permanently unresponsive CPU/device recovery. SEC-18 general allocator
work under serialization, broader reset support and abandoned-owner recovery
remain open. Authentication, production provisioning and security-time work
remain separate audit findings. Quarantined backing and charges are retained;
no administrative reclamation API is introduced.
