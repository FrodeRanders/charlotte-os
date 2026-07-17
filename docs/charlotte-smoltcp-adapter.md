# smoltcp → CharlotteOS Adapter

## The fit

smoltcp is the industry-standard embedded TCP/IP stack: `no_std` + `alloc`,
poll-driven, no threads, ~30 KiB of code. It already runs on bare-metal
microcontrollers through an Ethernet `Device` trait that looks almost
identical to what a CharlotteOS NIC driver exports:

| smoltcp `Device` | CharlotteOS NIC protocol |
|---|---|
| `receive() → Option<RxToken>` | `OP_RECV` (deferred receive — reply carries a frame) |
| `transmit() → Option<TxToken>` | `OP_SEND` (call with a moved memory object holding the frame) |
| `capabilities()` | `OP_STATUS` (MAC + MTU + link state) |

The adapter is a thin shim over the existing `net::OP_SEND`/`net::OP_RECV`
protocol. The TCP/IP stack itself needs no modification.

## The adapter (`CharlotteEthDevice`)

```rust
struct CharlotteEthDevice {
    conn:    u64,           // connection cap to the NIC driver endpoint
    mac:     [u8; 6],      // MAC address (cached from OP_STATUS)
    mtu:     usize,        // 1500 for Ethernet
    rx_buf:  Option<u64>,  // outstanding OP_RECV reply token (None = none)
}

impl smoltcp::phy::Device for CharlotteEthDevice {
    type RxToken = CharlotteRx;
    type TxToken = CharlotteTx;

    fn receive(&mut self) -> Option<(Self::RxToken, Self::TxToken)> {
        // If we have an outstanding OP_RECV, check whether the driver has
        // replied yet (non-blocking poll of the pending-call cap).
        if let Some(call) = self.rx_buf {
            if let Ok(Some(reply)) = ipc::poll_reply(AS, call) {
                let frame_cap = reply.memory?;
                self.rx_buf = None;
                // Issue the next receive immediately (pipelining).
                self.rx_buf = Some(ipc_scalar_call(self.conn, OP_RECV, 0)?);
                return Some((CharlotteRx { frame_cap }, CharlotteTx { conn: self.conn }));
            }
        } else {
            // No outstanding receive — issue one.
            self.rx_buf = Some(ipc_scalar_call(self.conn, OP_RECV, 0)?);
        }
        None
    }

    fn transmit(&mut self) -> Option<Self::TxToken> {
        // smoltcp allocates the frame buffer and hands it to us.
        // We'll copy it into a memory object and hand it to the driver.
        Some(CharlotteTx { conn: self.conn })
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut caps = smoltcp::phy::DeviceCapabilities::default();
        caps.max_transmission_unit = self.mtu;
        caps.max_burst_size = Some(1);
        caps.checksum = smoltcp::phy::ChecksumCapabilities::ignored(); // NIC handles it
        caps
    }
}
```

The tokens do the actual I/O when dropped (smoltcp's contract):

```rust
struct CharlotteRx { frame_cap: u64 }
impl smoltcp::phy::RxToken for CharlotteRx {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, f: F) -> R {
        // Map the received frame memory object at a scratch VA, call the
        // consumer closure, then close the cap (returning the page to the
        // kernel).
        let scratch: usize = 0x00a0_0000;
        memory_map(self.frame_cap, scratch, false);
        let len = /* read frame length from virtio-net header if present, else 2048 */;
        let result = f(unsafe { core::slice::from_raw_parts(scratch as *const u8, len) });
        memory_unmap(self.frame_cap);
        memory_close(self.frame_cap);
        result
    }
}

struct CharlotteTx { conn: u64 }
impl smoltcp::phy::TxToken for CharlotteTx {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        // Allocate a page, map it, let smoltcp fill it, then send.
        let cap = memory_alloc(1);
        let scratch: usize = 0x00a1_0000;
        memory_map(cap, scratch, true);
        let buf = unsafe { core::slice::from_raw_parts_mut(scratch as *mut u8, len) };
        let result = f(buf);
        memory_unmap(cap);
        ipc_scalar_call_move(self.conn, OP_SEND, len as u64, cap);
        result
    }
}
```

## The TCP/IP service binary

The service program itself is simple — it bootstraps, looks up `net0`, runs the
poll loop:

```rust
fn cmain(_args: Args, _input: Input<0>) -> ! {
    let ns = config::bootstrap_cap().unwrap();
    let (_, net_conn) = ns.lookup("net0").unwrap(); // generation 1+

    // Read MAC + MTU from the driver.
    let (_, mac) = net::decode_status(call_reply(net_conn, net::OP_STATUS, 0));

    let mut device = CharlotteEthDevice::new(net_conn, mac, 1500);
    let mut iface = smoltcp::iface::Interface::new(
        smoltcp::iface::Config::new(smoltcp::wire::HardwareAddress::Ethernet(mac)),
        &mut device,
    );
    // Configure IP, add a default route, etc.

    // smoltcp's poll loop — call as fast as possible.
    loop {
        iface.poll(Instant::now(), &mut device, &mut sockets);
        // Yield briefly to let the NIC driver process frames.
        thread_yield();
    }
}
```

## What's already in place

- The NIC driver exports `OP_SEND`/`OP_RECV` through the `net0` endpoint
- Memory objects provide the buffer ownership model smoltcp needs
- The name service handles discovery (generation-safe lookup)
- `catten-syscall` has all the IPC and memory primitives

## What's needed for a complete slice

1. **One new crate** — `charlotte-smoltcp` (~300 lines, the adapter + service loop)
2. **smoltcp as a dep** — `smoltcp = { version = "0.12", default-features = false, features = ["alloc", "log", "proto-ipv4", "proto-ipv6", "socket-raw", "socket-udp", "socket-tcp"] }` — compiles for `aarch64-unknown-none`, `no_std` + `alloc`
3. **A new EL0 binary** — `tcpip.elf`, registered as `"tcpip"` in the name service
4. **A socket protocol** — clients connect to the TCP/IP service and open sockets through a separate endpoint (the "legacy socket" interface that the architecture doc's compatibility section describes)

The adapter itself is straightforward. The harder part is designing the socket
API exposed to other CharlotteOS services — but that's a protocol-design
question, not a feasibility one.

## Timing / interaction model

smoltcp is **entirely poll-driven**: the application calls `iface.poll()` which
calls `device.receive()` and `device.transmit()`. The adapter translates these
into the CharlotteOS `OP_RECV`/`OP_SEND` calls. No threads, no interrupts, no
callback registration needed. The TCP/IP service just sits in a tight poll loop
with cooperative yielding — exactly the pattern the echo service already uses.

## Verdict

**smoltcp is a drop-in candidate.** The adapter is ~100 lines of code. The
harder work is the socket-API design for other services to consume TCP/IP, but
smoltcp itself requires zero modification to run on CharlotteOS.
