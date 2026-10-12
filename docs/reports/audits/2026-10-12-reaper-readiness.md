# Event-driven LP reapers — 2026-10-12

Scope: the retirement-worker notification gap from the
[weekly wait sweep](2026-10-12-wait-readiness-sweep.md), existing cleanup family
C16 and SEC-18. Baseline: `6bff0569c665ca5946674cc4f9028eac3522ce1c`.
This follows the [complete-payload fallback correction](2026-10-12-thread-metadata-retention.md).
Current rules are in [thread retirement](../../reference/thread-retirement.md)
and [cleanup/recovery](../../reference/cleanup-recovery.md).

## Change and ownership boundary

Each pinned LP reaper previously scanned its retirement head and slept for one
millisecond, even while idle or retaining only terminal failed pairs. It now
uses `try_park_until` with an owned, fallibly admitted waiter and no idle
timer/watchdog. The durable exact-head recheck repeats the original unstarted,
executing-SP and Arm `on_cpu` predicates after registration. Wake-before-bind
and work already present before worker startup therefore remain observable.
The actual detached owner is checked again before cleanup: a wake is not a
stack-quiescence or physical-release proof.

Staging can run inside `SystemScheduler::abort_thread_generation` while an
outer scheduler read guard exists. It must not invoke Waker callbacks there.
After linking the complete existing node under `DEAD_THREADS`, staging only
publishes a coalesced atomic LP wake. Yield, idle and both architectures'
post-switch boundaries claim that hint and deliver detached notifications after
producer/local scheduler guards leave. The list/source guards leave before
generation-bound Waker admission. Dispatch preserves its containing IRQ state,
including a masked post-switch/IRQ-tail entry; it performs no stack cleanup.
Physical release still runs on the scheduled IRQ-enabled owner-LP worker.

The fixed readiness array accompanies the existing 256 LP retirement heads. It
contains no thread payload, scalar recovery identity or independent custody.
Each lazy source list and subscription uses existing charged observer/waiter
admission; detached batches retain their original charges through notification.
Normal cancellation uses the existing owned registration. Idle dispatch reads
pending bits before attempting a write, and does not periodically scan payloads
or schedule an idle timer.

Production staging sites remain the post-switch requested-abort extractor and
the serialized immediate removal of a nonexecuting target. Previously executing
contexts retain their table/context identity through the owner-LP switch and Arm
ownership handshake. Boot fixtures stage only never-admitted contexts. This
does not introduce a cross-LP stack-release path.

Failed/started pairs remain in their original complete node but are nonready.
They cannot keep the worker runnable or cause timer rescans. Only rejected wait
admission increments `REAPER_WAIT_ADMISSION_FAILURES` and uses a 10 ms backoff.
The existing void sleep API retains that interval through its cooperative
fallback if timer admission also rejects. This is explicitly pressure backoff,
not ordinary resource waiting or a retry of uncertain physical cleanup.
Admission-capacity notifications remain separate work.

The existing phases are preserved: unstarted ordinary rejection may explicitly
release its exact owner; staged work waits in the LP head; a wake keeps the same
owner; confirmed pair release permits metadata/node disposal; physical failure
or abandonment is terminal retention. No retry registry, refund, root completion
or quiescence publication is derived from readiness.

## Targeted execution evidence

Serialized boot fixtures:

- Stage two complete owners while holding scheduler and master-table write
  guards; no callback may run before those guards leave. The detached callback
  checks availability of scheduler, master-table and retirement-table guards.
- Require one coalesced wake for the selected LP and keep an unrelated LP's
  registration live. Cancel retained tokens and verify their original charges.
- Register after publication and deferred notification, then require the
  durable readiness recheck to observe queued work. This is a deterministic
  ordering fixture, not exhaustive cross-LP race stress.
- Fill the source's 64-entry capacity. Rejected registration and scheduler
  blocking leave a never-admitted target in `NeedsLpAssignment`, with its
  attempted waiter charge refunded and no watchdog installed. Explicit cleanup
  recovers its stack frames; source cancellation restores its sponsor usage.
- Keep the existing failed-pair fixture terminal and assert that it is nonready.
  Current-SP/Arm ownership deferral, metadata retention and physical recovery
  fixtures remain enabled. No additional abandoned root or stack pair is added.

All four guests emitted the assertion-backed diagnostic:

```text
[reaper readiness] guarded staging, coalesced exact-LP wake, late registration recheck, full-source rejection and original frame recovery passed
```

All four also completed the scheduled exit-watch check:

```text
[exit watches] SUCCESS: live-target cancel/rearm, self-exit switch ownership, fast worker registration-before-admission and deferred producer cancellation
```

Existing scheduler-lifecycle and remote-abort deferred verifiers passed with
the production event-only workers. Shutdown observed child domains drained
before PSCI power-off. These checks establish working notification and cleanup
in these episodes; they do not measure CPU savings or prove every IRQ/outer
allocator/destructor context.

## Validation and artifacts

Both service bundles were rebuilt before guest execution with
`scripts/build-catten-services.sh --embed` and
`scripts/build-catten-services-x86_64.sh`. `scripts/run-host-tests.sh` passed:
**384 passed, 0 failed, 3 ignored**. Formatting and diff checks passed.
Strict `-D warnings` Clippy passed for bundled services, ARM
security/HTTP/DHCP kernel and x86 HTTP/DHCP kernel. No failed guest run occurred
in this change's matrix; routine ingress reset/retry probes recovered and the
runners returned success.

Fresh-storage guests used a 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Guest completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 55.789284 s |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 23.149754 s |
| x86 AMD-Vi, same kernel/features | 18 / 0 / 0 | `0x123bbff` | 22.506612 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.430804 s |

Zero failed/pending bitmaps accompany every result. Synchronous boot fixtures
are additional to those registered deferred-test counts. Runner commands used
`CATTEN_SKIP_EMBED_BUILD=1` only after rebuilding both bundles:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance reaper-readiness-arm
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance reaper-readiness-intel
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance reaper-readiness-amd
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance reaper-readiness-shutdown
```

HTTP/deployment ports were 19284/18584, 19285/18585, 19286/18586 and
19287/18587 respectively. AMD reused the exact successfully booted Intel
kernel using `CATTEN_SKIP_KERNEL_BUILD=1`. Immutable per-boot kernel hashes
were captured before subsequent profile builds:

| Artifact | SHA-256 |
| --- | --- |
| ARM combined kernel | `6900742e05c0ca225e6781335f9e6fc002e1639572affa62b533be9b48335c4e` |
| Intel and AMD kernel | `83e56892edb2973350b8b4ece27b399739fe0d8fd85e3305c7f3dc3bf83a0324` |
| ARM shutdown kernel | `36f1c138ae8fbaef7e9ffc24502a3519b5758a1227590c99e7c770960f17ea02` |
| ARM bundled service ELFs | `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621` |
| x86 bundled service ELFs | `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999` |

Bundle digests use the runner convention: SHA-256 of sorted newline-terminated
`basename:ELF-sha256` lines. Services are unchanged from the baseline.

## Remaining audit work

The ordinary one-millisecond LP-reaper polling gap is corrected. Memory
pin/transfer/lease drains and x86 shootdown coordination still have the separate
notification gaps recorded by the weekly sweep. Pressure-retry capacity
notification, comprehensive caller/IRQ/implicit-destructor qualification and
the integrated concurrency/progress matrix remain G1/G7 work. This does not
close C16, SEC-07 or SEC-18, create a failed-pair recovery adapter or attribute
historical Intel storage, AMD startup or teardown timeout failures.
