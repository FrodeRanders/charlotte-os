# IPC connection delivery — 2026-10-06

Baseline: `d753371c`, following
[owned-memory delivery](2026-10-06-security-memory-delivery.md).
This closes newly identified SEC-30. SEC-18 remains partial.

## SEC-30: undelivered connections could mint surviving authority

**Severity: medium capability-lifetime/availability. Corrected.** Connection
attachments and returned connections were installed in their recipient's live
IPC namespace before receive or reply observation. Ordinary lookup did not
distinguish those grants from delivered authority. A recipient guessing a valid
local ID could therefore send/call, register a close watch, resolve a management
target, explicitly close it, or mint/delegate a fresh child connection.

Queued cancellation and unobserved-result cleanup remove only the original
connection. A child minted from it has an independent identity and survives that
cleanup, so authority could escape the promised cancellation boundary before
delivery. Calls/sends could also cause side effects before the recipient acquired
the grant through its message/result. Earlier split-phase reply source checks
rejected queued/unobserved sources by scanning IPC records, but ordinary mint and
use paths did not share that qualification.

This is confirmed by source inspection and fixtures using actual live hidden
identities. The new regression checks execute the ordinary mint/use boundaries
and verify rejection without changing authority or sponsorship. They do not
demonstrate an external attacker or a separate EL0 exploit payload. The grant
targets its intended recipient and grants only the source's attenuated rights;
this finding does not establish arbitrary access to an unrelated endpoint or a
new rights-escalation primitive. It violates ownership and cancellation timing.

## Delivery ownership

`PreparedConnection::install` marks its admitted entry `delivery_pending` under
IPC. This covers queued connection-only/combined calls and both immediate and
split-phase returned-connection replies. Ordinary `IpcRegistry::cap` rejects the
entry with `UnknownCapability`. Every public connection use, mint/delegation,
watch, management-target resolution and explicit close uses that lookup.
Hidden grants retain their live unified authority and original record charge.

Successful receive publishes its exact queued connection under the IPC hold,
after speculative reply admission and result-page writing succeed. Failure
preserves hidden queue ownership. First reply polling publishes the exact
returned connection together with any returned owning memory before recording
observation under the same IPC hold. Readiness waiting does not publish either
family. Repeated polling returns the stored result without republishing consumed
authority.

Private cleanup can consume hidden entries. Namespace drain uses that lookup
without allowing public explicit close to bypass delivery. Queued/unobserved
entries still count as endpoint references, retaining backing metadata even when
the endpoint's owner closes. Internal queue/result removal returns original
sponsorship and can release the last reference to a closed endpoint.

Direct connection mint/delegation, including launch grants, remains immediately
usable. Delivered grants can legitimately mint attenuated children that survive
subsequent pending-call close. No connection authority is adopted twice, no
new provisional allocation/teardown snapshot is introduced, and no lifecycle
lease is acquired beneath IPC.

The connection-specific queue/result source scan has been removed. Ordinary
lookup now rejects hidden reply sources before grant/loan preparation. Delivered
sources keep their existing exact source-close claim and root leases through
split-phase loan cleanup. Hidden sources return `UnknownCapability` rather than
the previous scan's `Pending`; internal interfaces have no compatibility
requirement. The distinct memory source check remains outside this batch.

## Validation

New raw kernel ABI fixtures capture real queued/unobserved grants and check
rejected send, call, mint/delegation, close-watch registration/count, management
target resolution and explicit close. They verify unchanged capability counts,
record charges and target queue depth. Connection-only and combined copied-memory
calls cover failed receive result writing, successful handoff, caller
cancellation and endpoint close. After delivery, attenuated children survive
call close and still send; denied rights remain denied.

Closing either the caller or server root reclaims queued hidden connections and
original sponsorship. Returned grants remain hidden through readiness waiting;
first observation enables use, repeated polling after explicit grant close does
not republish missing authority, and unobserved call/root cleanup removes hidden
results. Existing split-phase reply fixtures still pass source-close waiting,
unrelated endpoint-owner retirement, loan cleanup, quota/preparation rollback,
staged root close and publication failure. Existing closed-endpoint accounting
fixtures continue to verify last-reference reclamation.

The new fixtures add no deliberately quarantined roots or pages. Existing
physical-failure/abandonment quarantines remain. These are kernel-boundary
interleavings plus the existing EL0/production guest suites, not a new physical
DMA or rejected-IPI experiment.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including slot/scratch ownership, runtime/services/protocol, signing and boot-result suites. |
| x86/AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Four-LP x86 guest | **15 passed, 0 failed, 0 pending**, including new connection fixtures and existing EL0 IPC tests. |
| AArch64 security guest | **19 passed, 0 failed, 0 pending**. Both probes reported `0xffff`, publication generations reached 1 and 2; cancellation traffic retired after 4,764 requests. TCP/IP clock/cycle progress held while pressure clients retired after 1,610 and 1,593 requests. |

Successful guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance connection-delivery-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18498 \
CATTEN_DEPLOY_HOST_PORT=17798 scripts/run-aarch64.sh --security-test \
  --instance connection-delivery-20261006 --fresh-storage --timeout 180
```

Runners rebuilt both kernels and checked native assembly permissions. Existing
validated signed service bundles were reused; this batch changes kernel code
only. The initial ARM launch could not bind host-forwarding ports in the sandbox;
the approved isolated retry passed. No unrelated VM or storage was stopped/reset.

Logs:

- `/private/tmp/charlotte-connection-delivery-host.log`
- `/private/tmp/charlotte-connection-delivery-clippy-x86.log`
- `/private/tmp/charlotte-connection-delivery-clippy-arm.log`
- `/private/tmp/charlotte-connection-delivery-guest-x86.log`
- `/tmp/charlotte-x86-connection-delivery-20261006-serial.log`
- `/private/tmp/charlotte-connection-delivery-guest-arm-approved.log`
- `/tmp/charlotte-connection-delivery-20261006-serial.log`

## Remaining scope

SEC-18 remains partial: allocator release latency under IPC, recoverable
shootdown failure, full CPU/DMA quiescence, physical-device reset and recovery
for abandoned roots remain open. Final root shutdown still depends on
caller-established thread quiescence and recipient progress. Loans retain their
existing owned revocation and delivery contract; this does not add deferred
visibility to every capability family.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
