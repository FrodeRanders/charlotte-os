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
frames through `net0` via the frouter, Raft peer RPCs (the distributed name
service) route through that service, and each VM runs a name-service replica.

Two-node tests use `--relmsg-test`, `--disco-test`, `--dns-test`, and
`--tcpip-test` with the stream backend (the relmsg smoke client derives its
peer from MAC last-octets 1 and 2). `--http-test` runs a single guest on the
SLIRP user network with `hostfwd=tcp::8080-:80` so the host can curl the
httpd keyhole.
