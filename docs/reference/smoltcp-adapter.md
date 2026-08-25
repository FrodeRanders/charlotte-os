# smoltcp → CharlotteOS Adapter

## The fit

smoltcp is the industry-standard embedded TCP/IP stack: `no_std` + `alloc`,
poll-driven, no threads, ~30 KiB of code. It already runs on bare-metal
microcontrollers through an Ethernet `Device` trait that looks almost
identical to what a CharlotteOS NIC driver exports:

| smoltcp `Device` | CharlotteOS NIC protocol |
|---|---|
| `receive() → Option<RxToken>` | frame queue fed by the frouter's `OP_FRAME` ingress |
| `transmit() → Option<TxToken>` | `OP_SEND` (call with a moved memory object holding the frame) |
| `capabilities()` | `OP_STATUS` (MAC + MTU + link state) |

The adapter is a thin shim over `net::OP_SEND` for transmit and a
receive-queue filled by the frame demultiplexer. The TCP/IP stack itself
needs no modification.

## Why the frouter feeds the receive path

The NIC driver owns a **single deferred `OP_RECV` slot**. Every Ethernet
consumer on a node (relmsg, cluster discovery, TCP/IP) would otherwise fight
over it. The **frouter** service holds that slot permanently and demultiplexes
each received frame by EtherType to the service registered for it:

- `0x88b5` (relmsg) → the `relmsg` service's `OP_FRAME`
- `0x88b6` (disco) → the `disco` service's `OP_FRAME`
- `0x0800` (IPv4) / `0x0806` (ARP) → the `tcpip` service's `OP_FRAME`

The tcpip service copies each forwarded frame into the adapter's receive
queue; `receive()` then hands those bytes to smoltcp. Transmit is
multi-consumer-safe on the driver (per-descriptor `tx_in_use`), so the
adapter uses `net::OP_SEND` directly.

## The adapter (`CharlotteEthDevice`)

```rust
struct CharlotteEthDevice {
    conn: u64,                     // connection cap to the NIC driver endpoint
    mtu:  usize,                   // 1500 for Ethernet
    rx:   VecDeque<Vec<u8>>,       // frames delivered through OP_FRAME
}

impl smoltcp::phy::Device for CharlotteEthDevice {
    type RxToken = CharlotteRx;
    type TxToken = CharlotteTx;

    fn receive(&mut self, _now) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let frame = self.rx.pop_front()?;
        Some((CharlotteRx { frame }, CharlotteTx { conn: self.conn }))
    }

    fn transmit(&mut self, _now) -> Option<Self::TxToken<'_>> {
        Some(CharlotteTx { conn: self.conn })
    }
}
```

The tokens do the actual I/O (smoltcp's consume contract):

- `CharlotteRx` hands smoltcp the queued frame bytes directly (no mapping —
  the bytes were copied when `OP_FRAME` arrived).
- `CharlotteTx` allocates a page, maps it at a scratch VA, lets smoltcp fill
  it, moves it to the driver with `OP_SEND`, and reaps the driver's reply so
  the pending-call slot is recycled.

## The TCP/IP service binary

`tcpip.elf` bootstraps, looks up `net0`, reads its MAC from `OP_STATUS`,
derives a stable link-local IPv4 address from it (`10.0.0.(100 + mac[5] % 100)`
— self-configuring, so both guests on a raw two-node link reach each other
directly on the same `/24`), registers as `"tcpip"` in the name service, waits
for the boot-done marker, then runs a poll loop that:

1. Pushes incoming `OP_FRAME` deliveries (IPv4/ARP from the frouter) into the
   adapter's receive queue;
2. Runs `iface.poll()` on a monotonic clock advanced by the poll interval;
3. Services the socket API (`OP_SOCKET`/`OP_CONNECT`/`OP_BIND`/`OP_LISTEN`/
   `OP_ACCEPT`/`OP_SEND`/`OP_RECV`/`OP_CLOSE`) over its endpoint;
4. Completes deferred `OP_RECV` replies when socket data arrives.

smoltcp 0.13 transitions a listening socket itself into the established
connection, so `OP_ACCEPT` reports success once the listener is no longer
listening. `OP_SEND` carries the payload length in the high 32 bits of `arg0`;
`OP_RECV` replies later by moving a page holding the received bytes.

## What's in place

- The NIC driver exports `OP_SEND`/`OP_RECV` through the `net0` endpoint
- The frouter demultiplexes frames by EtherType to `tcpip` via `OP_FRAME`
- Memory objects provide the buffer ownership model smoltcp needs
- The name service handles discovery (generation-safe lookup)
- `catten-syscall` has all the IPC and memory primitives
- `charlotte-smoltcp` implements the `phy::Device` adapter
- `tcpip.elf` serves the socket protocol over the `"tcpip"` name
- A self-configuring smoke client (`tcpclient.elf`) exercises a full
  two-node TCP echo through `--tcpip-test`

## Verification

Run two guests linked by a QEMU stream LAN (distinct MACs; the even-odd last
octet picks server/client):

```
scripts/run-aarch64.sh release --tcpip-test \
    --net-listen 5555 --instance a --mac 52:54:00:12:34:56 --smp 2 --timeout 120
scripts/run-aarch64.sh release --tcpip-test \
    --net-connect 127.0.0.1:5555 --instance b --mac 52:54:00:12:34:57 --smp 2 --timeout 120
```

Each guest's tcpip client resolves the peer MAC over ARP, completes the TCP
handshake, transfers a payload, and verifies the echoed bytes.

## HTTP keyhole / full report

`httpd.elf` turns the stack into a read-only "keyhole": a hardcoded HTTP
server on port 80 that answers every request with a JSON report of observable
state aggregated across the node:

- `meta` — wall-clock derived from the observe snapshot's monotonic counter:
  `uptime_ms`, `interval_ms` (time since the previous request), `counter_hz`
- `node` — NIC MAC + link (`net::OP_STATUS`)
- `ns` — the node-local name-service registry: registered-service catalog
  and pending lookups (`ns::OP_STATUS`, via the bootstrap connection)
- `tcpip` — ip, rx/tx frames, open sockets, send errors, DHCP mode, gateway,
  MTU (`socket::OP_STATUS`)
- `frouter` — rx/forwarded/dropped/unknown/routes (`frouter::OP_STATUS`)
- `dns` — Raft state/term plus the replicated `name -> node` cluster catalog
  (`dns::OP_STATUS` + `dns::OP_CATALOG`), plus the cluster posture
  (`raft::OP_CLUSTER_STATUS`: commit index, member count, leader id, self id),
  when running
- `disco` — probe-traffic counters (`disco::OP_DIAG`) and the live peer table
  (`disco::OP_LIST_PEERS`), when running
- `relmsg` — transport counters: peers, handled, retransmits, send failures,
  received, in-flight (`relmsg::OP_DIAG`), when running
- `threads` — system-wide thread statistics from the observe service's
  `OP_THREAD_SNAPSHOT`, backed by the kernel's unique SystemObserver
  capability (count, per-state histogram, sampled rows with per-thread
  `runtime_ms`/`cpu_pct`/affinity/pinning/`min`/`max` ticks)
- `http` — this server's own request counter, uptime, and request rate

Cumulative counters are paired with `*_delta` (count since the previous
request) and `*_rate` (per second) fields on `tcpip` and `frouter`, so the
report reflects activity between polls rather than lifetime totals. Rates are
integer per-second values computed from the observe snapshot's monotonic
counter and frequency — httpd never depends on a wall clock or floating point.

The `ns` and `dns` sections together are the node's picture of the cluster:
`ns` is the local registry (what is registered on *this* node), while `dns`
is the Raft-replicated catalog (which names live on *which* nodes, across
the cluster).

The aggregator uses non-blocking `ns::OP_TRY_LOOKUP`, so an absent service
renders as `null` rather than stalling a request. It consumes the same socket
API as the smoke client — one connection at a time, no keep-alive,
deliberately not a web server.

Two request targets are served, selected by the path of the `GET` request:

- `GET /` (or `/index.html`) returns a self-refreshing HTML dashboard; its
  embedded script polls `GET /metrics` every five seconds and renders the
  report as cards, which is handy on VMware where there is no framebuffer.
- `GET /metrics` (alias `/metric`) returns the JSON report described above.

Anything else is a `404`.

The NIC, DHCP-configured TCP/IP service, and `httpd` are launched by default.
The `--http-test` option below is a validation mode: it adds the QEMU host-port
forward and checks the response. It is not required for guest applications to
use HTTP or the socket service internally. Use `--no-network` only when an
isolated boot is intended.

From the host, run the guest on the SLIRP user network with a hostfwd and open
the dashboard in a browser (or curl the JSON endpoint directly):

```
scripts/run-aarch64.sh release --http-test --instance http --smp 2 --timeout 90
# browser: open http://127.0.0.1:8080/
curl -s http://127.0.0.1:8080/metrics
```

The guest self-test (`http_net_test`) verifies the httpd reaches its
listening stage; the run script then validates the JSON round trip from the
host. The guest uses `10.0.2.15` (SLIRP's default) with a default route via
`10.0.2.2` — the `ip`/`gateway` launch-manifest overrides the MAC-derived
address used on the raw two-node link.

### Capability model

The report has two sources with different privileges:

- **Per-service telemetry** must be *published* by each service over IPC
  (status ops). Status frames (`STATUS_VADDR`) stay per-domain; no capability
  lets an EL0 service read another domain's page. That is deliberate —
  services retain ownership of their telemetry.
- **Kernel scheduler data** comes from the `observe` service, which holds the
  unique `SystemObserver` capability granting system-wide thread statistics.
  The httpd queries it over IPC; it never holds the capability itself.
