# Security audit: host ingress operation deadlines

Date: 2026-10-21. Reviewed baseline: `f3b8aaf4`.

The signer now carries one checked monotonic deadline from file preparation
through admission and readiness observation. Its ingress commands share one
transport that bounds connect, partial writes and response reads by the time
left. Endpoints must be numeric IPv4 or bracketed IPv6 with an explicit nonzero
port. The [tracked evidence](evidence/2026-10-21-security-ingress-deadline.json)
records baseline failures, corrected results, timings and source/artifact hashes.
Findings remain **29 corrected / 6 partial / 3 open**; production trust stays
disabled.

## Baseline observations

The preceding release-generation work retained the admitted component generations,
but host operation timeouts still began after notification. Each connect used
blocking name resolution/connection establishment. Writes used no timeout on
status requests and a fresh per-write timeout elsewhere. Response reads used a
new deadline derived from their phase timeout. Descriptor batches admitted every
component before starting a separate readiness budget. Polling always slept one
second, even with less time remaining. User-controlled `Instant + Duration`
addition could panic before observation, including after successful admission.

The new [CLI controls](../../../scripts/tests/test-ingress-deadline.py) use the
actual signer and signed public fixtures against an owned loopback HTTP peer.
The baseline binary matches the preceding report's release-binary hash. Its
seven-method, nine-case matrix produces **eight expected failed controls and one
pass**: four expose deadline overruns/false success, three expose extreme-timeout
panics, and one records the intentional hostname-policy change. Matching receipt
and readiness progress passes.

A delayed admission followed by delayed readiness completes successfully after
the requested one-second budget. Two sequential delayed descriptor notifications
also complete successfully after that budget. The stale-status polling case
takes about **1.718 seconds**, and two trickled status replies take about
**10.009 seconds**. The release, descriptor-batch and named-status commands all
panic with `overflow when adding duration to instant` for `u64::MAX` seconds.
The exact diagnostics remain in the evidence. An earlier harness run stopped the
trickle case at its nine-second subprocess ceiling; the separate 13-second
ceiling run lets the baseline finish and preserves an ordinary failed timing
assertion. That harness cutoff is not a second production cause.

## Correction and ownership

The private `http_client` module holds `Deadline`, numeric endpoint admission,
remaining-budget writes and the existing bounded response decoder. The response
framing implementation moves unchanged apart from visibility. Checked clock
addition rejects zero operation budgets and out-of-range durations. Apply
captures the deadline before reading its descriptor/release files, checks it
between descriptor preparations, and passes it through every notification and
ready-state query. A phase cap can shorten that deadline; it cannot extend it.
Notification exchanges retain a 30-second cap and status exchanges a five-second
cap. Polling sleeps for at most the remaining operation budget.

Every ingress command uses `TcpStream::connect_timeout` with a parsed
`SocketAddr`. Hostnames, URLs, missing ports and port zero reject before network
access. IPv4 and bracketed IPv6 literals are the current grammar. Operators may
resolve names before invoking the tool; no resolver thread or older blocking
fallback is retained. CLI positional arguments, HTTP routes and header grammar, request bodies,
signed inputs, receipt encoding and runtime ABI are unchanged. The Host field
uses the parsed numeric address.

The shared exchange owns one host socket through connect, request header/body
submission and complete response observation. Each partial write and interrupted
retry recomputes the remaining budget while preserving its existing byte offset.
Each response fragment also uses the time left, and completion is checked against
the deadline before returning the response. A timeout after a submitted prefix
leaves work unconfirmed; no notification is replayed. Errors close the socket
through ordinary ownership and do not roll back remote work. Prepared public
inputs, admitted generation pairs and borrowed names remain in their existing
containing scopes. The [C18 row](../../reference/cleanup-recovery.md) records this
host context. No worker thread, Charlotte capability/root/pin, registry, recovery
family, custody or physical completion proof is added.

Standalone notify commands and ingress-policy status use a single 30-second
budget. Named deployment status uses its supplied positive wait budget, or one
five-second query for zero/omitted wait. It still observes the current named
record; this work does not bind that standalone observation to an admission or
replace its ready substring check. Apply retains its complete generation-safe
status decoder. See [tooling inputs](../../reference/tooling-inputs.md).

## Validation and limits

Both host profiles pass **81 Rust tests**: 53 unit tests and 28 existing
integration controls. The six new Rust controls cover endpoint grammar, checked
clock arithmetic and phase caps, no socket after expiry, partial/interrupted
writes, expiration before further submission, and terminal write errors without
prefix replay. Strict Clippy and workspace formatting pass.

Both profiles also pass **35 live CLI cases**: the nine deadline cases plus the
13 descriptor-generation and 13 release-generation cases. The four delayed
corrected scenarios return in roughly **1.004–1.009 seconds** against a one-second
budget, within the harness's scheduling margin. The peer checks POST counts,
accepted ordering and no status query after a failed batch notification. Numeric
endpoint and extreme-timeout rejection perform no network access. The evidence
retains individual timings, request sequences and CLI results. Test peers own and
join their server threads; production transport creates no background workers.

The signed AArch64 service build and freshness check pass. All **41 AArch64 and
37 x86_64 staged images** retain the preceding hashes. All **180 formal inputs**
retain their hashes. No kernel/service source or model transition changes, and
this batch makes no fresh guest, x86 build or TLC execution claim. Historical
reports and the manual PDF are unchanged.

The deadline is an admission/progress budget, not a hard real-time cancellation
mechanism. Blocking host filesystem calls and stdout are not preempted; a stalled
file read can delay return before network access. OS scheduling and socket timer
precision can also delay observation of expiry. The tests do not inject a live
blocked connect, exhausted kernel send buffers or host filesystem stalls. Read
and write faults are qualified through selected deterministic controls and the
scripted peer, without a general platform-latency claim.

Readiness still polls because this HTTP interface has no subscription. Peer
authentication, standalone named-status parsing, server-side progress/teardown,
coordinated rollback, production trust and existing audit findings remain
separate work. The boot runners' startup retry policy is unchanged; an uncertain
command outcome never supplies general permission to replay accepted work.
