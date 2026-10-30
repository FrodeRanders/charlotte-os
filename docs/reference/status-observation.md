# Bounded status observations

The authenticated status service uses one `status_observation::Snapshot` for
every requested node or cluster snapshot. Its `Deadline` starts before source
lookup and report collection, with the compile-time `SNAPSHOT_MS` budget of
5,000 milliseconds. Optional startup lookups also share one bounded observation.
Header input, TLS/session lifetime and response output retain their own existing
budgets; this is not an overall request or cleanup deadline.

Name-service lookups, scalar diagnostics, moved snapshot replies and the
caller-buffered TCP/IP status call all use this same deadline. After expiry,
another source rejects before allocation or submission. Each accepted call
uses `PendingCall::wait_timeout` with the remaining budget. There is no reply
polling, per-source deadline restart or automatic replay. Sources use their
existing unavailable-data rendering; an unavailable cluster snapshot is HTTP 503.

The adapter preserves three outcomes: submission/preparation rejection,
observation rejection (including waiter admission failure), and timeout. The
HTTP renderer may omit an unavailable observation, but none of those outcomes
proves remote rollback. The original pending owner remains live through checked
cancellation. Its existing Drop policy confirms close or aborts rather than
ending an uncertain loan. Returned capabilities and mappings retain their
ordinary owned cleanup. A timeout provides no new physical cleanup authority,
retry owner or custody registry.

The native security probe calls this production adapter against an eight-slot
endpoint that never receives. It checks scalar and moved-buffer timeouts,
reuse of the already expired snapshot at full memory/IPC capacity, separate
queue and allocation rejection, exact one-page recovery after rejected or
unreceived moved requests, all eight queue slots after cancellation, and a
healthy caller-buffered status reply with returned-page cleanup. Its 100 ms
observation budgets and two-second outer bounds are fixture limits, not changes
to the production budget. The controls do not inject a rejected waiter,
receiver-held cleanup claim or mapping failure.

## Client progress and ownership

Status ingress rotates four client positions across listener setup, TLS/header
input, response output and logical close. Setup/accept and close reuse the
deployment service's owning reactors, with an explicit listener port. Input
retains a `management_io::Connection`, a fallibly prepared zeroizing 1024-byte
header buffer and one original five-second deadline. The existing exact bodyless
GET parser and enrolled Observe authorization select a typed target; there is
no malformed-prefix dashboard fallback. Readable inputs rotate behind other
positions, including after local TLS progress or a partial header.

Responses use the shared output owner, with a status-specific 512 KiB retained
response-capacity ceiling and the existing five-second output budget. Deployment
keeps its 64 KiB ceiling. Positive prefix receipts advance the same output;
only a complete response returns its total byte count. Pending TLS RPCs bind to
the same CQ as setup and close. Idle accepts wait without a timer; other waits
use the earliest remaining phase/TLS deadline or explicit queue-admission backoff.
No accepted request or output is replayed.

Shutdown stops replenishment and transfers each complete input/output owner into
the existing logical close pool. That adapter confirms cancellation before
destroying TLS state or consuming its socket. Every position, including pending
setup and close, must leave before cooperative shutdown completes. Uncertain
binding, setup or logical close follows the existing retain-and-abort policy;
neither timeout nor a logical receipt supplies physical cleanup authority.

The native host control requires healthy status replies within three seconds
beside one or three idle TLS peers and an authenticated incomplete header, three
repetitions each. Idle TLS peers must still be open without output when the
healthy reply completes. The unchanged serial server failed that distinction:
it replied only after the idle peer ended. Passing repetitions qualify finite
client progress, not sustained full-pool flooding or aggregate pressure.
The [control record](../reports/audits/evidence/2026-10-30-status-client-progress.json)
keeps the failed serial-server run separate from the corrected native matrix.

Snapshot dependency collection and rendering remain synchronous within each
completed request. A silent dependency can still occupy the observation budget;
the reactor does not make formatting, allocation, cryptography or receiver-held
cancellation preemptible. Those contexts and broader SEC-06/18 pressure remain
open. The [original observation report](../reports/audits/2026-10-28-security-status-observation.md)
retains its historical serial-handler scope.
