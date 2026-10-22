# Deployment controller reply observation

Date: 22 October 2026. Reviewed baseline: `debef33e`.

SEC-06 remains partial. The previous input-isolation change left all seven
controller RPCs blocking the deployment HTTP loop. This follow-up retains each
accepted request as an owning CQ-bound operation, so one unanswered controller
call leaves other inputs and ready controller replies able to progress.
The ledger remains 29 corrected, 6 partial and 3 open. Production trust remains
disabled.

## Ownership and scheduling

`deployment_control::Controls` shares four active positions with listening and
input phases. Each `Operation` owns its accepted pending call, socket,
reply-decoding kind, retained release envelope, original deadline and logical
request reservation. The executable keeps accepting/parsing input and observing
all controller sources on the default CQ. Its wait uses the earliest remaining
input/RPC deadline. It does not wait synchronously on an individual controller.

All seven controls submit exactly once. Five-second RPC budgets start before
request validation and memory preparation. Binding rejection after submission
aborts with the complete operation pool live. Ordinary completion, observation
failure and expiry dispose the exact pending call before handing its socket to
the existing response/close policy. Timeout returns HTTP 503 with code -22;
it cannot authorize mutation replay or prove remote rollback. Accepted-call
observation rejection returns uncertain -5 instead of a routing rejection. Release receipt
validation retains the original envelope through reply observation. Returned
memory and connection owners are consumed or dropped through the runtime;
unexpected attachments to scalar results reject as uncertain.

Input prefixes and active controller reservations share 1,049,088 bytes. A
reservation counts rounded moved-request pages plus retained release envelope
capacity. Preparing a request rejects before allocating beyond the available
budget, and subsequent input growth cannot consume those reserved bytes.
Temporary copying, receive pages, result memory, allocator overhead and TCP
buffers are separate. A locally completed or cancelled call releases only its
logical active reservation. A receiver may retain delivered memory and its
original kernel sponsor charge after caller timeout; no physical or accounting
refund is inferred. Actual kernel admission still applies to that backing.

This extends the existing C18 userspace transaction family. It adds no kernel
root/pin, custody registry, physical completion proof or retry controller.
Preparation, rendering, TCP send, socket setup/close and implicit runtime
cleanup keep their existing context and fail-closed policies.

## Verification

The [version-controlled evidence](evidence/2026-10-22-security-deployment-control.json)
records commands, hashes, validation outcomes and any failed runs separately.
The host library suite passes 170 tests. Added cases cover independent ready
reply observation, unchanged deadlines through unrelated wakes, timeout without
replay, admission/byte-budget rejection, full operation capacity, terminal
observation rejection, pending-before-socket disposal, and reservation rounding.
Malformed mutations reject before kernel memory preparation. A binding-rejection
adapter checks that the operation owners remain live at the abort boundary;
host unwinding is not production teardown qualification. Actual mapping and
binding rejection are source-reviewed adapter paths, not native fault injections.

The native scoped-security fixture uses the actual controller pool and owned
TCP/IP sockets. Two moved rollout requests enter a private endpoint. The fixture
retains one complete incoming request, replies to the other and requires the
healthy 404 within two seconds while the first call remains pending. The first
then expires at its original five-second budget with uncertain 503/-22. Its
received memory remains owned until explicit fixture release; no additional
request may appear in the endpoint. This exercises real accepted calls, CQ
binding, returned observation and cancellation without polling sleeps. It
qualifies the ingress-side owner/notification contract; the private endpoint
supplies deferred serving and does not establish real `clusterctl` concurrency.

Native HTTP regressions retain incomplete header/body/mixed-client isolation,
framing rejection, EOF/reset/idle recovery and the thirty-second trickled-body
budget. Signed central-store release validates the unchanged request framing,
receipt binding, pull, verification, launch and named readiness. Development
fixture trust and plaintext transport do not establish production enrollment.

`CharlotteDeploymentControl` adds a two-call finite safety projection. Its safe
configuration explores 688 states at depth 10. A serial wait beside a ready
reply violates `ParkCoversAll`; replay after timeout violates `SubmittedOnce`.
The model separates caller expiry from receiver backing release and retains
all owners through rejected admission. Preparation, active-byte admission,
reply bytes, dynamic clients, server execution, response latency, kernel
scheduling and physical teardown are outside the projection. The conformance
map records those limits.

The first expanded security run passed the new CQ case, then expired the old
30-second aggregate observer budget at `self_test/el0_security.rs:108` while
waiting for the full adversarial application checks. The new case adds a
serial five-second fault wait. Only that fixture's full-check observer budget
is now 45 seconds; the helper default and all production budgets remain
unchanged. The initial failed run and subsequent corrected runs remain separate
in the evidence. This is a fixture scheduling correction, not a claim of a
production deadline defect.

## Remaining work

Response rendering/send and TCP/IP open/listen/close remain synchronous. Their
accepted-call scheduling, backpressure and teardown can still delay all clients.
The real controller can serialize its own DNS/storage/Raft work. Four occupied
positions or large active request reservations can deny a healthy client; no
protected operator slot or per-peer flood policy is added. Controller-connection
generation recovery remains its existing outer policy. General principal/
heap admission, sustained packet/connection floods, NIC failure and bounded
whole-service shutdown under pressure remain SEC-06/07 work.
