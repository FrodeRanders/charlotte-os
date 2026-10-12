# Exact transfer-close readiness — 2026-10-12

Scope: SEC-18, existing cleanup families C18/C05/C16 and gates G1/G4/G7.
Baseline: `74f05038e0aa1987dce905c9a7116eea8a509c10`. This follows the
[weekly wait sweep](2026-10-12-wait-readiness-sweep.md) and
[LP-reaper readiness change](2026-10-12-reaper-readiness.md).
Current contracts: [memory retirement](../../reference/memory-object-retirement.md),
[cleanup/recovery](../../reference/cleanup-recovery.md), and
[locking](../../reference/locking.md).

## Finding and implemented boundary

The explicit memory-close loop yielded while its source capability was escrowed
by a `PreparedTransfer`. Inspection narrows the sweep's broader pin/fence
description: this loop waited only for `transfer_in_flight`; ordinary loan,
DMA/copy and mapping pins returned `LendingActive`. Their policy is unchanged.

A contended close now leases the exact source root recorded when that transfer
armed its fence, captures the monotonic object identity, and parks on an owned
transfer interest. It never resolves a reusable ASID under the memory registry.
Charged observer-list storage and a typed disposal node are prepared outside
memory/lifecycle/IPC guards; relinking revalidates the exact object/fence. Losing
preparation is canceled explicitly after unlock. Registration enters only the
independent list beneath the scheduler table. Its completed-source check closes
publication-before-registration; a late bind to a closed source is immediate
readiness, without a new waiter. There is no production timer or yield fallback.

The close owner keeps its exact root and transient interest together behind
`ManuallyDrop`. Ordinary success/error consumes that owner and completes its
lease after local guards; abandonment retains all fields and the root count.
Nonwaiting close retains its existing serialized fixture path without fresh
lifecycle admission. The permanent kernel namespace needs no reusable-slot lease.
Wait/source admission rejection returns `ResourceLimit` without consuming source
authority or pins; it is neither timeout nor cleanup completion.

Transfer cleanup restores/consumes exact escrow, releases its pin and clears
the fence under the existing memory hold. Its original metadata work completes
before closing the interest. That completion can run under an outer IPC guard,
so immediate scheduler notification or final source destruction would be unsafe.
The complete source list/allocation charge and preadmitted disposal node stay
owned through a shared **producer-LP deferred waiter transport**. Source fallback
retains both allocations without cleanup; unused preparation cancels explicitly.

The same transport now delivers reaper waiter batches. It moves already-admitted
nodes without queue growth allocation; appended traversal is bounded by that
source's 64-entry limit. A short IRQ-preserving setup binds enqueue/claim to the
current LP before preemption/migration. Only that producer LP claims after its
guards leave, from yield/idle/post-switch boundaries; a different LP cannot run
notifications while the producer holds IPC. Queue/source/list guards leave
before callbacks, and final source/node disposal follows notification. Dispatch
preserves enclosing IRQ policy, including masked switch entries. Static queue
slots match the existing 256-LP ceiling and contain admitted metadata only.

This transport owns no backing, scratch, root transaction, physical completion
proof or recovery ticket. There is no new cleanup family or recovery registry.
Existing `PreparedTransfer`, escrow, pins and `RetiredMemory` remain authoritative.
Wake readiness does not observe hidden IPC attachments, refund uncertain backing
or authorize physical retry. A successful move still makes old-source close
return `UnknownCapability`; ordinary loan/pin rejection remains nonterminal.

## Targeted execution evidence

Serialized fixtures verify rollback and committed move, exact-root busy-close
rejection during a wait, independent closed-source late registration, preserved
bytes/original sponsor accounting, and notification remaining deferred while IPC
is held. The callback checks availability of IPC, memory, lifecycle and master
thread-table guards. Detached waiter admission persists until dispatch; source
allocation remains in its owning disposal node even when all waiters canceled.

Additional fixtures fill all 64 entries and reject another registration, inject
preparation rejection, and complete the fence between preparation and relinking.
They require explicit unused-source disposal, original list/waiter charge recovery,
unchanged authority on wait rejection, and successful final root close. These are
deterministic ordering/admission adapters, not real OOM or exhaustive race stress.

The scheduled fixture observes a real closer in `Blocked` with one exact source
subscription, rejects source-root close with `OperationsInFlight`, then rolls
back the transfer and requires watched worker completion, original page-account
recovery and successful close of both roots. Parent registration sampling is a
one-millisecond **fixture-only** sleep under a five-second deadline. The production
closer uses event-only registration. Existing IPC returned-memory, hidden-delivery,
loan/source-fence, reaper, exit-watch and remote-abort tests remain enabled.

All four final guests emitted these assertion-backed diagnostics:

```text
[memory close readiness] exact-root wait, rollback/move, closed-source late bind, IPC-deferred notification/list disposal, saturation and preparation race passed
[memory close readiness] scheduled closer parked on the exact transfer; rollback woke it and both roots closed
```

No additional abandoned root or stack pair is introduced by the passing fixtures.
They do not establish arbitrary kernel-operation abandonment recovery or all
outer IRQ/allocator/destructor contexts.

## Failed fixtures preserved separately

The first ARM and Intel guests passed serialized readiness checks but panicked
at the new scheduled fixture's exit-watch submission (`close/tests.rs:210` in
those binaries). The fixture used `KERNEL_ASID` as a completion caller without
opening that completion namespace:

```text
called Result::unwrap() on an Err value: UnknownAddressSpace
```

ARM captured the panic around 9.25 s; its last pre-panic waiting observation was
14 passed / 7 pending (`0x26ffd` / `0x21219002`). The runner returned exit 1:
`authoritative self-test result was not produced within 180s`.
Intel captured the panic around 15.73 s; its last waiting observation was
12 passed / 6 pending (`0x22bfd` / `0x1219002`). Its runner returned exit 1:
`guest HTTP keyhole was not validated from the host`.
Neither run produced an authoritative successful deferred result or verified
the scheduled close wake. Their binaries were:

| Failed artifact | SHA-256 |
| --- | --- |
| ARM initial kernel | `e0ef5044a2f455e3add1b0b3c1979006fc031868a813ce74025d402c7e144b51` |
| Intel initial kernel | `a21144a5c2a1f33377eafb3f25ce17b73e5f267cc3d71b29fb70a10d23209a53` |

Correction: open private fixture completion namespace `0x5d06`, use it for the
exit watch/wait/close, and close its completion/authority metadata afterward.
The child remains a kernel thread; the fixture namespace is not adopted as an
address-space owner. Corrected ARM and Intel runs then passed 21/0/0 and 18/0/0
at 57.340569 s and 18.581723 s respectively, including the scheduled diagnostic.
Their hashes were `4c5491f2b3d29eaabbe2fe3278234bab96056d28ce71aec933c92b0af3fb5c50`
and `ad13bc45da4e3225ad4b5d127a9cc998b7b2f6d9bead8a11baaba8299526009a`.
Final LP-binding masks were added during review; the final matrix below was
rebuilt and rerun after that change. The identified fixture error does not
attribute historical storage/startup/teardown failures or prove cleanup in the
failed guests.

## Final validation and artifacts

Both bundled services were rebuilt before guests with
`scripts/build-catten-services.sh --embed` and
`scripts/build-catten-services-x86_64.sh`. Host suite:
**384 passed, 0 failed, 3 ignored**. Formatting and diff checks passed.
Strict `-D warnings` Clippy passed for bundled services, ARM
security/HTTP/DHCP kernel and x86 HTTP/DHCP kernel.

Fresh-storage final QEMU guests used a 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Guest completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 56.664764 s |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 18.800964 s |
| x86 AMD-Vi, same kernel/features | 18 / 0 / 0 | `0x123bbff` | 21.807758 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.732892 s |

All four runners exited 0 with zero failed/pending bitmaps. Synchronous fixtures
are additional to these deferred counts. Shutdown observed child domains drained
at 14.032316 s, followed by PSCI power-off at 14.101898 s. Routine ingress
EOF/reset/deadline probes recovered; no failed final guest run is hidden by these
passes.

Commands used `CATTEN_SKIP_EMBED_BUILD=1` after rebuilding both bundles:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance memory-close-arm-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance memory-close-intel-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance memory-close-amd-final
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance memory-close-shutdown-final
```

HTTP/deployment ports were 19294/18594, 19295/18595, 19296/18596 and
19297/18597. AMD used `CATTEN_SKIP_KERNEL_BUILD=1` for the exact successfully
booted Intel binary. Immutable per-boot kernel hashes, captured before later
profile builds, and unchanged bundle hashes were:

| Artifact | SHA-256 |
| --- | --- |
| ARM final combined kernel | `2c5e48a82a9af659586725e37e7ac398afca699b68a7a4de93465c0ae04b8af3` |
| Intel and AMD final kernel | `c780f32baea75e08d6f6ce0e411431df182433573195eb77ab7af9ed9520d845` |
| ARM final shutdown kernel | `5d6bf339311575dedb69bce3cd32e23779f93032712a1ed997030cb0a4054245` |
| ARM bundled service ELFs | `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621` |
| x86 bundled service ELFs | `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999` |

Bundle hashes follow the runner convention: sorted newline-terminated
`basename:ELF-sha256` lines hashed with SHA-256.

## Remaining limits

Explicit source-escrow close is now event-driven. Loan, DMA/copy/mapping pins,
namespace/lease/claim drains, IPC close claims and x86 shootdown coordination
retain separate notification/qualification work. `PreparedTransfer`'s existing
rollback/metadata destructor contexts, nonwaiting physical-close outer callers,
general metadata/principal-byte admission and the integrated pressure/concurrency
matrix remain open. Close/source fallback retention is structural; these tests
do not simulate kernel unwinding or recover an abandoned close owner. Passing
episodes do not measure CPU savings or close C05/C18, G1/G4/G7, SEC-07 or SEC-18.
