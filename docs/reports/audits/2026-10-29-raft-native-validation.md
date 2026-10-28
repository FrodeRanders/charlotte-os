# Native Raft recovery and catalog qualification

Date: 2026-10-29. Source revision `9f3d5203e4604c3113afb07e25b28ef6cd0db48c`,
with local changes, including the retained-membership/quorum corrections described
in the [Raft contract](../../reference/raft-conformance.md#joint-entry-quorum-and-uncompacted-membership-recovery).
The [versioned evidence](evidence/2026-10-29-raft-native-validation.json) records
source, kernel and service hashes and the decisive serial excerpts. Other local
changes were present; this is qualification of the captured working tree.

The ordinary four-vCPU AArch64 boot passed **17/0/0** self-tests. Its NVMe fixture
observed term 1, restarted the Raft EL0 process, then observed term 2 and reported
that term/vote state survived. A second run was deliberately interrupted: after
`[raft-storage] first process reached term 1`, the controller verified that the
QEMU PID belonged to its newly created runner process group and sent that PID
SIGKILL. The interrupted runner returned 1, as expected. Rebooting with the same
instance disk and `--reuse-storage` returned 0, first reached term 2, and reached
term 3 after the ordinary process-restart fixture. Its final verdict was again
**17/0/0**. The interruption is retained separately from that passing recovery.

Two new socket-LAN guests, with distinct MACs and fresh disks, each passed
**20/0/0**. Both emitted the native DNS success verdict: the Raft-elected name
service replicated the catalog, served a remote invocation and automatically
fenced endpoint-death unregister by generation. The listener waited for the
connector's build/boot; its verdict arrived at 122.841366 guest seconds and the
connector's at 27.900256 seconds. Those are fixture completion times, not Raft
latency measurements.

The no-network kernel SHA-256 was
`88f58b7a98ba44d5be188524f1f29da571400ae55b90dae16a8bab2891d15476`;
the two network guests used
`ff8d3400a7687d64a0cf8aa76c68a4aa538501c884305c728c605cf9d72293a6`.
Expected EL0 fault-injection diagnostics are not the suite verdict; the controller
required `SELFTEST COMPLETE` with zero failed and pending tests.

## Reproduction

Build the current signed development bundle with
`scripts/build-catten-services.sh --embed`. Use new instance names to preserve
existing disks. The baseline invocation was:

```sh
./scripts/run-aarch64.sh --no-network --instance raft-validation-20261028-a --timeout 150 --smp 4
```

For abrupt recovery, launch the same no-network configuration with a new
`raft-validation-powercut-20261028-a` instance and set `CATTEN_QEMU_PID_FILE` to a
new PID file. Observe that instance's serial log for the first durable-term
verdict, verify the PID's process group against the owned runner, then kill only
that QEMU process with SIGKILL. Retain its serial capture before starting another
run. Repeat the invocation with `--reuse-storage`; require an advanced recovered
term and a successful complete self-test verdict. The evidence records terms,
exit codes and hashes for both captures. No existing instance was reset.

The network pair was started serially through the first QEMU launch, then run
concurrently:

```sh
./scripts/run-aarch64.sh --dns-test --instance raft-validation-dns-20261029-1 --mac 52:54:00:12:34:01 --timeout 180 --smp 4 --net-listen 12911
./scripts/run-aarch64.sh --dns-test --instance raft-validation-dns-20261029-2 --mac 52:54:00:12:34:02 --timeout 180 --smp 4 --net-connect 127.0.0.1:12911
```

## Limits

QEMU process loss leaves the host filesystem and its caches running. This does
not qualify physical power loss, device flushes, interruption during log/snapshot
publication or simultaneous multi-voter restart. The interrupted fixture proves
recovery of an acknowledged term; the ordinary fixture checks durable term
advancement across process restart. Its success message labels it a term/vote
test, but this campaign does not independently check same-term vote retention
across a VM crash. It does not prove arbitrary application data survives every
crash point.

These guests use Charlotte capability IPC/relmsg and its native catalog. They do
not form a mixed cluster with Graft's Java/C++/Rust TCP runtimes. No new TLA+
execution, throughput result or management TLS availability result is claimed.
Existing management failover limitations remain in their own reports.
