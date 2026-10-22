# Deployment logical-close observation

Date: 22 October 2026. Reviewed baseline: `4cadfb0d`.

SEC-06 remains partial. Deployment ingress now consumes completed or rejected
clients into CQ-bound logical-close owners. One outstanding close leaves other
ready inputs, controller replies, output receipts and close acknowledgements
observable. The client position stays occupied until checked acknowledgement.
The ledger remains 29 corrected, 6 partial and 3 open; production trust remains
disabled.

## Ownership and failure policy

`OwnedSocket::into_closing` consumes the exact socket into `ClosingSocket`, with
its accepted pending call before the socket field. `deployment_close::Closes`
retains that owner, one original five-second admission/observation deadline and
optional admission backoff. Listening, input, controller, output and close phases
share four positions. The service visits each processing pool before blocking
on the earliest original operation or admission-backoff deadline.

Admission enters the existing `OP_CLOSE` (8), with no new wire format or scalar
owner reconstruction. Queue-full rejection alone permits another attempt after
50 ms, retaining the original budget and exact owner. That backoff has no queue
readiness producer and is not described as event-driven waiting. After acceptance,
the owner binds its reply to CQ0 and never submits again. A held close cannot
make the reactor wait on only that source.

Only typed Started (0) or Unavailable (1) scalar receipts without attachments
end the logical owner. It disarms the
socket's ordinary remote-close fallback and disposes the completed pending owner
before releasing the socket and client position. Logical acknowledgement is no
proof of peer delivery, TCP-buffer release, physical cleanup or original-charge
refund. TCP/IP retains its existing graceful-close/reaping policy and timed
transport cadence.

Non-queue submission error, rejected binding/wait, invalid/service/observation
result or original deadline expiry aborts the domain with its complete existing
scope live. There is no cancellation followed by a reconstructed close. An
incomplete `ClosingSocket` destructor also aborts before implicit field cleanup;
it cannot synchronously wait or issue a second remote close. This is terminal
retention, without retry custody or recovery authority. Unexpected returned
attachments use existing runtime owner disposal; no loan or request memory is
introduced by close submission.

On lifecycle shutdown, the reactor drains existing close operations with their
original deadlines while retaining all other client owners. Once this pool is
empty, the existing other-phase destructors run. Partial listener setup and
those older `OwnedSocket` fallback destructors can still block. This change
qualifies normal/rejected-client close scheduling, without claiming bounded
whole-service shutdown or replacing every synchronous socket cleanup path.

The existing C18 row records these owner and destructor contexts. No new
registry, cleanup family, root, pin, physical proof or retry controller is added.

Socket IDs no longer wrap or reuse within a service endpoint lifetime. An
opaque `ReservedSocketId` is consumed by publication; failed publication burns
that serial too. Exhaustion rejects before backend storage/buffer allocation.
The runtime owner retains its connection to that same service incarnation.
Unavailable means an absent full ID below the reservation watermark; it attests
to no former owner, backing cleanup or refunded charge. Foreign live entries
and zero/unreserved IDs still reject. Both synchronous and reactor close
recognize the two current receipts. Immediate packed operations remain a
separate full-ID migration gap.

## Verification

The [version-controlled evidence](evidence/2026-10-22-security-deployment-close.json)
records commands, native artifact hashes, results and failure diagnostics.

All 193 host library tests pass. Eight close controls cover a held operation
beside healthy completion; admission backoff beside a healthy completion;
queue-full-only retries under an unchanged deadline; accepted timeout without
replay; expiry with an unstarted owner; rejected submission/binding;
service/observation/malformed-result failures; and capacity retention until
confirmation. Abort adapters assert that the exact socket and any accepted
pending call remain live. Successful fake owners dispose pending before socket.
Host panic/unwinding checks are no production domain-retirement proof. The
production receipt decoder is source-reviewed; native Started/Unavailable receipts exercise it,
while native malformed returned-capability cleanup is not fault-injected.

Three additional producer/codec controls check nonreuse/exhaustion, exact live
foreign/unavailable/unreserved classification and the two-receipt decoder.

The scoped native security fixture consumes actual TCP and UDP socket owners
into the close pool and requires both logical acknowledgements within two
seconds, leaving no occupied close position. It retains an unactivated UDP owner
across the existing five-second held-controller case, then requires a typed
Unavailable receipt. No extra sleep or wider deadline is introduced. Both cases
run in both scoped roots.
Normal HTTP and signed central-store release flows exercise the production
response-to-close transition. These cases do not withhold actual TCP/IP close
replies, saturate its admission queue or inject live observation/binding failure.
Host adapters cover those policies separately.

Final native results:

| Run | Self-tests | Additional result |
|---|---|---|
| AArch64 security | 19 passed, 0 failed, 0 pending | Both scoped roots confirm Started and actual reaped-owner Unavailable |
| x86 security | 17 passed, 0 failed, 0 pending | Same two scoped-root cases |
| AArch64 HTTP, fresh repeat | 19 passed, 0 failed, 0 pending | Abortive-peer recovery and all framing/input/deadline controls pass |
| x86 HTTP | 17 passed, 0 failed, 0 pending | Healthy stalled-peer replies in 0.301–0.481 s; trickled body ends in 30.30 s |
| AArch64 signed release | 20 passed, 0 failed, 0 pending | Complete release receipt and exact-generation ready result |

Strict host/library and native service Clippy pass. Both signed bundles build;
AArch64 reproduction matches current-source signed bytes. Rust formatting,
shell syntax and diff whitespace checks pass. Markdown and LaTeX carry the
new contract; the existing manual PDF remains unchanged. The signed fixture
retains its initial connection refusal and two HTTP 409/error -23 admissions
before acceptance, alongside the final successful receipt/readiness result.

`CharlotteDeploymentClose` starts with two already owned sockets. The safe
configuration explores 783 states at depth 12. It checks accepted-once submission,
retained owners until acknowledgement, original budget, admission backoff and
complete wait interests. Accepted replay violates `AcceptedOnce`; releasing a
position without acknowledgement violates `OwnersUntilAck`. Bound sources and
atomic transitions are assumptions. Actual nonreused ID allocation/reaping, socket setup, wire decoding, cross-phase
admission, whole-shutdown timing, kernel scheduling and physical release remain
outside this finite projection. The full suite checks 40 safe configurations and
117 expected counterexamples across 38 models.

The initial ARM and x86 HTTP runs passed stalled-input and EOF controls, then
failed during abortive-peer recovery. Both recorded deployment-domain abort
with `logical close failed with owner retained: Observation(Service(-2))` and
host connection resets, despite otherwise passing kernel self-tests. Source
review showed that terminal/reset/grace/unused reaping can remove a socket
before its owner submits close. Requiring only zero conflated that state with
uncertainty and stopped the domain. The producer now gives the explicit,
strictly logical Unavailable receipt under nonreused IDs; it does not reinterpret
a generic BAD_SOCKET denial or infer physical cleanup from absence. The initial
failed runs remain separate from final corrected runs in the evidence. Their
artifact hashes identify the intermediate build; its complete uncommitted source
snapshot was not retained. The relevant zero-only predicate and observed -2
rejection are described here, without claiming complete source reproducibility. Earlier
ordinary security/release success did not cover this reaping boundary.

A later AArch64 HTTP guest using the corrected bundle expired its 240-second
budget before kernel entry. It captured only UEFI diagnostics, CPU0 at
`0x5fdf74ac` and secondary PCs at zero. This is retained separately from the
close-contract failures and the fresh-storage repeat. The same pre-kernel
symptom is recorded in the preceding response-send audit. No service code was
exercised in this failed boot; a passing repeat does not resolve its cause.
No runner, request or aggregate fixture deadline was widened.

The ordinary synchronous helper remains a separate policy gap: after an
accepted observation/service failure it can restore its ID and let `Drop`
submit another close. The new consuming reactor owner never takes that path.
This batch does not qualify the older helper's uncertain-error fallback.

## Remaining work

Socket open/listen and partial-setup failure cleanup remain synchronous. Ordinary
`OwnedSocket::close`/Drop, including other phases' implicit shutdown cleanup,
retain their older blocking/fallback policy and need separate qualification.
Response rendering and controller-server work can serialize clients. Saturated
admission, protected operator capacity, native slow-reader buffers, sustained
flood/NIC failure, principal/heap admission and whole-service shutdown under
pressure remain SEC-06/07 work. A failing close deliberately stops its domain;
this change provides no close-recovery owner or availability guarantee through
that terminal boundary.
