# UTC time service

CharlotteOS applications resolve the short name `time` through the local name
service. The returned capability implements `charlotte-protocol-time` v1:

- `OP_NOW` moves an 80-byte binary snapshot containing Unix seconds and
  nanoseconds, Gregorian UTC fields, monotonic-counter calibration, estimated
  uncertainty, source stratum, leap indicator, drift in parts per billion,
  and synchronization state.
- `OP_UNIX_SECONDS` returns whole Unix seconds as a scalar.
- `OP_ISO8601` moves a fixed 30-byte UTC value such as
  `2026-08-25T14:03:27.123456789Z`.

The state must be checked before using time for an accuracy-sensitive action:

- `STATE_UNSYNCHRONIZED` means no usable source exists; queries return
  `ERR_UNSYNCHRONIZED`.
- `STATE_HOLDOVER` means a persisted calibration is advancing from the current
  boot's monotonic clock. Since neither the object store nor the monotonic
  counter measures powered-off time, uncertainty is reported as `u32::MAX`
  milliseconds until a fresh network sample arrives.
- `STATE_SYNCHRONIZED` means at least one NTP reply was validated during this
  boot. Uncertainty starts with half the measured network round trip plus the
  server's root dispersion and grows with oscillator age.

## Default boot lifecycle

Time synchronization is an ordinary OS capability. The AArch64 and x86-64 QEMU
runners attach a NIC by default. Once the kernel discovers a supported controller,
steady-state launch starts the network driver, frame router, discovery and cluster
services, DHCP-configured TCP/IP, and the `time` service. The time client waits
until the local boot-ready marker is published and TCP/IP reports a nonzero DHCP
address, then sends its first NTP request. It continues sampling independently
of application queries.

Options such as `--net-test`, `--dhcp-test`, and `--disco-test` add verifiers
and pass/fail reporting only. `--no-network` intentionally omits the QEMU NIC;
because no network stack is then discovered, network-backed services including
`time` are not launched. On physical systems, the same launch decision follows
hardware discovery rather than a runner flag.

## Synchronization

The service sends NTPv4 client datagrams to UDP port 123. Replies must have a
supported version, server mode, a synchronized stratum, nonzero receive and
transmit timestamps, and an originate timestamp identical to the request
nonce. It pairs the midpoint of the server's receive/transmit timestamps with
the midpoint of the local monotonic send/receive interval. This follows the
four-timestamp model in [RFC 5905](https://www.rfc-editor.org/rfc/rfc5905.html).

Successful samples are taken every 15 minutes; failures retry after 64 seconds
and time out after five seconds without blocking the application endpoint.
Successive samples estimate monotonic oscillator drift, clamped to ±500 ppm
and damped to reduce network jitter. Returned time never decreases within one
service lifetime.

The steady-state launch manifest currently selects Cloudflare's documented
anycast NTP address `162.159.200.1`. `ntp_ip` can override it with four raw
IPv4 bytes. When local storage exists, the `persist` manifest flag enables the
reserved calibration object `0xfffd000000000001`.

The current SNTP exchange validates packet consistency but is not
cryptographically authenticated. Applications must not treat it as secure time
against an on-path attacker; NTS is a future protocol extension.
