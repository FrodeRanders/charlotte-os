# Bulk IPC cleanup failure propagation — 2026-10-06

The SEC-23–25 fixes were committed as `8337c31e`. This follow-up addresses the
specific ignored-error path remaining under SEC-18 in the
[renewed audit](2026-10-05-security-audit.md). It does not close all of SEC-18.

## Defect and correction

Serialized pending-call cancellation removed its pending record and reply token
before revoking loans, ignored revocation errors and then discarded queued
ownership. Endpoint close retained a failed reply token but had already consumed
its endpoint capability and queue, and still returned success. Namespace cleanup
ignored individual close errors and unconditionally removed its remaining
capability table. Root retirement had no IPC failure result to propagate.

Bulk close now confirms the relevant loans before consuming close authority,
queued attachments or pending records. Each confirmed loan is removed from the
token immediately. A rejected revocation marks the token `cleanup_failed`,
returns `MemoryTransferFailed`, preserves the close capability and queue, and
publishes no terminal result or cancellation notification. Existing receive,
readiness and reply checks honor that fence. A partial cleanup cannot restore
uncertain loan authority.

Namespace cleanup preflights every token involving the closing domain, including
delivered replies and foreign callers. It returns errors before consuming IPC
capabilities. Root retirement propagates `IpcCleanupFailed`, retains its exact
root and accounts, and neither destroys memory objects nor detaches/recycles
the root. Immediate root close now also owns a `ClosingSlot` before irreversible
subsystem mutation. Both close modes therefore retain a closing fence on error;
fresh operation leases and competing close requests reject. Existing supervisor
teardown caches terminal errors and retains deployment bookkeeping.

The teardown walks use existing admitted registry/queue storage rather than
allocating capability or token snapshots. Confirmed earlier loans and subsystem
retirement are not rolled back. Failed namespaces are retained without a retry
or administrative recovery path.

## Validation

Guest regression fixtures cover successful endpoint close and queued/delivered
caller-root close with actual mapped loans. Failure fixtures abandon an actual
prepared loan on the second cleanup step, keeping its pin and revocation fence.
They check preserved partial receipts, queued/call/capability authority, zero
terminal waiter notifications, receive rejection, root-error propagation,
closing-lease rejection and non-reuse of frames, charges and software slots.
Existing explicit cancellation/reply and staged retirement fixtures remain enabled.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including direct generic retirement-slot fixtures and existing runtime, service, protocol, authorization and signing tests. |
| x86 kernel Clippy | Custom target, `--locked`, `-D warnings`: passed. |
| AArch64 kernel Clippy | Custom target, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Isolated four-LP x86 guest, no network | **15 passed, 0 failed, 0 pending**. Bulk failure, explicit reply/cancellation, staged-root retirement and existing register/fault containment fixtures executed. |
| Isolated AArch64 security guest | **19 passed, 0 failed, 0 pending**. Bulk fixtures executed; both scoped probes reported `0xffff`, publication generations reached 1 and 2, cancellation traffic retired after 4,836 requests. Two TCP/IP clients retired after 1,696 and 1,652 requests with clock/cycle progress. |

Both runners rebuilt the kernel and reused the previously rebuilt signed service
bundles; no service source or dependency/lockfile change was needed. Commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance bulk-security-20261006-v2 --fresh-storage --timeout 160

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18387 \
CATTEN_DEPLOY_HOST_PORT=17687 scripts/run-aarch64.sh --security-test \
  --instance bulk-security-20261006-v2 --fresh-storage --timeout 150
```

An initial guest exposed an older fixture's deliberate direct revocation of an
IPC loan without updating the token's receipt, followed by an expectation that
root cleanup would succeed. The corrected fixture verifies conservative root
retention for that unconfirmed receipt. Existing partial-reply failure fixtures
also now verify propagated root rejection rather than silent teardown. Initial
unsuccessful runs are not passing evidence. The sandbox ARM launch could not bind
its forwarding ports; the approved isolated rerun passed. Existing VMs and their
storage were not stopped or reset. A final fixture log correction removes obsolete
retained-page/root counts; it changes no tested behavior.

Logs:

- `/private/tmp/charlotte-bulk-host.log`
- `/private/tmp/charlotte-bulk-clippy-x86-final.log`
- `/private/tmp/charlotte-bulk-clippy-arm-final.log`
- `/private/tmp/charlotte-bulk-guest-x86-v2.log`
- `/tmp/charlotte-x86-bulk-security-20261006-v2-serial.log`
- `/private/tmp/charlotte-bulk-guest-arm-v2.log`
- `/tmp/charlotte-bulk-security-20261006-v2-serial.log`

## Remaining scope

This is conservative failure propagation, not split-phase bulk retirement.
Endpoint/domain cleanup still retains IPC/lifecycle serialization across some
x86 invalidations. It still needs composed multi-call ownership, admission for
an already-closing namespace, unlocked physical cleanup, recipient progress and
hardware CPU/DMA quiescence. Whole-domain device cleanup remains serialized.
The injected failure models uncertain physical cleanup; it is not a real failed
IPI, allocator corruption or physical-device reset. Failed fixtures deliberately
retain their roots and backing rather than inventing a reclamation bypass.

SEC-07 translation/metadata admission and the earlier peer/management
authentication, production provisioning and security-time work remain open.
