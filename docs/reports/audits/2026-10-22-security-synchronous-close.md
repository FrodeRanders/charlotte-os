# Synchronous socket-close ownership

Date: 22 October 2026. Reviewed baseline: `589df875`.

SEC-06 remains partial. Ordinary `OwnedSocket::close` and Drop now consume the
same logical-close owner used by the deployment reactor. The old synchronous
helper restored its ID after accepted observation/service failure; the enclosing
Drop could then submit another close. That path is removed. The ledger remains
29 corrected, 6 partial and 3 open; production trust remains disabled.

## Lease and driver

A private `SocketLease` keeps the remote selector and original service connection
together. `OwnedSocket::take_closing` moves that lease once into `ClosingSocket`,
leaving the original wrapper empty. Neither explicit close nor its enclosing
Drop can reconstruct the selector or submit through the emptied wrapper. No raw
kernel handle is adopted, copied or reconstructed for cleanup.

CQ polling and the new borrowing `ClosingSocket::wait_timeout` share one receipt
observer. Only Started (0) or Unavailable (1), without attachments, ends the
logical lease. The pending owner disposes before that lease. Unavailable remains
a logical selector receipt in the captured service incarnation, without former
ownership, peer-delivery, physical-release or charge-refund proof.

The synchronous `Blocking` transaction retains the complete close owner, original
five-second deadline, one-second admission limit and driver phase. Both deadlines
start from the same monotonic sample before submission. Only explicit queue-full
rejection permits 50 ms admission backoff, clipped to the remaining first-second
window and overall budget. Queue admission has no readiness notification source;
this backoff is not accepted-call polling. Other admission errors return directly.

After acceptance, the driver calls the existing owned `PendingCall::wait_timeout`
once with the remaining original budget. The kernel/runtime checked wait handles
unrelated wakes and its own absolute-deadline repark loop. This adds no one-wake
adapter, extra watchdog capability, timer-polling loop or new readiness producer.
Exhausted remaining budget, timeout, rejected wait admission, malformed/service
receipt or observation failure aborts with the containing close transaction live.
Those outcomes remain distinct in diagnostics and never restore an ID or permit
another submission. A terminal runtime observation may already have consumed its
pending kernel grant; the containing logical-close owner remains retained.

Final unstarted admission rejection returns its actual IPC error and forgets the
complete driver without running its field destructors. No accepted call, request
memory or loan exists in that state. Its runtime fields own no heap allocation
or kernel grant. TCP/IP may still retain the remote socket record and its original
accounting; this is terminal abandonment, without custody, retry authority,
rollback, physical cleanup or a local/remote refund. Ordinary Drop ignores that
reported admission error after the same one best-effort driver invocation. It
cannot begin a second attempt. Repeated admission rejection can therefore retain
server quota while the original owner domain stays live.

These are userspace calls without new local guards or IRQ manipulation. Ordinary
Drop still blocks under this policy. A containing destructor may already have
disposed independent fields; no broad parent-field, TLS-state-erasure or total
shutdown-latency qualification is added. Accepted uncertainty intentionally stops
the domain instead of returning to business code. Existing C18 documents this
scope, with no new registry, family, root, pin or recovery controller.

## Verification

The [version-controlled evidence](evidence/2026-10-22-security-synchronous-close.json)
records commands, source/model/artifact hashes, outcomes and failed runs separately.

All 203 host library tests pass. Ten new driver controls cover successful
pending-before-lease disposal; queue-full backoff carrying the remaining budget
into one wait; the one-second admission limit; non-queue admission error; clipped
backoff; accepted timeout without reentry; wait-admission/observation/service/
malformed-result distinctions; slow accepted submission exhausting the budget;
slow rejected submission remaining unstarted; and rejected-operation reentry.
Two wrapper tests require terminally retained fake owners with no destructor
work or second call. Abort adapters assert that their complete current owner is
live; host unwinding is not production domain retirement or field-disposal proof.

The scoped native fixture keeps a second unactivated UDP owner across the
existing five-second held-controller case, then closes it with the synchronous
driver. It also exercises explicit close and ordinary Drop for fresh TCP and UDP
sockets in both scoped roots. The existing CQ fixture confirms the actual
already-reaped Unavailable boundary through the same observer. No extra fault
sleep or wider request/fixture budget is introduced. Native HTTP and signed
central-store release exercise ordinary setup/error/shutdown socket scopes and
TLS/S3 client close success. They do not inject actual synchronous admission,
wait-admission, malformed-attachment or observation failure, and do not establish
progress through NIC failure, saturated admission or every parent destructor.

| Final run | Deferred tests (passed / failed / pending) | Observation |
|---|---|---|
| AArch64 security | 19 / 0 / 0 | Synchronous close marker in both scoped roots |
| x86-64 security | 17 / 0 / 0 | Same two-root close checks |
| AArch64 HTTP | 19 / 0 / 0 | All 14 host controls; healthy replies with stalled peers in 0.316–0.384 s; trickled-body rejection in 30.13 s |
| x86-64 HTTP | 17 / 0 / 0 | All 14 host controls; healthy replies with stalled peers in 0.249–0.352 s; trickled-body rejection in 30.43 s |
| AArch64 signed RustFS release | 20 / 0 / 0 | Accepted release; desired/ready replicas 1/1, deployment/service generations 1/1 |

Every runner exited successfully. The signed-release fixture first recorded one
refused ingress connection and two HTTP 409 responses with `{"error":-23}`,
then obtained its admission receipt and readiness. These intermediate results
remain in the evidence without attributing a cause. No runtime failure occurred
in this batch. The preceding report's failed close-policy runs and intermittent
UEFI timeout remain historical evidence; passing here does not resolve that
intermittence.

Strict host and AArch64 Clippy checks pass. Both signed service bundles build,
and the AArch64 current-source reproduction check verifies the signed bytes.
Native manifests match those bundles. Formatting, diff and model-runner shell
checks pass. The existing manual PDF is unchanged; only its LaTeX sources change.

The existing `CharlotteDeploymentClose` adds a one-owner blocking configuration.
It explores 69 states at depth 7. `RetainAdmission` represents terminal unstarted
retention; `UnsafeFallback` re-enables work from that state and must violate
`NoReentry`. Existing accepted-replay and premature-release negative controls
remain. The two-owner reactor configuration still explores 783 states at depth
12. Bound sources and atomic transitions are assumptions; exact deadline lengths,
actual Rust field cleanup, nonreused-ID allocation/reaping, kernel scheduling and
physical release remain outside this finite projection. The full suite checks
41 safe configurations and 118 expected counterexamples across 38 models.

## Remaining work

Socket open/listen and other controller/render work remain synchronous. Close
still blocks its caller, so it cannot supply service-wide progress when used in
a reactor's ordinary error or destructor path. Kernel entry/return latency and
whole-parent shutdown under pressure remain unqualified. Terminal unstarted
retention can consume remote quota; no recovery custody or protected operator
capacity is added. Packed immediate socket operations retain their separate
full-ID migration gap. SEC-06/07 flood, saturated-buffer, NIC-failure and broader
principal/heap-admission gates remain open.
