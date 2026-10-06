# Owned explicit endpoint close — 2026-10-06

Baseline: `072a817b`, following the
[bulk cleanup failure correction](2026-10-06-security-bulk-cleanup.md).
This advances SEC-18 by removing IPC/lifecycle serialization from loan cleanup
for explicit endpoint close. Whole-domain cleanup and hardware quiescence remain
separate work.

## Correction

An endpoint containing queued loans previously confirmed all revocations under
the global IPC write guard before consuming close authority. Failure was retained
correctly, but mapped-loan cleanup could still perform x86 invalidation while IPC
masked interrupts and blocked recipient progress.

`PreparedEndpointClose` now owns the exact server root and an endpoint claim.
The claim rejects new sends, receives, minting, resize and CQ rebinding without
reporting terminal closure. The owner uses the existing admitted queue as its
work list, processing one call at a time. No whole-queue snapshot is allocated.
Each queued call borrows that server owner through a Rust lifetime and leases
its exact caller before IPC. Revalidation and bounded loan preparation precede
the token claim. Mapped-loan detach, invalidation and scratch completion then run
outside IPC/lifecycle, memory-registry and table guards.

The server owner predates any staged server close. Per-call preparation borrows
that retained lease rather than attempting forbidden new admission after the
closing fence. A competing caller cancellation can win before token claim;
endpoint close waits cooperatively outside IPC and re-resolves the front.

Each call publishes `REPLY_ENDPOINT_CLOSED` only after its own loans complete and
queued attachment authority is removed. Earlier confirmed calls may finish before
a later call fails. Endpoint close watches, readiness and CQ closure notifications
remain pending until all queue work is gone, authority is removed and the server
lease completes. Admission stays fenced through final publication.

Ordinary rejection preserves endpoint authority, returns the server lease and
clears only its endpoint claim. A failed loan retains its pin/token fence, while
unstarted receipts can be restored. Readable work exposed by preparation rejection
is re-signaled after unlock. Abandonment retains the endpoint claim/root and any
active call's caller lease, claim and uncertain backing. There is no force-clear
or teardown retry that restores uncertain authority.

Asynchronous sends have no loans and retain their existing copy/move cleanup.
Endpoints without queued loans still close atomically under IPC. Whole-domain
cleanup retains its non-leasing serialized adapter and propagated failure result;
this live owner is never invoked beneath the root's lifecycle guard.

## Validation

New boot fixtures cover two independent callers with mapped read/write loans,
queued copies/moves and a delegated connection, scalar send/call, a self-call,
competing close/cancellation, endpoint/CQ/authority admission fencing, no premature
endpoint watch/readiness notification, staged server close, caller-admission and
loan-preparation rollback, readiness restoration, partial physical failure and
abandoned endpoint/call ownership.

A deferred fixture runs after secondary LPs are online. It closes an endpoint
with mapped loans from two callers using the public kernel API and verifies both
terminal results and restored loan authority before releasing all three roots.
On x86 this exercises actual cross-LP invalidation with the new unlocked owner;
the borrower roots have no application threads. It is not a concurrent user
hardware-walk or failed-recipient injection campaign.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including direct slot/scratch ownership and existing runtime, services, protocol, authorization and signing suites. |
| `scripts/build-catten-services.sh --embed` | Passed; rebuilt/staged/signed the AArch64 service bundle. |
| x86 and AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Isolated four-LP x86 guest | **15 passed, 0 failed, 0 pending**. New boot ownership/failure fixtures passed; the deferred mapped-loan test completed with secondary LPs online and actual synchronous shootdowns. Existing fault/register, lifecycle and storage tests also passed. |
| Isolated AArch64 security guest | **19 passed, 0 failed, 0 pending**. New boot and deferred endpoint fixtures passed. Both scoped probes reported `0xffff`, generations reached 1 and 2, cancellation traffic retired after 4,724 requests. TCP/IP clients retired after 1,348 and 1,322 requests with clock/cycle progress. |

Both runners rebuilt the kernel and reused the previously built signed service
bundles; service source and dependency/lockfile versions did not change. The
first sandbox ARM launch failed to bind its forwarding ports. The approved
isolated rerun passed; no existing VM or storage was stopped or reset.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance endpoint-security-20261006 --fresh-storage --timeout 160

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18388 \
CATTEN_DEPLOY_HOST_PORT=17688 scripts/run-aarch64.sh --security-test \
  --instance endpoint-security-20261006-approved --fresh-storage --timeout 150
```

Logs:

- `/private/tmp/charlotte-endpoint-host.log`
- `/private/tmp/charlotte-endpoint-services-build.log`
- `/private/tmp/charlotte-endpoint-clippy-x86-final.log`
- `/private/tmp/charlotte-endpoint-clippy-arm-final.log`
- `/private/tmp/charlotte-endpoint-guest-x86.log`
- `/tmp/charlotte-x86-endpoint-security-20261006-serial.log`
- `/private/tmp/charlotte-endpoint-guest-arm-approved.log`
- `/tmp/charlotte-endpoint-security-20261006-approved-serial.log`

## Remaining scope

SEC-18 remains partial: whole-domain IPC/memory/device retirement, composition
for already-closing namespaces, comprehensive CPU/DMA quiescence, recoverable
shootdown failure and physical-device reset need further work. The failed-loan
adapter abandons a real prepared pin; it does not simulate an actual rejected IPI.
Failed/abandoned fixtures intentionally retain roots and charged backing.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
