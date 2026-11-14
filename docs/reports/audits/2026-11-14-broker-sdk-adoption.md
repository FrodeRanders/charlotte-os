# Broker SDK adoption and manual client review

14 November 2026. This follows the [consumer review](2026-11-14-broker-sdk-review.md).
The broker demo is adapted to CharlotteOS's existing ownership/reactor model.
Kafka wire behavior, pure partition state and dispatch remain application code.
This checkpoint does not expand the broker's production or Kafka feature claims.

## Changes and current limits

The EL0 adapter uses `charlotte-app::Application`, its returning entry wrapper,
owned socket operations and one CQ-zero network loop. At most eight sessions
retain their accepted calls, original request deadlines, sockets, input frames
and confirmed output prefixes. Sockets borrow one enclosing connection; no
process-lifetime leak, bare socket-ID handoff or session thread is needed.
Listener setup precedes signed readiness publication. An incomplete frame does
not prevent other sessions progressing. Only checked positive send receipts
advance output, and timeout cannot authorize replay of accepted sends.

Partition workers keep their actual Sitas handles. Startup prepares channels
and join slots before worker publication. Shutdown closes network scopes,
observes partition acknowledgements, waits for terminal exit observation and
consumes joins. A failed shutdown is fenced, retains unjoined handles and cannot
later report success. The EL0 caller aborts with its engine owners retained on
failed drain. Host fallback retains uncertain workers rather than pretending
that dropping an observation stopped them.

The demo consumes a single exported source SDK through relative workspace
paths. Its resolver checks the pinned revision/schema/toolchain and exact file
hashes, including paths containing spaces, and stages mutable signer build inputs
outside the SDK. It keeps its existing workspace and uses the public
manifest/binary builder. The OS exporter adds `charlotte-kafka` and `sitas-unix`
for host conformance, accepts an explicit read-only Sitas source, and pins the
hardened Sitas backend at `6d63266eb52492c1958c45f3ed71c8c7a451489a`. That backend
captures spawn generations, handles rejected mandatory startup terminally and
owns its exit observations. No broker-specific kernel API is introduced.

Sitas mandatory worker spawning remains infallible at its trait boundary, with
terminal rejection in the native backend. Shared socket transfer between
`'static` threads remains an extension for applications requiring that model.
The demo instead keeps borrows in one bounded application scope. Portable
business dispatch still has its documented bounded mailbox/reply retry policy;
it can occupy the network thread and is not preempted by a session deadline.
The 30-second network operation budget does not bound blocking destruction or
rollback remote effects. Long guest soaks and native x86 execution were not
requalified here.

## Host, build and SDK validation

The adapted broker passed 58 default-workspace Rust tests, three additional EL0
host tests, six platform/SDK tests, denied-warning host Clippy and public API
documentation generation. The delayed-exit regression holds partition workers
after their acknowledgements and verifies shutdown waits for actual joins.
Another test checks failed shutdown remains fenced with its worker retained.
EL0 host tests cover fragmented/pipelined framing, rejected lengths and advertised
port parsing. SDK tests reject modified files, wrong revisions and incorrect
archive sidecars while preserving previously selected inputs.

The exported SDK alone built the AArch64 and x86-64 broker ELFs, each with three
non-overlapping, non-W+X LOAD segments and entry `0x20000`. The full OS host suite,
application/runtime Clippy and workspace formatting passed. The newly added
runner gate also passed its focused tests: combined deployment/shutdown waits
for exact-run readiness and the host probe window before submitting shutdown.

## Native deployment and drain

The final AArch64 TCG fixture used `--deployment-ingress-test`,
`--shutdown-ingress-test`, a 480-second runner window and a 20-second host probe
window after readiness. The signed broker descriptor granted `tcpip=client` and
`broker=publish`, eight stack pages per thread, 16 threads and 5000 ms shutdown
grace. It followed the existing signed upload, atomic-release admission, artifact
pull, verification, scoped launch and readiness path.

A host codec client completed API versions, metadata, produce, fetch and
list-offset checks while a second TCP client retained an incomplete 100-byte
frame. The successful result was:

```text
remote smoke ok: partition 0 base offset 0, high watermark 1, latest 1
Concurrent incomplete-frame and active client passed.
```

Relevant final guest diagnostics:

```text
[17.790031] SELFTEST COMPLETE: passed=20 failed=0 pending=0 passed_bitmap=0x303ffff failed_bitmap=0x0 pending_bitmap=0x0
[24.763229] [broker] listening on tcp port 9092; readiness published
[25.697149] [broker] accepted connection; active=1
[26.199943] [broker] accepted connection; active=2
[49.680558] [broker] partition workers joined; network scope released
[49.716635] [shutdown] TcpIp: acknowledged=1 unacknowledged=0 forced=0 lifecycle_status=2
[49.731221] [shutdown] POWER-OFF REQUESTED via PSCI Hvc
PASS: active status/deployment clients drained before cooperative retirement and PSCI
```

The coordinated shutdown test observed the broker's own join marker before its
network dependency retired and before poweroff. This is distinct from the
coordinator's 20 regression verdicts. The earlier development deployment also
passed the traffic check and 20/0/0; it did not exercise cooperative shutdown.

Artifact SHA-256 identities for the final drain run:

| Artifact | Digest |
| --- | --- |
| AArch64 test kernel | `9d49bb7e775498cd12ae370a0da16ef2ac923088a6cc821a376c7b39f3febbdb` |
| Signed AArch64 broker ELF | `47be49188b109179689d747198db92636991ecfaab82019d14a83f222f722a94` |

The preceding review preserves the original broker baseline and sandboxed TCP
listener failure separately. Successful repeats do not erase those captures.
Temporary logs and build directories are not durable evidence dependencies;
relevant identities and diagnostics are recorded inline.

## Manual example

“A client with explicit recovery scopes” includes the actual
`charlotte-app/examples/reconnecting_client.rs` source. The review replaced the
misleading `ServiceRestarted` name with `EndpointLost`, separated unstarted
`DeadlineExpired` from `SessionLimit`, retained rejected-wait details and used
shared lifecycle-aware admission with the original operation deadline. Accepted
timeout/cancellation still stops the client, with explicit owner-preserving
cleanup and terminal abort on rejected cancellation. A replacement publication
never retargets the old connection.

The common application prelude includes launch/lifecycle and IPC result/error
types, reducing listing setup. Four host policy tests and both freestanding
target builds passed. The manual rebuilt twice with resolved references; the
included listing pages were rendered and visually inspected. These checks do
not claim execution of a deployed reconnect/restart/cancellation episode.
