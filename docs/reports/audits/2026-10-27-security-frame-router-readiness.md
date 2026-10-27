# Frame-router readiness and management availability

Date: 2026-10-27. Parent: `9008c95b`. Follow-up to
[management leader loss](2026-10-27-security-management-failover.md).
SEC-10 remains partial. Finding counts remain **29 corrected in scope, seven
partial and two open**; production remains disabled.

## Observed delay and correction

A traced parent-bundle repeat again fails on a healthy reader at node 2's direct
status endpoint, after leader replacement and signed policy generation 2. Its
first SYN is visible at host monotonic `2110074.714585`; the SYN-ACK arrives at
`2110084.515970`, **9801.385 ms** later. This run completes TLS and receives data,
then fails reading the response at the original 12-second budget. It is a
different transport outcome from the earlier handshake timeouts, not an
authentication rejection. Both survivors subsequently campaign through term 7.

The frame router polled its pending receive and other asynchronous replies in
50 ms sleep steps. It now registers all those interests on the existing CQ and
uses the original pending owners to observe their results. Installed consumer
routes refresh at the existing 1000 ms assignment/membership cadence; absent
consumer lookups retain their deferred subscription. Successful lookups do not
create an immediate completion/resubmission loop. Rejected admission waits for
the next refresh, and unrelated wakes do not restart that deadline.

Every accepted receive, lookup, control query, moved-frame forward and L2 send
is CQ-bound. Binding or wait rejection retains accepted work through the shared
fail-closed policy; it cannot authorize replay. The existing snapshot-lease
completions and serving endpoint share the queue. No resource owner is rebuilt
from a scalar ID. Configuration and remaining contexts are documented in the
[readiness contract](../../reference/frame-router-readiness.md).

## Native results

The corrected full campaign completes all eight direct-node controls after
leader loss: unknown/missing client on both management interfaces at both
survivors, each with a healthy reader before and after. Unknown-client controls
require a new native policy-rejection event. No unauthorized application byte,
server-verification failure or timeout is accepted as rejection proof. The
first and second signed policies are each submitted once, with reader mutation
denial and administrator admission through a follower.

The corrected full run still **fails** at VIP recovery. During recovery from the
unknown-client probe, a healthy-reader SYN for `10.0.2.42:443` is forwarded to
`52:54:00:12:34:03`, the killed node 0, including its retransmission. There is no
SYN-ACK for that flow. This establishes selection of the unavailable backend for
this run; the earlier untraced VIP timeout retains its original uncertainty.
The committed three-voter configuration still includes that member. Automatic
backend availability/drain policy remains a separate gap. No flow rehash or
accepted-request retry is added to make the campaign pass.

A separate **direct-only repeat passes to completion**, again with all eight
controls, term-1 to term-2 replacement, reader mutation denial, administrator
receipts 1 then 2 and matching generation/sequence-2 readback on both survivors.
All three initial kernel suites pass **18/0/0**. No read-observation retry is
needed, and no later term change is logged during the controls. Its kernel is
`5c6fad6ba8256fe110d29b65724d1a3814497ccb976a890896b6647e69dcb6ce`;
the failed corrected full run uses
`846f51815a32c4cf43dc01b2dbcf03ee459cf74fd088afc960e2b50846afc3c7`.
These are separate scopes; the direct pass never changes the full run's result.

Among 43 completed direct TCP handshakes in that corrected run, observed
SYN-to-SYN-ACK median/max are **54.800/69.830 ms**, compared with
**282.541/9801.385 ms** across 27 in the traced parent-bundle run. These are
observations from disposable development runs, not a controlled benchmark or
latency guarantee. Resources and keys are regenerated; artifact hashes record
the actual binaries. The rebuilt bundle changes DNS, Raft and frouter ELF
hashes. The source change in this batch is confined to frouter; no election,
request or handshake budget is raised. The result does not establish one cause
for every earlier timeout or election disturbance.

The [tracked evidence](2026-10-27-security-frame-router-readiness-evidence.json)
retains the failed baseline, failed full campaign, completed direct controls,
packet metadata, native diagnostics, the passing direct-only repeat and artifact
hashes independently. Reproduce with the existing fixture, adding
`--trace-tcp --trace-limit 4096` for bounded packet metadata and `--direct-only`
only for the explicitly narrower campaign. Neither trace mode records payloads;
`--trace-limit` bounds each trace class and defaults to 512. The shared hub also
exposes `--trace-tcp-data` to include data-packet metadata when TCP tracing is on.

## Validation and limits

All **254 service-library tests** pass, including TLS interoperability. The first
sandboxed attempt passes 251 but its three socket fixtures cannot bind loopback
(`PermissionDenied`, OS error 1); the authorized repeat passes all 254. The
initial traced native attempt is likewise blocked before guest startup by the
hub's loopback bind. These infrastructure failures are separate from native
transport failures. ARM and x86 service bundles rebuild, with strict Clippy for
the actual frouter service targets. Observer/trace host tests, rustfmt,
Python syntax and whitespace checks pass. No fresh full host-suite, TLC,
native x86 multi-node or fault/pressure campaign is claimed.

The normal frouter receive loop now has actual reply notifications. TCP/IP's
timed transport cadence, initial/terminal NIC discovery, driver receive-queue
count/byte admission, forced shutdown, cancellation/binding rejection under
pressure and hardware restart remain separately qualified contexts. The
historical firmware intermittence remains unresolved. Shared development
service credentials, unauthenticated L2/SNTP, protected custody/anti-rollback and
live TLS-session retirement keep their prior scope. No numbered finding closes.
