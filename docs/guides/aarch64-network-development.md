# AArch64 network development

KVM is not a CharlotteOS networking requirement. It is an acceleration
backend. On macOS, QEMU's software translator (TCG) can run the complete
AArch64 network stack by default. Add `--net-test` only to verify the PCI,
MSI-X, SMMUv3, and EL0 virtio-net path explicitly:

```sh
./scripts/run-aarch64.sh debug --net-test --timeout 60
```

HVF remains unsuitable for networking because direct device-MMIO
access from the EL0 driver is not handled reliably. Omit `--hvf`; the runner
then selects TCG on macOS. Use `--hvf --no-network` for an intentionally
networkless compatibility boot. Linux may use TCG as well, although KVM is
faster.

The smoke test uses an explicitly placed transitional virtio-net PCI device.
The kernel discovers it from the published PCI topology, programs MSI-X,
delegates its translated legacy register page and interrupt, and creates a
private SMMU domain for its requester ID. The userspace driver negotiates the
MAC and link-status features, builds DMA-mapped legacy vrings, reaches
`DRIVER_OK`, and accepts a frame from an EL0 client.

## Two QEMU nodes on one Mac

QEMU's socket network backend forms a private Ethernet segment without TAP,
root privileges, vmnet, or KVM. Start the listener first:

```sh
./scripts/run-aarch64.sh debug \
  --instance node-a --mac 52:54:00:12:34:01 --net-listen 12000
```

Then start the second node in another terminal:

```sh
./scripts/run-aarch64.sh debug \
  --instance node-b --mac 52:54:00:12:34:02 \
  --net-connect 127.0.0.1:12000
```

`--instance` gives each VM independent boot media, persistent NVMe storage,
and serial logs. With a timeout, the logs are
`/tmp/charlotte-node-a-serial.log` and
`/tmp/charlotte-node-b-serial.log`. These ordinary boots run discovery and
cluster formation; add `--disco-test` only when pass/fail verification is
desired.

For simultaneous scheduler/timer diagnostics, give each runner a distinct GDB
stub port. Trace and snapshot files include the instance name, so the two
captures do not overwrite one another:

```sh
./scripts/run-aarch64.sh debug --dns-test --scheduler-trace --debug-snapshot \
  --gdb-port 1234 --instance node-a --mac 52:54:00:12:34:01 \
  --net-listen 12000 --timeout 60
./scripts/run-aarch64.sh debug --dns-test --scheduler-trace --debug-snapshot \
  --gdb-port 1235 --instance node-b --mac 52:54:00:12:34:02 \
  --net-connect 127.0.0.1:12000 --timeout 60
```

This establishes the host-side L2 link and runs a NIC in each guest. It is
the distributed-services test path: the reliable-message service exchanges
frames through `net0` via the frouter, each VM runs a name-service replica,
and the DNS-owned Raft member uses a dedicated Ethernet route for consensus
RPCs. Admission and distributed application/control messages continue through
the reliable-message service.

Two-node tests use `--relmsg-test`, `--disco-test`, `--dns-test`, and
`--tcpip-test` with the stream backend (the relmsg smoke client derives its
peer from MAC last-octets 1 and 2 and exchanges a 70,000-byte v3 message).
`--http-test` runs a single guest on the
SLIRP user network with `hostfwd=tcp::8080-:80` so the host can curl the
httpd keyhole.

To configure the operational distributed-ingress path (without adding a test
workload), supply a cluster service to every participating guest:

```sh
./scripts/run-aarch64.sh release --cluster-service 10.0.2.42:80
```

On the default single-guest SLIRP network the runner forwards host port 8080
to that VIP and port, which provides a basic HTTP demonstrator. For a real
multi-node load-sharing exercise, place all guests and the client on a shared
tap/bridge or socket-backed L2 fixture and use the same `VIP:port` on every
node.

The repository includes the complete socket-backed validation:

```sh
./scripts/run-distributed-ingress-test.sh
```

It starts an Ethernet hub, three AArch64 guests, and an independent host-side
ARP/TCP participant. After stable three-voter membership it establishes one
flow per selected backend, stops the Raft leader/VIP advertiser, waits for a
replacement advertisement, and sends HTTP on the already established flows.
The fixture requires remote forwarding, at least one surviving flow, and a
fresh HTTP connection to a live backend after the failure. It uses
`--cluster-ingress-test` internally; that switch is verifier plumbing, whereas
`--cluster-service` is the operational configuration.
