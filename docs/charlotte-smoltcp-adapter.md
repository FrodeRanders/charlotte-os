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
