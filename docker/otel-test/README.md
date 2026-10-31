# Local OpenTelemetry collector

The digest-pinned official Collector accepts OTLP/HTTP on localhost port 14318
and decodes metrics through its debug exporter. It installs no storage backend;
container logs are bounded.

```sh
docker compose -f docker/otel-test/compose.yaml up -d

# QEMU's user-network address for a service on the host is 10.0.2.2.
CATTEN_OTLP_ENDPOINT=http://10.0.2.2:14318/v1/metrics \
CATTEN_OTLP_INTERVAL_MS=10000 CATTEN_OTLP_TIMEOUT_MS=3000 \
  scripts/run-aarch64.sh release --minimal

docker compose -f docker/otel-test/compose.yaml logs -f collector
```

Ordinary nodes use their collector's actual IPv4 address. The same guest
configuration works on x86. Local telemetry archiving can stay disabled;
export waits for a synchronized time sample. Stop the fixture with
`docker compose -f docker/otel-test/compose.yaml down`.

`scripts/tests/otlp-collector-fixture.py` is a separate local fault receiver,
not an interoperability substitute for the official Collector. Its mode file
selects `success`, `partial`, `retry`, `stall`, `oversized`, or `malformed`.
It records only request time, length and SHA-256:

```sh
printf '%s\n' partial > /tmp/charlotte-otlp-mode
python3 scripts/tests/otlp-collector-fixture.py --port 14318 \
  --mode-file /tmp/charlotte-otlp-mode --log /tmp/charlotte-otlp-requests.jsonl
```

Stop the official fixture before binding the same port. Throttling responds
with `Retry-After: 5`; stalled replies outlast a one-second guest timeout.
