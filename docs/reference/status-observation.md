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

The service still handles one HTTP connection at a time. This bounds its
dependency observation, without qualifying connection-flood fairness,
synchronous rendering, listener setup, blocking teardown or full SEC-06/18
progress. See the [execution report](../reports/audits/2026-10-28-security-status-observation.md).
