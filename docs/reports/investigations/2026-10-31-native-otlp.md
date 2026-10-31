# Native OTLP export qualification — 2026-10-31

Charlotte can export current metrics directly from memory without a local
archive. The exporter is an optional userspace service with one immutable
HTTP/IPv4 destination, bounded memory, one attempt deadline and no disk spool.
Sampling and cluster placement reporting continue through collector failure.
The current contract and configuration live in [OTLP export](../../reference/otlp-export.md).

## Findings

- OTLP/HTTP protobuf interoperates with the official Collector without bringing
  an OpenTelemetry SDK into the kernel. Collector 0.164.0 returns an initialized
  empty `partial_success` message (`0a00`) for full success. Default fields mean
  success; rejected points or a nonempty warning make the batch terminal partial
  success. Neither ambiguous failure nor partial success permits replay.
- The original node busy counter included dedicated idle-thread `WFI`/`HLT`
  residency. An idle Collector sample consequently reported `0.999993` CPU
  utilization. Classifying scheduler-created idle contexts before publication
  and excluding both live and retired contributions corrected that counter.
  This also fixes placement input, which uses the same counter. Per-thread
  residency diagnostics are preserved. Corrected samples included `0.048021`
  and, on the final image, `0.019625`. These are scheduler-residency estimates,
  not cycle-accurate hardware or QEMU process CPU measurements.
- Export needs synchronized UTC, not boot-relative timestamps. Unsynchronized
  startup samples are skipped. Collector failure drops the batch and backs off;
  the next attempt reads fresh history rather than retaining a stale queue.
- The accepted call, creation, socket and buffers need one enclosing owner.
  Checked cancellation precedes consuming remote close on every outcome.
  CQ records are hints; their raw attachment IDs must never be adopted as owners.
  Shutdown must wake that same CQ before transport dependencies retire.

## Qualification

The [version-controlled evidence](evidence/2026-10-31-native-otlp.json) retains
kernel/profile/exporter identities, decoded metrics, fault request digests,
live reporting observations and shutdown diagnostics. Base commit: `c3443372`.
Runs used QEMU, public development trust and isolated fresh storage.

| Control | Result |
| --- | --- |
| Full ARM suite, export disabled | 20 passed, 0 failed/pending; bitmap `0x2023ffff` |
| Full Intel suite, export enabled | 18 passed, 0 failed/pending; bitmap `0x2023bbff` |
| Final ARM minimal image, archive disabled | Official Collector decoded 12 gauges with UTC sample timestamps and MAC resource identity |
| Terminal partial success | Next fresh request after 2.102 s; different payload digest |
| HTTP 503, `Retry-After: 5` | Next fresh request after 5.735 s |
| Stalled response beyond 1 s attempt budget | Deadline failure, next fresh request after 5.720 s |
| Oversized / malformed response | Rejected; fresh requests after 4.142 / 4.131 s |
| Live history and placement during faults | History samples 16→30, last ticks advanced; accepted capacity reports 2→5; same boot nonce, fresh capacity |
| ARM signed shutdown during accepted stalled export | Admission 0.459 s after request; exporter observed `Shutdown`, acknowledged=1, unacknowledged=0, forced=0; PSCI poweroff followed |
| Intel isolated shutdown, exporter enabled | Exporter acknowledged without force; isolated verifier 1/1 |
| Host suites | Services 267, launch 14, HTTP/signing 9, boot-profile 8, tooling 11; all passed |
| Static checks | Workspace formatting; ARM and Intel exporter Clippy with warnings denied |

The full native suites preceded the final explicit consuming `Export::finish`
refinement. The final fault campaign, real Collector repeat, live-report checks
and both shutdown controls cover that refinement. The earlier successful fault
campaign is retained separately in the evidence; it is not substituted for the
final run. The first host repeat failed three OpenSSL controls because the
sandbox denied localhost socket binding; the permitted repeat passed all 267
service tests. This was a host harness restriction, not a guest result.

The Collector fixture pins
`otel/opentelemetry-collector:0.164.0@sha256:28b6d41ac51bff6b41ba5e8fe503569ffb9d6a1ea90f7a78419e86f34e44f750`.
Use the [fixture instructions](../../../docker/otel-test/README.md) for real
protocol and fault receivers. The final ARM profile selected
`http://10.0.2.2:14318/v1/metrics`, interval 2000 ms, timeout 1000 ms and archive
interval 0. Fault modes were selected one at a time; each was followed by a
successful fresh sample before the next fault. All recorded payload digests
were distinct. Test guests, receivers and Collector were stopped afterward.

## Limits

Initial export supports metrics over cleartext HTTP/IPv4 only. HTTPS, DNS
resolution, authentication headers, gRPC, traces and logs remain unimplemented.
There is no lossless-delivery, durable-spool or exactly-once claim. Exporter
allocation/mapping admission failures and unconfirmed remote-close abort were
reviewed but not fault-injected in these guest controls. Logical socket close
and service acknowledgement do not independently qualify physical cleanup or
the broader C18/R18 caller inventory. This work closes no unrelated audit gate.
