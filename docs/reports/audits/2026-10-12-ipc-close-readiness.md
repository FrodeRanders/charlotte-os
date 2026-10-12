# Exact IPC close-claim readiness — 2026-10-12

Scope: SEC-18, existing cleanup families C07/C08/C09, C16 fixture qualification,
and gates G1/G2/G4/G7.
Baseline: `bd0231e6`. This follows the
[weekly wait sweep](2026-10-12-wait-readiness-sweep.md) and
[transfer-close readiness](2026-10-12-memory-close-readiness.md).
Current contracts: [IPC records](../../reference/ipc-record-budgets.md),
[cleanup/recovery](../../reference/cleanup-recovery.md), and
[locking](../../reference/locking.md).

## Finding and implemented boundary

Explicit IPC close released IPC but repeatedly yielded while another reply,
cancellation or endpoint close held a claim. Owned endpoint close also yielded
on a competing cancellation of its queue front. These threads remained runnable
without a claim-release subscription. Reply-result or endpoint-readable interest
cannot substitute: rollback and failed cleanup need not produce either result.

A contended explicit close now retains its captured root, capability payload and
one active claim interest through the existing close loop. It prepares a charged
observer list and typed disposal node outside IPC/lifecycle, using classification
from the claim's original admitted record. Relinking revalidates the root,
capability, token/endpoint and checked claim epoch. Each publication increments
that epoch; exhaustion rejects before loan preparation or claim mutation. A
rollback followed by another claim on the same token is a different episode,
and cannot capture the old preparation. Losing storage is disposed explicitly
after unlock. Kernel-root identity needs no reusable-slot operation lease.

Scheduler registration enters only the independent list, with closed-source
rechecks before/after registration. There is no production timer or yield
fallback. Rejected source or waiter admission returns ResourceLimit without
consuming authority, caller borrow or uncertain loan backing. The complete wait
owner retains its lease and interest behind ManuallyDrop on abandonment;
ordinary success/error explicitly disposes its interest and completes the lease.

Owned endpoint-front waiting borrows the endpoint closer's existing server
lease. Its transient interest remains with that owner behind ManuallyDrop;
ordinary completion/rejection disposes it after IPC. It never acquires a new
server lease after endpoint-close claim publication or snapshots the queue.
Other explicit closes subscribe to that endpoint-close claim's exact epoch.

Reply success and ordinary rollback, cancellation success and cleanup failure,
and endpoint-close success/rejection close the exact claim source under IPC.
The existing producer-LP deferred transport retains detached waiter nodes and
the admitted source-disposal node until producer guards leave. Registration and
source closure invoke no scheduler callbacks inline; notification and final
source allocation destruction occur outside local IPC/list/transport guards.
The transport preserves enclosing IRQ state and owns metadata only.

Claim-source fallback retains both admitted allocations without callbacks,
deallocation or physical cleanup. Abandoned claims remain fenced and open;
waiting does not recover them. A failure wake permits revalidation: cancellation
still rejects cleanup_failed tokens and publishes no terminal call result;
uncertain loan pins/fences are never reconstructed or retried. Readiness is not
delivery observation, hardware completion, charge refund or recovery authority.
Serialized namespace/fixture close returns Pending on an unexpected claim,
without parking under lifecycle or admitting a fresh live root. Namespace drain
progress still belongs to the containing ClosingAddressSpace owner.

This adds no custody registry, cleanup family, physical retry owner or ABI.
Broader metadata field destruction and outer IRQ/cleanup contexts retain their
existing qualification limits.

## Targeted execution evidence

Existing real loan/source fixtures now observe claim readiness across call/reply
close, endpoint/connection minting-source close, successful reply/cancellation,
reply-publication rollback, rejected loan cleanup and endpoint-close rejection.
Callbacks require IPC, lifecycle and master thread-table guards to be available.
They remain deferred until dispatch; a late registration on the closed source
is immediately ready and creates no new waiter. Detached charges return after
notification and explicit interest completion.

Additional fixtures fill all 64 source entries, reject another registration,
cancel every token, reject source/wait admission, and complete a claim between
preparation and relinking. They require original list/waiter charge recovery,
unchanged authority on rejection and successful root close. A valid loan-bearing
reply restores its returned-memory escrow after injected publication rejection,
then immediately claims the same token again. Old preparation must reject that
successor epoch; the successor explicitly completes before fixture-root close.
These are deterministic ordering/admission adapters, not real OOM or exhaustive
race stress.

Scheduled fixtures require actual call and owned endpoint-front closer threads
to be Blocked with one exact cancellation subscription. The parent verifies
root close returns OperationsInFlight, then completes the competing claim.
Watched workers must finish and all roots must close; the queued loan must no
longer be lent. Parent sampling uses a one-millisecond fixture-only sleep with
a five-second deadline. Production waiting is event-only. Exit watches use an
opened private completion namespace, explicitly closed after each case.

Assertion-backed diagnostics expected in every final guest:

```text
[IPC claim readiness] preparation rejects a successor episode on the same reply token
[IPC claim readiness] exact-root wait, late closed bind, saturation, admission rejection and completion-before-relink passed
[IPC claim readiness] scheduled call and owned endpoint closers parked on exact cancellation claims; completion woke both and their roots closed
```

No additional abandoned roots, uncertain loan backing or stack pairs are
introduced by these passing fixtures. Existing intentional failure/abandonment
fixtures retain their original charges. Structural fallback retention is not
kernel-unwinding or abandoned-operation recovery evidence.

## Failed fixture runs retained separately

The initial ARM and Intel guests panicked before the new claim-epoch assertion.
The test constructed the existing vector-call fixture with zero entries; that
ABI rejects an empty vector, so submission returned MemoryTransferFailed:

```text
Kernel panic:
panicked at crates/catten/src/ipc/reply/tests.rs:48:71:
called Result::unwrap() on an Err value: MemoryTransferFailed
```

Both initial runners exited 1: ARM reported no authoritative result within
180 seconds; Intel reported that the guest HTTP keyhole was not validated.
ARM's last timestamp before panic was 3.831758 s; Intel's was 4.737162 s.
Neither reached an authoritative successful deferred self-test result or the
scheduled readiness fixture. Their immutable boot artifacts were:

| Failed artifact | SHA-256 |
| --- | --- |
| ARM initial kernel | `b38ce0e6c01140a731a87a3310d6c92fc616ea8d626f820fc4230139f187e7ca` |
| Intel initial kernel | `08c876fc36065450006b8e7a253646c4f2860553988c515336977d348d847b1c` |

Correction: use one valid mapped loan and explicitly confirm its cleanup before
the injected returned-memory publication failure. Reclaim the same token only
after rollback, then confirm successor completion. This fixes the fixture and
does not attribute historical startup/storage/shutdown failures or prove physical
cleanup in either failed guest.

### Intermediate passes and shutdown contention failure

After correcting the vector fixture, ARM security/HTTP/DHCP passed 21/0/0 at
65.233609 s (`0x2123ffff`), Intel passed 18/0/0 at 29.321803 s and AMD passed
18/0/0 at 25.278853 s (both `0x123bbff`). Their kernels were
`ac05cee0b541468060797519a1089950ea942dd9012f66a3dcb0dff46d25632a`
and `05b03ef841c99e0e30d369381e9bf7808e8bbb81c674177d6ef5688b0b811c08`.
All three runners exited 0 and emitted the scheduled claim-readiness diagnostic.

The first shutdown-profile guest instead panicked in the pre-existing deliberate
remote-lifecycle contention fixture, before scheduled claim readiness:

```text
panicked at crates/catten/src/cpu/scheduler/system_scheduler/domain_abort/tests.rs:298:18:
self-test deadline expired while waiting for remote lifecycle ownership observation
```

Its last timestamp was 6.643908 s. The immutable shutdown kernel was
`21249e9ec1bcb2e6ffed5f854c6e7dbed723c6df6ff5fe3d480a83a827402af7`.
The runner exited 1 with no authoritative result within 180 seconds.
No successful deferred result, domain drain or power-off was observed in that
run. Serialized claim-readiness assertions had passed. The capture supplies the
remote worker's panic backtrace, not the observer's blocked stack.

Source review identifies a circular-wait risk in the fixture: the worker holds
lifecycle while awaiting observer acknowledgement; the observer yields before
checking the phase, allowing another task on its LP to block on lifecycle and
prevent acknowledgement. Preemption creates the same risk. This is a supported
explanation of the risk, not proof of that exact interleaving in the failed run.

Correction: mask the observing LP before publishing the remote worker and keep
it executing through the existing five-second bounded phase handshake. The
fixture polls atomics with spin_loop deliberately, restores the original IRQ
state after remote release, and adds no production polling path. Masking only
after phase 1 leaves the race open. This qualifies the existing ownership probe;
it does not change abort, physical release or production resource-wait policy.
The complete final guest matrix was rebuilt/rerun after this correction, and the
failed shutdown is retained separately from passing repeats.

### Masked preparation rejection

The first handshake correction passed ARM security/HTTP/DHCP (21/0/0,
`0x2123ffff`, 56.969166 s; kernel
`a4feeb997e9589e4756bc5d228d93166ff6f6512723dfa1e26fd6d687c1a38dc`).
An intermediate ARM shutdown build also passed 19/0/0 at 10.494034 s,
`0x103ffff`, drained children at 13.789189 s and requested power-off at
13.852497 s; its kernel was
`d81b7a430314cc7ce546d9166abd0a2a06fa5d9e8f2b4deff198e0615f1b310c`.
Both runners exited 0. These precede the final stable-source matrix below.

Intel instead rejected mandatory thread construction while the fixture prepared
its remote worker beneath the newly added mask:

```text
panicked at crates/catten/src/cpu/scheduler/threads/mod.rs:595:42:
mandatory thread construction failed: ThreadPreparationFailed
```

Its last timestamp was 16.927502 s; the kernel was
`a41ca59fadb790bc4287a16c308ba6a398293c90910d4821eb91a3b3b52c0fdd`.
Symbolization against that exact hash places the caller in
`cpu::scheduler::spawn_thread_on_lp`, inlined from the user-isolation verifier's
lifecycle ownership fixture. The captured error does not identify the underlying
constructor error variant; it is not evidence of kernel heap exhaustion or a
production IPC claim failure. The guest reached no successful deferred result; its runner exited 1 because
the guest HTTP keyhole was not validated.

Final correction: construct the complete kernel thread's stack/context owner
before masking. Consume it through existing thread publication and exact-LP
submission under the short handshake mask; do not publish the worker before
pinning its observer. This preserves the existing physical preparation's IRQ
requirements without reopening the acknowledgement race. Every final profile
is rerun after this source change; passing intermediate ARM runs do not substitute
for final Intel execution.

### Final-source Intel DMA startup failure remains unresolved

The first Intel boot of the final source panicked before the scheduled ownership
and readiness fixtures, while launching the NIC driver after requester `0x20`
was discovered:

```text
panicked at crates/catten/src/service/supervisor.rs:1195:14:
[supervisor] DMA-domain grant failed: DmaUnavailable
```

The last pre-panic timestamp was 15.579220 s. Kernel SHA-256:
`0ab5e61c44b2834ae9feabf75e8807547fca74848f1d4091788a5e2890b9f63b`.
Exact-hash symbolization identifies `spawn_driver_with_name_service` from
`launch_steady_state_with_trust` and `finish_boot`. Real DMA cleanup/configuration
fixtures had passed, and serialized claim readiness had passed; the scheduled
claim test and authoritative deferred result were not reached in this run.

`device::create_dma_domain` collapses both stream resolution and backend/reset
errors into DmaUnavailable. The capture therefore does not distinguish busy
unit/engine admission, reset rejection, backing pressure or hardware error.
Its runner exited 1 because the guest HTTP keyhole was not validated.
No causal attribution to IPC readiness, the contention fixture, QEMU load or
memory exhaustion is established. Backend failure diagnostics and startup
handling remain an audit gap; no uncertain creation is replayed or force-cleared.

A fresh-storage repeat used the identical verified kernel hash with kernel build
skipped, and passed 18/0/0 at 18.118052 s, including the scheduled readiness and
lifecycle ownership diagnostics. A passing repeat does not resolve the failed
startup or establish reliable production progress. That failure is separate from
the fixture errors corrected above and remains open in the living ledger.

## Final validation and artifacts

Both service bundles were rebuilt before guests with
`scripts/build-catten-services.sh --embed` and
`scripts/build-catten-services-x86_64.sh`. Host suite: **384 passed, 0 failed,
3 ignored**. Formatting and diff checks passed. Strict `-D warnings` Clippy
passed for bundled services and final-source ARM security/HTTP/DHCP and x86
HTTP/DHCP kernels.

Final stable-source fresh-storage QEMU guests used a 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Guest completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 55.703354 s |
| x86 VT-d, HTTP + DHCP, identical-artifact repeat | 18 / 0 / 0 | `0x123bbff` | 18.118052 s |
| x86 AMD-Vi, same kernel/features | 18 / 0 / 0 | `0x123bbff` | 20.644573 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.605173 s |

All four final passing runners exited 0 with zero failed/pending bitmaps and all
three claim-readiness diagnostics. Synchronous assertions are additional to these
deferred counts. The lifecycle ownership probe also passed in all four guests;
shutdown drained child domains at 14.463333 s and requested PSCI power-off at
14.517370 s. The failed same-artifact Intel startup remains separate evidence.

Commands used `CATTEN_SKIP_EMBED_BUILD=1` after rebuilding both bundles:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance ipc-claim-arm-v2
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance ipc-claim-intel-v2-repeat
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance ipc-claim-amd-v2
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance ipc-claim-shutdown-v2
```

HTTP/deployment ports were 19308/18608, 19310/18610, 19311/18611 and
19312/18612. Intel repeat and AMD used `CATTEN_SKIP_KERNEL_BUILD=1` only after
checking the current ELF hash against the immutable captured final-source Intel
boot hash. Per-boot hashes precede later profile rebuilds:

| Artifact | SHA-256 |
| --- | --- |
| ARM final combined kernel | `ffbd7419c2f7603d6f6fffd62c845c3d012c4f4141dd44ef4e8f7519f409b413` |
| Intel and AMD final kernel | `0ab5e61c44b2834ae9feabf75e8807547fca74848f1d4091788a5e2890b9f63b` |
| ARM final shutdown kernel | `cb6ba84d5d53d26392925b35d8a4ac2b11c2f3969b7565a6d77f07a02786c62e` |
| ARM bundled service ELFs | `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621` |
| x86 bundled service ELFs | `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256, matching the runner convention. Published evidence above is inline;
temporary serial/runner logs and ignored build files are not report dependencies.

## Remaining limits

Explicit IPC claim waits are event-driven. Namespace, operation-lease and other
pin drains, x86 shootdown coordination, general metadata/principal admission,
outer destructor/IRQ contexts, and integrated pressure/concurrency qualification
remain open. The transport supplies no root/backing custody, failed receipt
retry or terminal call result. This closes the narrow wait-notification gap,
not C07/C08/C09, G1/G2/G4/G7, SEC-07 or SEC-18 as a whole.
