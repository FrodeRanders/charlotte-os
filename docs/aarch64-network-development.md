# AArch64 network development

KVM is not a CharlotteOS networking requirement. It is an acceleration
backend. On macOS, QEMU's software translator (TCG) can run the complete
AArch64 PCI, MSI-X, SMMUv3, and EL0 virtio-net smoke path:

```sh
./scripts/run-aarch64.sh debug --net-test --timeout 60
```

HVF remains unsuitable for this particular test because direct device-MMIO
access from the EL0 driver is not handled reliably. Omit `--hvf`; the runner
then selects TCG on macOS. Linux may use TCG as well, although KVM is faster.

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
./scripts/run-aarch64.sh debug --net-test \
  --instance node-a --mac 52:54:00:12:34:01 --net-listen 12000
```

Then start the second node in another terminal:

```sh
./scripts/run-aarch64.sh debug --net-test \
  --instance node-b --mac 52:54:00:12:34:02 \
  --net-connect 127.0.0.1:12000
```

`--instance` gives each VM independent boot media, persistent NVMe storage,
and serial logs. With a timeout, the logs are
`/tmp/charlotte-node-a-serial.log` and
`/tmp/charlotte-node-b-serial.log`.

This establishes the host-side L2 link and runs a NIC in each guest. It does
not yet constitute a distributed-name-service test. The remaining guest-side
work is to make the reliable-message service exchange frames through `net0`,
route Raft peer RPCs through that service instead of the local IPC transport,
and then run one name-service replica per VM. The current `relmsg` binary is a
local prototype and is not yet an Ethernet peer transport.

