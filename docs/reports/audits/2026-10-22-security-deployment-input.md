# Deployment ingress input isolation

Date: 22 October 2026. Reviewed baseline: `3e93d259`.

SEC-06 remains partial. This change removes deployment input's single-client
serialization: a client holding an incomplete header or upload no longer
prevents another admitted input from progressing. Controller execution,
response transmission, close and saturated admission still need qualification.
The finding ledger remains 29 corrected, 6 partial and 3 open. Production trust
remains disabled.

## Implementation

`deployment_input::Inputs` owns four fixed receiving slots. Spare positions have
preposted listening sockets, keeping listening plus receiving positions bounded
to four. Each input retains its socket, accepted receive, request prefix and
original deadlines. The pool
advances one ready input per turn from a rotating cursor. Prefixes share one
`MAX_REQUEST_BYTES` limit, 1,049,088 bytes including header allowance. Four caller-sponsored receive pages,
TCP buffers and allocator overhead are separate from that byte count. Rejection
occurs before appending bytes above the shared limit and preserves other inputs.
No attacker-sized second body copy is introduced.

Header and total deadlines are five and thirty seconds from observed accept
completion, respectively. Neither input progress nor an unrelated wake restarts
them. These are reactor observation budgets: synchronous controller/send/close
work and kernel scheduling can delay expiry handling. They do not establish
hard wall-clock service bounds or begin at the wire handshake timestamp.

TCP/IP now retains the complete authenticated request for `OP_ACCEPT` (15).
Listen and SynReceived remain pending; Established and CloseWait complete with
scalar zero and no attachments. Terminal states or explicit close reject.
Existing immediate callers use the distinct `OP_TRY_ACCEPT` (5). Deferred
receive drains buffered bytes before delivering peer EOF. Both operations use
the existing socket entry and exact authenticated owner; no registry is added.

Accept and receive pending owners bind to the default CQ. The loop drains CQ
hints, checks lifecycle and exact owners, then waits for the earliest remaining
input deadline, or event-only when idle. Spare listeners are prepared before
observing accepts, and a freed input position becomes a listener before business
work. An accept therefore does not leave the whole service without a listening
socket while spare positions exist. Listener setup rejection retains a
50 ms admission backoff because that rejected operation has no readiness source.
TCP/IP's enclosing timed reactor cadence remains a separate boundary; this
change does not qualify its whole polling/transport path as event-driven.

The existing C18 userspace transaction family now contains `Input` and
`Accepting` owners. Pending calls precede sockets in field destruction order.
Ordinary input completion, parser/mapping/allocation failure, EOF and deadline
expiry dispose the exact accepted call before returning the socket to existing
explicit close. Accepted-call binding failure or CQ wait-admission rejection
aborts with the containing owners live. Neither rejection authorizes replay,
physical cleanup or a fabricated timeout. No general cleanup custody or retry
controller is added; synchronous remote teardown retains its previous policy.

## Verification

The version-controlled [evidence record](evidence/2026-10-22-security-deployment-input.json)
contains commands, artifact hashes, observed timings and separate failed runs.
The host library suite passes 160 tests. New tests cover healthy input beside
three stalled peers, rotating turns, original header/body budgets, aggregate
prefix rejection, full slot admission, initial/rearm submission rejection,
EOF/mapping-result rejection and pending-before-socket destruction. Producer
state tests cover handshake completion and data-before-EOF. Mapping and remote
owners in the pool tests are modeled adapters; actual mapping rejection and
accepted CQ binding rejection are source-reviewed, not native fault injections.

Native HTTP probes on AArch64 and x86 hold one incomplete header, one incomplete
body, then three mixed stalled peers. Each case requires one healthy status
response within three seconds and verifies the stalled peers have not completed.
They also retain malformed framing, invalid signed-body, EOF, abortive-close,
idle-client and thirty-second trickled-body recovery controls. The signed
central-store flow checks upload, atomic release admission, pull, verification,
launch and named readiness. Public development keys and plaintext fixture
transport qualify behavior only; they do not establish production enrollment.

`CharlotteDeploymentInput` adds a finite four-slot safety projection. It checks
aggregate bytes, unchanged budgets and a ready healthy input's intervening turn
bound. The safe configuration explores 684 states at depth 17. Keeping a busy
input's cursor violates `HealthyTurnBound`; restarting a request budget violates
`OriginalBudget`. The model assumes atomic input turns and producer/CQ delivery.
It omits TCP, dynamic admission, controller/send/close latency, kernel scheduling
and physical teardown. The conformance map records those limits. The full
harness passed all 37 safe configurations and 111 named expected failures,
including nonzero coverage of each required new action.

The first one-listener implementation passed both architectures but a stricter
AArch64 and x86 repeats reset the healthy connection beside an incomplete header.
Source review identified an admission window between observing the prior accept
and submitting its replacement listener. The correction preposts every spare
position instead. That failed corrected-artifact run and its earlier passing
runs remain separate in the evidence. Host load may affect the size of the
window; it is not established as the cause of the observed reset.

An early baseline host-probe failure is retained separately from corrected
passes. Replaying its preserved boot image on isolated ports passed a normal
status control, then the healthy request beside the first incomplete header
failed with `ConnectionResetError: [Errno 54] Connection reset by peer`. The
negative control admits no transport retry; the serial listener had no second
listening socket during that client input. A later validation mistakenly overlapped service restaging: the kernel
rejected the unsigned/incompletely signed `frouter.elf` and panicked at
`service/launch.rs:489`, before ingress qualification. That run is invalid test
setup, not a passing native check or evidence of an ingress defect. Complete
signed-bundle runs are recorded separately; no repeat erases either failure.

## Remaining acceptance

Four hostile inputs can occupy every slot until expiry. There is no per-peer
reservation, authenticated early admission, flood-rate policy or protected slot
for an operator. Prefix-budget rejection is bounded allocation policy, not fair
admission. A controller RPC, response send or close can still serialize other
inputs, and TCP/IP open/listen/close calls have unqualified accepted-call waits.
The HTTP keyhole listener remains serial. General principal/heap admission,
NIC failure, sustained packet/connection floods and bounded shutdown under
pressure remain SEC-06/07 work. Input isolation is a substantial scoped step,
not closure of either finding.
