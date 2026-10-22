# Owned controller-server DNS relays

Date: 22 October 2026. Reviewed baseline: `5ade750b`.

SEC-06 remains partial. The controller server now observes slow DNS replies
alongside other management requests. All seven HTTP controls and named status
use one bounded owning relay, replacing their synchronous forwarding helpers.
The ledger remains 29 corrected, 6 partial and 3 open; production trust remains
disabled.

## Forwarding and ownership

`clusterctl_relay::Relays` owns eight positions. Each retains its complete
`IncomingMessage`, source read-only mapping, accepted DNS `PendingCall`, phase,
original deadline and checked returned result. Original incoming backing plus
one copied DNS request reserve at most 2,098,176 bytes across the pool, before
mapping/copying. Per-operation protocol bounds reject oversized incoming backing
or undeclared connection attachments. Admission rejection consumes that incoming
transaction once and replies without submitting DNS work.

This is a logical request reservation. Returned records, decoder allocations,
allocator metadata and copies retained by DNS after earlier phase completion or
cancellation are separate. Their original kernel accounts still charge live
backing. Ending a relay's local reservation supplies no physical refund.

Every accepted DNS phase binds to the controller CQ. Each server turn receives
at most eight requests, rotates relay observation and checks lifecycle before
another turn. The reactor drains CQ hints, rechecks exact owners and waits for
the earliest original relay deadline. Binding/wait rejection retains the
complete pool through terminal abort. No timer polling, new readiness source or
separate recovery registry is added. Raw receive treats `PENDING` like an empty
intake pass: the existing cancellation-claim producer re-signals endpoint/CQ
readiness when a fenced front can advance. The old adapter instead counted it
as productive intake. This follows the owned receive contract; native injection
of that episode in the controller loop remains separate.

One five-second server deadline starts before validation/preparation and spans
both notification preflight and submission. It is independent of the caller's
unchanged deadline; neither phase renews either budget. Preparation rechecks
expiry before submitting. Accepted calls are never replayed. Unstarted admission
failure reports routing rejection; accepted observation/malformed result reports
`dns::ERR_UNCERTAIN`. Timeout cancels the exact DNS owner and replies
`ERR_RPC_TIMEOUT`. DNS may still hold the moved copy and its original charge.

Returned connections are unsupported. Returned memory must fit its current
record bound and pass the operation's decoder before forwarding; named status
also binds the descriptor's full name to the query. Release receipts validate
against the retained original envelope. Consuming publication moves the returned
memory once. Unexpected returned owners follow existing runtime disposal.

Explicit finish and Drop cancel pending DNS work before ending the source
mapping and incoming reply/loan authority. Normal terminal reply also disposes
unused incoming backing and attachments before consuming the reply token.
Shutdown uses that same order. A cancelled incoming call, DNS call or reply is
not remote rollback or physical cleanup proof. These guard-free userspace paths
extend C18 without a new root/pin, owner family, custody or registry.

## Preflight correction

The earlier `current_deployment` returned `None` after transport failure,
missing/malformed returned memory or decoding failure. Notification then treated
that outcome as an absent record and proceeded to submission. The replacement
requires an explicit `dns::ERR_NOT_FOUND` scalar without attachments, or a checked
record for the exact requested artifact with an older revision. Same signed
bytes return the existing generation; stale/conflicting revisions reject.
Failures and foreign/malformed records never authorize mutation. The DNS leader
continues to perform authoritative signature, sequence, placement and admission
checks; this local query is not a substitute for them.

Host controls feed failed/malformed preflight results and require rejection
without the next submission. The old fallback is source-reviewed; its unsafe
decision is exercised separately by the model negative control. No native
DNS failure injection or committed-state corruption is claimed.

## Verification and failed runs

The [version-controlled evidence](evidence/2026-10-22-security-controller-relay.json)
records source/model/artifact hashes, commands, selected failure diagnostics and
separate final outcomes.

All 223 host library tests pass. Nine new controls cover held DNS beside a
healthy terminal reply; pending-before-mapping-before-upstream disposal; original
budget across preflight/mutation; expired/failed preflight without submission;
preparation/mapping/submission rejection; preparation time and binding rejection;
count/backing ceilings; protocol/returned-backing bounds; and explicit absence,
newer/same/stale/conflicting revisions and foreign/malformed records. Abort
adapters inspect live fields before host unwinding, which is no native retirement
proof. Strict host-library and both native service Clippy pass; test-inclusive
Clippy uses the baseline's four documented lint-group exclusions.

The first x86 run panicked at `self_test/el0_security.rs:110` with
`security probe failed check 8`. The existing grant fixture had published its
endpoint, then rejected the supposedly available lookup result. This happened
before the new relay adapter. The capture lacks that returned scalar/attachment
value; it establishes neither the rejection's cause nor a relationship to this
change. The same-source repeat passed this check but later reached the separate
fixture error below. A passing repeat cannot resolve the original failure.

The initial ARM run and that x86 repeat exited during the new adapter, after the
existing held-controller test. The adapter attempted a read loan to an endpoint
in its own address space. `memory::object::prepare_loan` rejects `owner == target`
with `WrongOwner`; the old fixture unwrapped failed admission into domain abort.
The outer verifier then panicked with `security probe exited before adversarial
application grant checks`. Those failed artifact identities remain separate.
The corrected adapter uses the protocol's moved-memory form; production relay
code and five-second budgets are unchanged. Native cross-domain loan behavior
is not qualified by this local adapter.

The corrected native adapter submits a held rollout, healthy rollout and policy
query through actual endpoints/CQ/owned RPCs. A well-formed moved rollout record
must reach its caller within two seconds while DNS holds the other request. An
unexpected returned connection must reject as uncertain. The held operation
must terminate under its original five-second budget within a seven-second
outer fixture allowance. DNS still owns its copied page afterward. Exactly three
DNS requests, no queued replay and zero final positions/reservations are required.
Both scoped roots execute it. This validates real IPC ownership and observation;
it does not simulate a real slow DNS/Raft server or prove physical backing release.

After correcting the moved request, intermediate ARM and x86 security runs
passed 19/0/0 and 17/0/0, with the adapter in both roots. Their marker still used
the stale phrase “incoming loan ended”; the actual call was Move, and that label
supplies no loan qualification. Final source corrects the marker and the raw
receive `PENDING` handling. The final source/bundles pass these combined and release runs:

| Final run | Deferred tests (passed / failed / pending) | Observation |
|---|---|---|
| AArch64 security + HTTP | 20 / 0 / 0 | Two relay adapters; all 14 host controls; healthy stalled-peer replies in 0.206–0.355 s; trickled body rejected in 30.16 s |
| x86 security + HTTP | 18 / 0 / 0 | Same controls; healthy replies in 0.278–0.399 s; trickled body rejected in 30.23 s |
| AArch64 signed release | 20 / 0 / 0 | Accepted release, deployment/service generations 1/1 and desired/ready replicas 1/1 |

Combined runs also retire cancellation traffic after 18,696 and 15,228 requests.
These are concurrent fixture runs, without a controlled all-position/byte or NIC
saturation contract. HTTP keyhole connection-reset retries remain in the evidence;
each deployment concurrency probe uses its preset single-attempt three-second
budget.

The release fixture first reports `Resource temporarily unavailable (os error 35)`
while reading the ingress response, explicitly warning that submitted work may
be committed. A later fixture attempt obtains the release receipt and generation-1
readiness. That success does not establish whether the first attempt committed,
prove rollback or justify general replay. The host fixture's retry/reconciliation
policy requires separate qualification; this relay submits each accepted phase
once per incoming transaction.

Both signed bundles build; AArch64 current-source reproduction/signature checks,
formatting, shell syntax and diff checks pass. The manual PDF is unchanged.
The full formal suite passes 44 safe configurations and 127 expected
counterexamples across 41 modules (171 cases, 213 hashed inputs).

`CharlotteControllerRelay` starts with one admitted preflight/mutation and one
query, under a two-tick budget. Its safe configuration explores 684 distinct
states (2,281 generated), depth 12, with all ten required actions exercised.
Four negatives violate `MutationRequiresCheckedQuery`, `OriginalBudget`,
`ParkCoversAll` and `AcceptedOnce`. Binding/producer delivery, atomic turns and
classification of a checked query are assumptions. Admission, bytes/signatures,
actual returned owners, shutdown and wall-clock fairness are outside the model.

## Remaining work

Legacy upload, join and key handlers still run synchronously on the same endpoint
and can delay these relays. Validation/signature work is bounded but synchronous.
DNS/Raft execution, all-position saturation, protected operator capacity, response
rendering, NIC/flood pressure and complete shutdown latency remain open. Actual
map/bind faults, cross-domain incoming-loan cleanup and host-fixture recovery
after uncertain admission are separate qualification.
The initial x86 publication-lookup failure and historical cancellation-stress
reclamation timeout remain unresolved observations. No physical retry or charge
refund is inferred from a passing repeat or this new local reservation.
