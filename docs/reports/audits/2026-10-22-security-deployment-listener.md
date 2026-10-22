# Deployment listener setup on the shared CQ

Date: 22 October 2026. Reviewed baseline: `48ba5f34`.

SEC-06 remains partial. Deployment ingress now observes socket creation and
listen setup alongside its other clients. Previously, replenishing a position
called synchronous `OwnedSocket::open` and `listen` before returning to the
reactor. A delayed reply could stop input, controller, output, close and lifecycle
observation together. A partial setup error could also invoke synchronous socket
Drop in that loop. The ledger remains 29 corrected, 6 partial and 3 open;
production trust remains disabled.

## Ownership and scheduling

`deployment_listener::Listeners` retains creation, listen and accept in the same
four client positions already shared with input, controller, output and close.
Reserving a position captures one five-second monotonic setup deadline before
any submission. A rotating turn advances at most one listener transition, then
the outer reactor visits the other processing phases and lifecycle state.

Every accepted phase binds its exact pending owner to the default CQ. The
reactor drains hints, checks all owners and waits for the earliest remaining
setup/input/controller/output/close deadline. An admitted idle accept has no
timer. No extra completion capability, unrelated wake source, accepted-call
poll timer or kernel readiness registry is added.

`OpeningSocket` retains the accepted creation request and the original service
connection borrow. Only a positive scalar reply without attachments constructs
its private `SocketLease`, exactly once. A negative scalar confirms rejection
and disarms the opening owner. Zero, attachments or observation failure retain
uncertainty. Incomplete opening destruction aborts before field disposal; a
socket ID is never guessed or reconstructed for cleanup.

The containing listener owner then retains that socket alongside each accepted
listen/accept call. `listen_call` consumes its caller-owned moved page through
the existing protocol and runtime; the owner still holds the socket through
reply observation. Both replies require zero without attachments for success.
A checked negative is a confirmed protocol rejection. Unexpected returned owners
follow the runtime's existing disposal policy; their presence never supplies
permission to continue setup or replay an accepted operation.

Only queue-full submission rejection retries a phase, using 50 ms admission
backoff under the original setup deadline. Queue admission has no readiness
producer. Confirmed service rejection or unstarted preparation/submission failure
removes that setup position and backs off replenishment. If its socket is known,
that socket moves directly into the existing CQ-bound close pool. Its position
therefore remains occupied until a checked logical close receipt. No synchronous
socket destructor runs on this normal rejection path.

Binding rejection, malformed/uncertain observation, setup expiry or incomplete
listener-owner destruction aborts with the complete current owner live. An
accepted setup request is never submitted again. These are guard-free userspace
operations with no new IRQ manipulation, root/pin, recovery custody, registry or
owner family. Existing C18 carries the cleanup contract. Neither cancellation,
a logical close receipt nor domain abort proves physical TCP/backing release or
refunds the receiver's original charge.

## Shutdown

Shutdown stops replenishment. An accepted creation or listen remains owned and
observed under its original setup budget. An observed new socket moves to close
without further setup. An idle accept is cancelled through its pending owner
before its known socket moves to close. Unsubmitted phases require no further
RPC. Existing closes retain their original deadlines while the other client
phases stay live.

After listener and close pools drain, the other phases still use their existing
shutdown destructors. Ordinary socket Drop can block; response preparation,
rendering, controller-server work and syscall return latency are separate
boundaries. This change does not establish whole-service shutdown latency or
remote rollback of a cancelled accept.

## Verification

The [version-controlled evidence](evidence/2026-10-22-security-deployment-listener.json)
records source/model/artifact hashes, commands, selected diagnostics and outcomes.

All 214 host library tests pass. Ten new controls cover a held creation beside a
healthy complete accept; one setup budget across phases; event-only idle accept;
queue-full-only backoff without accepted replay; delayed creation/listen timeout;
submission cost exhausting the deadline at accept admission; confirmed rejection
and replenishment cooldown; mapping/submission rejection returning a known
socket; binding/uncertain-observation retention in every phase; shutdown with
held creation and cancelled idle accept; and the existing position ceiling.
Abort adapters inspect retained fields before test unwinding. Host unwinding is
not native domain-retirement or physical cleanup proof.

The native scoped IPC adapter holds the first creation request while a second
position observes creation, its moved listen page, accept and logical close.
Only after the healthy close completes does it reject the held creation. It
requires exactly two creation calls, no extra queued request and zero remaining
positions, within a preset two-second fixture budget. Both scoped roots execute
this adapter. Its socket IDs are fixture metadata with no TCP backing; the HTTP
runs separately exercise the actual TCP/IP service and ordinary setup/accept.

All four native runners exited successfully with the final signed bundles:

| Run | Deferred tests (passed / failed / pending) | Observation |
|---|---|---|
| AArch64 HTTP | 19 / 0 / 0 | All 14 host controls; healthy replies beside stalled peers in 0.246–0.443 s; trickled-body rejection in 30.15 s |
| x86-64 HTTP | 17 / 0 / 0 | All 14 host controls; healthy replies in 0.269–0.360 s; trickled-body rejection in 30.38 s |
| AArch64 security | 19 / 0 / 0 | Listener IPC marker in both scoped roots; cancellation traffic retired after 13,764 requests |
| x86-64 security | 17 / 0 / 0 | Same two-root listener controls; cancellation traffic retired after 7,672 requests |

Both HTTP runs recorded keyhole connection resets and retries before later
successes. These intermediate diagnostics remain separate from the deployment
concurrency controls, which each use a single three-second observation attempt.
No native run failed in this batch. Passing cancellation repeats do not establish
a cause or resolution for the earlier x86 reclamation timeout.

Strict host library and both native service Clippy checks pass. The unfiltered
host test-inclusive check reports five inherited findings: two
`items_after_test_module`, one `unnecessary_result_map_or_else`, one
`byte_char_slices` and one `bool_comparison`. Excluding those four lint groups
for that check passes; no source lint suppression is added. Actual mapping/CQ
binding faults, malformed attached replies, saturated socket admission and a
held real TCP/IP setup reply are not injected natively.

`CharlotteDeploymentListener` checks two fixed positions and a two-tick setup
budget. The safe configuration explores 3,574 distinct states (6,709 generated),
with graph depth 23. All nine required actions have positive total coverage.
Three negative controls violate `AcceptedOnce`, `OriginalBudget` and
`ParkCoversAll` by replaying accepted work, restarting setup time or parking on
one pending call beside another ready reply. Binding/producer delivery and
atomic turns are assumptions. Shutdown, byte/attachment validation, dynamic
capacity across all five phases, TCP correctness and wall-clock fairness remain
outside this projection. The full suite checks 43 safe configurations and 123
expected counterexamples across 40 modules (166 cases, 207 hashed inputs).

Both signed service bundles build, and AArch64 current-source reproduction and
signature verification pass. Formatting, shell syntax and diff checks pass. The
manual PDF retains its existing hash; Markdown and LaTeX sources carry this update.

## Remaining work

Four occupied positions can still deny an operator. Sustained admission/flood
pressure, a protected operator slot, slow controller-server work, response
rendering, NIC failure and complete lifecycle shutdown remain open qualification
work. HTTP keyhole serving and other synchronous socket clients retain their
existing boundaries. This listener correction does not resolve the historical
x86 cancellation-stress reclamation timeout or establish aggregate principal/
heap admission. SEC-06/07/18 acceptance remains broader than this batch.
