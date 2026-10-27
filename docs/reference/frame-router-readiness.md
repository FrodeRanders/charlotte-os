# Frame-router readiness

The normal `frouter` receive reactor binds its NIC receive, deferred consumer
lookups, DNS assignment/membership queries, forwarded-frame replies and L2 sends
to CQ 0. Its status endpoint and owned snapshot-lease completions signal that
same queue. It drains hints before checking the current owners, then waits when
the outstanding NIC receive is not ready. A CQ entry never observes returned
memory/connections or authorizes replay; each original pending owner does that.

Successful route lookups refresh periodically instead of immediately submitting
another lookup on every completion. An absent publication keeps its existing
deferred lookup. Ordinary rejected lookup/control admission waits for the next
refresh opportunity; accepted calls are never replayed. Rejected CQ binding or
checked wait admission aborts with the complete reactor and accepted work live.

The existing `catten_services::frouter::SNAPSHOT_REFRESH_MS` in
`crates/catten-services/src/lib.rs` supplies the shared **1000 ms** route,
assignment and membership cadence. `SNAPSHOT_SOURCE_MAX_AGE_MS` derives from it,
and `SNAPSHOT_LEASE_MS` is five times that interval. These are compile-time
settings requiring a service rebuild/restart. Review registration refresh lag,
fresh quorum observations, VIP admission lease and timer/waiter admission
together when changing them. Unrelated CQ wakes retain the refresh deadline.
The former independent 50 ms receive-poll interval is removed.

TCP/IP still advances its transport state on its own timed cadence; NIC hardware
and other service reactors retain their separately documented contexts. This
does not qualify the entire network stack as event-driven. Initial NIC lookup
and the existing terminal-receive/reconnection adapter remain separate from the
normal receive-loop qualification. No automatic membership removal, backend
drain, existing-flow reassignment or TCP/TLS replay is introduced.

The pending owners, moved frame backing, socket/endpoint authority and original
charges keep their existing cleanup policies. No new recovery owner or registry
is created. Shutdown, CQ admission rejection, source restart and device fault
qualification are distinct from healthy progress under the management fixture.
See [cleanup family C18](cleanup-recovery.md) and the
[native follow-up](../reports/audits/2026-10-27-security-frame-router-readiness.md).
