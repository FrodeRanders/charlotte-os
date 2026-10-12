# DMA grant cancellation admission readiness — 2026-10-12

Scope: SEC-18 / C01, C14, C15, C18 / G1, G2, G3, G4, G7. Baseline: `e0b98b95`.

Scheduled grant cancellation now retains its original complete preparation and
registers interest in the existing unit slot before its first destruction claim.
It parks rather than rejecting ordinary unit/engine contention at that boundary.
Descriptor detachment and the existing hardware/physical cleanup run once after
admission. Failed or interrupted cancellation remains terminal to that owner.
An AMD network startup failure was captured separately below; its identical
artifact repeat passed, and its cause remains unresolved.

## Complete owner and admission boundary

`DmaDestruction::Prepared` borrows the containing grant's `DmaCreation`, retaining
the original domain obligation, exact root operation, capability reservation,
reset claim and partial metadata. The scheduled supervisor grant API selects
that adapter. Existing immediate callbacks remain explicit serialized fixture
adapters, with their rejecting policy; public device operations and whole-domain
cleanup keep their existing contexts.

Fresh creation and pre-destruction cancellation now share `readiness::admit`.
Its charged source/disposal storage, captured root classification, closed-source
registration recheck and producer-LP deferred notifications remain the existing
mechanisms. Its claim callback runs once under backend serialization and returns
a retaining owner before guard exit. Only then does transient interest disposal
occur. A competing claim after a wake can cause another admission episode;
the callback and hardware cleanup are not replayed.

Fresh creation still rejects an armed creation before admission. Cancellation
is a different consuming operation: the same grant's rollback-started fence is
set before its checks/wait and remains set through one destruction. A rejected
wait returns the complete owner with that fence. Later readiness does not permit
another cancellation attempt, release admission or clear endpoint claims.
Drop retains every field without physical work, guards, allocator or callbacks.

All three backends require the actual engine and no detached domain cells before
admission. Under that same hold, the recorded domain must exist and its original
tables must be published and unfrozen. Missing, uncertain, released or frozen
backing is no completion receipt; it rejects before descriptor mutation. Then
the exact domain/engine enters its existing `Maintenance` owner. The previous
detached retirement body supplies the architecture-specific completion/drain,
physical release, pin cleanup and post-guard metadata disposal. No queue/epoch,
domain or scalar backing is reconstructed.

Engine return while another domain remains in physical finalization is still
nonready for this conservative whole-unit admission. Masked or nonscheduled
waiting and source/waiter rejection remain explicit; no enclosing IRQ policy
is force-enabled. Hardware polling, actual completion requirements, requester
reset fencing and release of original charges remain unchanged. The recorded
domain is cleared only after confirmed backend success, before existing explicit
reset cancellation and metadata/root completion.

## Executed ownership and hardware evidence

The serialized private-slot cancellation fixture starts with an armed complete
grant and its original reserved authority. During interest registration, exact
root close rejects with OperationsInFlight and authority usage stays one.
Returning only the engine does not wake while a domain cell is detached.
Completed availability delivers the admitted callback outside slot/device/
lifecycle/scheduler holds, preserving the raw fixture's original IRQ policy.
One cleanup callback runs, then ordinary cancellation refunds authority and
closes that same root. List usage returns to its original baseline. The private
slot and empty payload do not claim hardware quiescence.

The existing failed-grant fixture now injects waiter rejection while its armed
creation owns an admitted source interest. Descriptor work is never entered.
Its second cancellation invocation rejects before another callback. Completing
the source closes the interest but leaves the original domain ID, reservation
and root count retained. Guarded complete-owner Drop retains that closed interest
and its original ordinary list allocation as well. This adds one retained list
allocation to the existing failed root; it adds no abandoned root, registry,
coverage row or physical owner. Existing registered-creation/hardware-failure
fixtures remain enabled through their immediate adapters.

That same exact empty fixture owner is passed to the prepared backend path with
its original unregistered fixture domain ID. UnknownDomain (or Unsupported)
rejects rather than reporting successful cleanup; its ID and reserved authority
remain unchanged. The frozen-backing guard uses the existing table state proof;
this batch does not execute an additional partial physical failure under parked
live-hardware cancellation.

A scheduled kernel worker consumes an armed grant on the same private fixture
slot used by the creation readiness tests. The parent observes one registration,
an actual Blocked thread, zero cleanup work, busy root close and one original
reservation. Availability wakes it; one cleanup is admitted, authority returns
to zero and the exact root closes. The parent's bounded one-millisecond sampling
is fixture observation; production cancellation has no polling timer/watchdog.

The real staged-root-close creation fixture now uses the prepared destruction
adapter on VT-d, AMD-Vi and SMMUv3. It configures a real domain, stages the original
root close, rejects publication before activation, then completes actual backend
rollback/reset and authority refund through the older lease. No fresh root is
acquired, original table charges recover, and that same closing owner completes.
This qualifies ready-path physical integration, not arbitrary parked live-hardware
contention or kernel unwinding.

Assertion-backed diagnostics in every tested artifact include:

```text
[DMA grant cancellation readiness] armed grant kept exact root/reservation through pre-destruction wait; engine-only return did not wake; one admitted cleanup and explicit refund/root completion passed
[DMA grant cancellation readiness] scheduled armed grant parked before descriptor work; original reservation/root retained, one cleanup admitted and exact root closed
[DMA creation closing] real configuration with retained root/reservation; staged close stayed Pending, publication rejected before activation, confirmed post-guard rollback refunded authority and original closing owner completed
```

## Failed AMD network startup, retained separately

The first AMD guest completed the new cancellation assertions and the real
staged-close path, but also captured this startup failure and later deadline:

```text
[+ 17.343831] [launch] driver grant batch prepared before threads: storage=true entropy=false network=true
[+ 17.362002] [EL0 LOG] lp=0 asid=106 [net] startup rejected stage=4 error=DmaDomainMap detail=0x3
[+ 17.811479] [frouter] waiting for serving stage (current=1)
panicked at crates/catten/src/self_test/el0_net.rs:105:22:
self-test deadline expired while waiting for EL0 frame router startup
[+ 47.874025] SELFTEST COMPLETE: passed=15 failed=1 pending=2 passed_bitmap=0x1bbff failed_bitmap=0x20000 pending_bitmap=0x1200000
```

Runner exit: **1**, `guest HTTP keyhole was not validated from the host`.
Net failed; HTTP and DHCP remained pending. NVMe reached stage 18 and completed
its storage checks. The raw backtrace, resolved against the matching immutable
symbol sidecar/kernel hash, is:

| Return address | Resolved symbol |
| --- | --- |
| `0xffffffff8007500e` | `__rustc::rust_begin_unwind+0x4e` |
| `0xffffffff802cfaab` | `core::panicking::assert_failed::<usize, usize>` |
| `0xffffffff80059df6` | `catten::self_test::el0_net::verify_el0_net+0x786` |
| `0xffffffff8000664c` | `kernel_thread_trampoline+0x8` |

The captured source diagnostic is narrower than the later router deadline.
`net::alloc_dma` records `pages` as detail when the DMA-map syscall returns
zero, so `0x3` means a rejected three-page mapping. It does not identify a queue
index or kernel error. The current map ABI collapses causes into zero. Contention,
mapping/admission rejection and hardware failure cannot be distinguished from
this capture; no cause is inferred from the timeout or passing repeat.

An identical-artifact fresh-storage AMD repeat passed 18/0/0. The failed run is
not superseded by that pass. This is a public DMA-map/startup progress issue;
there is no evidence identifying the new cancellation path as its cause, nor
evidence that this correction fixes it. It is distinct from the older Intel
grant failure retained in the [IPC readiness report](2026-10-12-ipc-close-readiness.md).

## Validation and artifacts

Both signed bundles were rebuilt before guests. Host suite: **389 passed,
0 failed, 3 ignored**. Strict `-D warnings` Clippy passed for bundled services
and both final-source kernel profiles. Formatting, diff and changed Markdown-link
checks passed. Fresh-storage guests used the 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Completion time | Runner |
| --- | --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 57.034258 s | 0 |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 19.101324 s | 0 |
| x86 AMD-Vi, first run | 15 / 1 / 2 | passed `0x1bbff`, failed `0x20000`, pending `0x1200000` | 47.874025 s | 1 |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.837540 s | 0 |
| x86 AMD-Vi, identical-artifact repeat | 18 / 0 / 0 | `0x123bbff` | 18.379689 s | 0 |

The four passing runners validated their ingress controls; shutdown requested
PSCI power-off at 14.682696 s. All five guests passed the new synchronous and
scheduled cancellation assertions; the first AMD guest failed later network
progress. Deferred totals include the scheduled CQ-wait fixture; synchronous
assertions are additional. These samples are not a pressure-soak/reliability claim.

Commands used `CATTEN_SKIP_EMBED_BUILD=1` after both bundle builds:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance dma-cancel-arm
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-cancel-intel
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance dma-cancel-amd
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance dma-cancel-shutdown
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance dma-cancel-amd-repeat
```

HTTP/deployment ports were 19344/18644, 19345/18645, 19346/18646,
19347/18647 and 19348/18648. The AMD repeat additionally used
`CATTEN_SKIP_KERNEL_BUILD=1` only after the current ELF matched the immutable
failed AMD boot hash. Per-boot hashes precede subsequent profile rebuilds:

| Artifact | SHA-256 |
| --- | --- |
| ARM combined kernel | `504ae8dff07571f04f46b43f319585f590344ac0663884bc8c0b4ba631bdee83` |
| Intel / failed AMD / AMD-repeat kernel | `d4d230248cc0fa4ad0fc47bcb936ee041f77433414f009e8fb6ca45d1460a926` |
| ARM shutdown kernel | `7412128fae2ca072967829539ceab406b127dc730194c82e403b8adf1d1d5751` |
| ARM bundled service ELFs | `f74e73eeae72102ea2b0b43837c11934151df5beb57698af4e61bc549a76f644` |
| x86 bundled service ELFs | `90e43c8e691e01e151898a1caf337e22d9f7141744dbc3054d9c4424d0e69ccd` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256. Diagnostics, failure symbols, results and hashes are inline;
temporary logs and ignored build artifacts are not report dependencies.

## Remaining limits

Public DMA map/unmap/close and whole-domain device cleanup still have separate
busy-rejection and caller/IRQ qualification boundaries. The captured AMD mapping
failure needs precise per-return cause and readiness/progress evidence; its
passing repeat proves neither cause nor reliability. The older Intel startup
failure remains unresolved as well.

This correction introduces no retry for started maintenance/physical release,
no recovery controller or custody registry, and no forced root/endpoint claim
release. Wait admission failure and indefinite retained claims remain terminal
to the existing complete owner. Broader outer guards, implicit metadata lifetime,
principal/heap pressure and parked live-hardware concurrency remain G1/G4/G7.
SEC-18 and SEC-07 remain partial, with production enrollment restrictions intact.
