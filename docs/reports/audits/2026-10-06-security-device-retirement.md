# Owned device retirement and DMA-pinned loans — 2026-10-06

Baseline: `796f7a31`, following
[owned namespace cleanup](2026-10-06-security-namespace-close.md).
This advances SEC-18 for whole-domain devices and closes the newly identified
DMA loan-authority gap, SEC-27. SEC-18 remains partial.

## SEC-18: whole-domain device cleanup

The old device adapter removed every capability before physical completion,
ignored MMIO detach errors, and logged DMA-domain destruction errors while root
retirement continued. It also retained the outer lifecycle guard through device
cleanup. A detached closing root protected final destruction, but did not own
this earlier cleanup interval.

`PreparedNamespaceDevices` now borrows the exact `ClosingAddressSpace`. After
old operations drain, lifecycle serialization retires backing/capability and
IPC sponsorship, fences owned endpoints, and detaches admitted device registry
storage. IRQ routes are masked/removed under the device registry so another
grant cannot publish a replacement that old teardown subsequently disables.
The existing capability map becomes the work list; this step allocates no
namespace snapshot. The closing owner retains its software generation, root,
hardware tag and accounts throughout the unlocked interval.

After lifecycle/device guards leave, each mapped MMIO region checks its leaf's
physical identity under the table before detach. The table guard leaves before
range invalidation. Even a failed detach invalidates its potentially removed
prefix. Only confirmed detach/invalidation permits exact scratch release.
DMA destruction likewise runs without outer lifecycle/device/table guards;
existing backend serialization and hardware acknowledgement checks remain.
Backend rejection retains its tables/pins rather than recycling reachable backing.

Each successful object releases unified device authority before removing its
owned record. Preparation, detach, invalidation, scratch or DMA failure stops
root close with `DeviceCleanupFailed`. Unfinished records, device admission
charges, scratch reservations, root and accounts remain retained. Earlier
confirmed devices stay closed. Drop retains admitted map storage and performs
no hardware callback. There is no reconstruction, retry or forced reclamation.

Only the exact `NamespaceDevicesClosed` receipt advances the root to IPC loan
cleanup. A failed/abandoned device phase cannot be skipped on a subsequent poll.
DMA teardown still precedes that namespace's loan drain and all root backing
destruction. Existing supervisor error handling propagates/caches the new close
error and retains deployment state.

## SEC-27: loan revocation could return authority with live DMA pins

**Severity: high, conditional on delegated DMA authority. Corrected.** A
borrower can obtain a nonexclusive DMA pin for a read loan, or a writable loan
with matching rights. Before this change, `LoanRevocation::prepare` rejected
retirement pins and destroyed objects but did not reject DMA pins. Its completion
could remove borrower authority and restore lender access after only CPU
detach/invalidation. The DMA pin retained the physical allocation but did not
prevent the lender from reusing the same buffer after IPC completion. A device
could therefore retain access while the lender resumed reading/writing it.

This is a source-confirmed kernel authority/lifetime gap; validation exercises
real pin and loan state, not a malicious physical device or an end-to-end DMA
data-corruption exploit. It also matters for unlocked device teardown: another
namespace may attempt to revoke a loan while the borrower's DMA destruction is
still pending.

Loan preparation now rejects **all live DMA pins** under the memory registry
before publishing `Revoking`. DMA admission already rejects that fence under
the same registry, closing the preparation/admission race. Rejection preserves
original loan state and mappings and completes newly admitted operation leases.
Reply/cancellation reports cleanup failure without publishing a terminal result
or restoring lender authority. After explicit DMA unpin, normal revocation can
succeed. This conservatively includes pins belonging to another reader or the
lender; it does not try to infer per-pin hardware ownership.

The common preparation guard covers direct revocation, borrowed-memory replies,
explicit cancellation, endpoint close, and whole-domain IPC loan cleanup. Whole
domain close may retain its fenced root on this rejection, according to its
existing failure policy. It does not wait under a registry guard or pretend
CPU invalidation revoked DMA translations.

## Validation

Boot fixtures cover successful scratch/direct MMIO teardown and fake backend
DMA completion; rejected preparation before registry detachment; partial MMIO
detach with a foreign leaf left intact; rejected invalidation, scratch and DMA
completion; abandoned ownership; exact generation reuse; retained capability
charges and scratch extents; and refusal to skip an unfinished device phase.
Callbacks assert lifecycle/device/table guards are absent. Six deliberately
quarantined closing roots remain, including five heap-page charges; these are
additional to existing audit fixtures and have no recovery bypass.

Read/write loan fixtures hold real DMA pins, reject revocation while mappings
and lender restrictions remain, then unpin and complete ordinary cleanup.
Queued/delivered IPC fixtures verify cancellation/reply publishes no result
while read/write loans are DMA-pinned, followed by successful cleanup after
unpin. These do not inject a hardware DMA timeout.

A deferred production fixture closes scratch/direct MMIO mappings after
secondary LPs start. The x86 guest exercises synchronous cross-LP invalidation;
these roots have no application threads, and the fixture never accesses device
register contents. Concurrent hardware walks and failed IPI delivery remain
separate validation work.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including 23 slot tests, scratch ownership, runtime/services/protocol, signing and boot-result suites. |
| `scripts/build-catten-services.sh --embed` | Passed; staged/signed AArch64 service bundle. |
| x86/AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed after the DMA guard and fixtures. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Final four-LP x86 guest | **15 passed, 0 failed, 0 pending**, including DMA-pinned loan fixtures and deferred device retirement. |
| Final AArch64 security guest | **19 passed, 0 failed, 0 pending**. Both scoped probes reported `0xffff`, generations reached 1 and 2, cancellation traffic retired after 4,816 requests. TCP/IP clock/cycle progress held while pressure clients retired after 1,764 and 1,671 requests. |

Final guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance device-dma-security-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18494 \
CATTEN_DEPLOY_HOST_PORT=17794 scripts/run-aarch64.sh --security-test \
  --instance device-dma-security-20261006 --fresh-storage --timeout 180
```

Both runners rebuilt their kernels, checked assembly section permissions and
reused signed service bundles. A preceding device-only x86/ARM pair passed
15/15 and 19/19 respectively; final runs additionally include the DMA loan
guard and its regression fixtures. ARM runs used approved isolated forwarding
ports. No unrelated VM or storage was stopped/reset.

Logs:

- `/private/tmp/charlotte-device-host.log`
- `/private/tmp/charlotte-device-services-build.log`
- `/private/tmp/charlotte-device-clippy-x86-final.log`
- `/private/tmp/charlotte-device-clippy-arm-final.log`
- `/private/tmp/charlotte-device-guest-x86-final.log`
- `/tmp/charlotte-x86-device-dma-security-20261006-serial.log`
- `/private/tmp/charlotte-device-guest-arm-final.log`
- `/tmp/charlotte-device-dma-security-20261006-serial.log`

## Remaining scope

SEC-18 remains partial: whole-domain memory and IPC move/copy/result attachment
cleanup still retain outer serialization. Recoverable shootdown failure, full
CPU/DMA quiescence and physical-device reset remain open. Production x86 range
invalidation still has no recoverable rejected-IPI result; a fault adapter's
`false` result does not establish failed-recipient recovery. Existing DMA
backends retain their hardware-locking and allocation paths; this change removes
outer lifecycle/device guards, not every backend allocation or hardware wait.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
