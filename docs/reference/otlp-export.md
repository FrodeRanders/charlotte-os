# Native OTLP metric export

The optional `otlp` userspace service pushes current node measurements to an
explicitly configured OpenTelemetry collector. It reads observe's bounded
in-memory history over owned IPC. Sampling, placement reporting, local
archiving and remote export are independent; export has no disk spool.
No configured destination means no exporter domain is launched.

```sh
CATTEN_OTLP_ENDPOINT=http://192.0.2.20:4318/v1/metrics \
  scripts/run-aarch64.sh release --minimal
```

Both runners and direct kernel Cargo builds accept these inputs:

| Build input | Selection |
| --- | --- |
| `CATTEN_OTLP_ENDPOINT` | Unset disables export. Explicit `http://IPv4[:port][/path]` enables it. Defaults: port `4318`, path `/v1/metrics`. |
| `CATTEN_OTLP_INTERVAL_MS` | Default `30000`; range `1000..=86400000`. |
| `CATTEN_OTLP_TIMEOUT_MS` | Default `3000`; range `100..=60000`, no greater than the interval. |

Intervals/timeouts without an endpoint reject. Empty/malformed endpoints,
userinfo, query strings, fragments, multicast and invalid ports reject. Paths
are bounded to 256 ASCII bytes. The initial transport is explicit cleartext
HTTP/IPv4; HTTPS, DNS resolution, authentication headers and gRPC are not
implemented. HTTPS rejects rather than downgrading or skipping authentication.
This public launch policy must not contain credentials. A local collector can
provide authenticated forwarding to further backends.

Selection requires rebuild/restart. The kernel supplies one complete, tagged
read-only profile (`COTLP001`, version 1) and logs its SHA-256. Cached-kernel
inspection checks the canonical profile hash, including the disabled case;
changing destination or budgets cannot silently reuse another policy. The
exporter requires the operational network stack and joins the normal shutdown
roster before time and TCP/IP dependencies.

## Wire data and ownership

The service uses [OTLP/HTTP protobuf](https://opentelemetry.io/docs/specs/otlp/):
HTTP/1.1 POST, `application/x-protobuf`, and an
`ExportMetricsServiceRequest`. No full OpenTelemetry SDK enters the kernel or
runtime. Resource attributes are `service.name=charlotte-os` and the node MAC
as `service.instance.id`; nodes require distinct MACs. The instrumentation
scope is `charlotte.telemetry`.

Each attempt exports the latest sample, not an upload of the entire history
ring. The interval sets remote observation cadence independently of the
one-second sampler. Gauges cover threads, domains, logical processors, free/usable/owned frames,
live/peak heap bytes, reserved/touched stack pages and thread high water. CPU
utilization divides the non-idle busy-counter delta by elapsed counter ticks
and processor count. Custom `charlotte.*` names do not claim semantic-convention
compatibility. Per-thread idle residency remains available for diagnostics;
node pressure excludes scheduler-created idle threads, including retirement.
This measures scheduler residency rather than cycle-accurate utilization;
interrupt work charged to an idle context is excluded too.

A synchronized time-service sample maps the history sample's monotonic instant
to UTC. Unknown/inconsistent clocks and history older than five seconds are
skipped. The exporter never invents a Unix timestamp from boot-relative ticks.
Gauges have no cumulative-series reset ambiguity across reboot.

One attempt owns request/response buffers, accepted creation, socket and pending
calls. Preparation and transport share one monotonic deadline. Pending calls
bind to the default CQ. Its sole consumer drains notification hints and observes
results through their original Rust owners; no raw hint handle is adopted.
The binary has one thread; `otlp_transport::post` explicitly requires that
sole-consumer contract. Requests and decoded response bodies are bounded to
8 KiB; the shared HTTP parser bounds headers to 16 KiB. Content type/length,
chunk framing and protobuf acknowledgements are checked.

Errors cancel/drop the complete operation and consume the original socket close
owner. The existing socket-close contract has its own finite five-second
budget. Unresolved creation or teardown aborts only the exporter domain, keeping
uncertain work retained rather than replaying it. These are ordinary EL0
operations, not kernel or IRQ callbacks.

## Delivery and failure isolation

Delivery is best-effort with one memory batch. A slow/unavailable collector
does not stop sampling, memory history or placement reporting. Failed/stale
batches are dropped; later attempts collect fresh data. Backoff doubles up to
the larger of the interval and five minutes, with up to one second of scheduling
jitter. Numeric `Retry-After` on HTTP 429/503 extends the pause, bounded to one
day. HTTP-date `Retry-After` is not decoded; normal backoff applies.

Empty default protobuf replies, including an initialized empty `partial_success`
message emitted by Collector 0.164, mean full success. Rejections or a nonempty
warning make the batch terminal partial success; it is never replayed. Logs
report accepted/dropped batches without sample bodies or remote error strings.
The service does not promise lossless or exactly-once delivery.

The [local Collector fixture](../../docker/otel-test/README.md) tests the actual
protocol. A separate fault receiver exercises partial success, throttling,
oversized/malformed replies and stalled peers.
The [initial qualification](../reports/investigations/2026-10-31-native-otlp.md)
records actual Collector decoding, collector-failure isolation and shutdown
during an accepted stalled export, including its coverage limits.
