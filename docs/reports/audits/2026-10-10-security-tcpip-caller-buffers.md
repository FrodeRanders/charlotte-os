# TCP/IP caller backing and negative audit probes

This follows commit `b945a59` and the
[object-store follow-up](2026-10-10-security-object-store-follow-up.md).
It addresses SEC-32's receive-retention path and adds direct negative evidence
for SEC-31, SEC-37 and SEC-38. Historical audit reports remain unchanged.

## Receive and status backing

`OP_RECV` and `OP_STATUS` now require one writable 4096-byte memory object
moved into the request. TCP/IP fills and returns that supplied object, rather
than allocating a reply page. All in-tree callers were migrated, including
asynchronous TCP clients, time, Kafka, HTTP diagnostics and kernel DHCP tests.
Missing or oversized buffers reject; there is no service-allocating compatibility
path. Status uses the same ownership machinery so it cannot bypass the fix.
UDP checks the configured peer and payload length before copying into the
supplied page, so a discarded foreign datagram cannot dirty its later reply.

The kernel already charges a move to the original allocating generation and
charges a copy to the copying caller. This correction changes the protocol's
allocation sponsor, not the kernel accounting rules. It preserves the exact
original charge through return, retention, sponsor close and physical release.
A client may exhaust its own admission, but retained receive/status pages no
longer consume TCP/IP's transmit allocation budget.

`PendingReceive` contains the complete received request, buffer, reply/loan
authority, and any transient mapping. Mapping rejection restores its owner;
unmapping failure retains its mapping until owned cleanup. Local access ends
before reply or loan cancellation. Explicit cancellation, socket close,
shutdown and owner reclamation all dispose this same operation owner. The
runtime's new unsafe `IncomingMessage::from_raw` is a documented raw receive
adapter boundary, with exactly-once adoption; no owning capability escapes
into a scalar field in the new operation. Existing kernel returned-memory
`PreparedTransfer` and cancellation receipts remain authoritative.

## Negative probes

- After its flood drains, the security fixture uses the live signed ordinary
  application with a ceiling of 32 additional pages/objects above its baseline.
  It checks missing and oversized buffers for receive and
  status, read-only mapping rejection with safe loan revocation, and explicit
  pending-receive cancellation. It retains 32 status replies, checks exact
  caller charges after each return, rejects the next allocation, and verifies
  an independent sponsor's status still progresses within its deadline.
  It then moves one page out, closes its original sponsor, verifies that the
  charge persists, creates another root, and checks physical release refunds
  the original account without touching that root.
- Both cross-node TCP clients retain 32 actual receive objects while 32
  bidirectional exchanges complete. Received contents are checked every time;
  buffers are explicitly released after the socket closes.
- The scoped security fixture publishes the short name `secavail` using its
  authenticated descriptor. While that application remains alive, a separate
  ordinary namespace tries all five legacy lookup opcodes: scalar, keyed,
  immediate, named and immediate named. Each must deny without returning
  memory or connection authority, at both publication generations. Existing
  authorized acquire/call controls remain in the application probe.
- The storage fixture dispatches `SPAWN_UPGRADE` using the exact installed
  manager identity, a valid target connection, and actually allocated backing
  for zero-size and above-4-MiB requests. It checks rejection, input-capability
  consumption and
  manager liveness, then executes the ordinary successful persistent upgrade.
  The oversized bytes are not a signed executable; rejection-before-snapshot
  ordering is additionally source-reviewed, not inferred solely from rejection.
- An opt-in trusted relmsg fixture sends `MAX_MSG + 1` and `u32::MAX` SYN
  fragments through the production frame handler. These must admit no new peer
  and preserve an existing session and partial assembly. Inspection exposed
  session replacement occurring before the earlier inner size gate. The gate
  now precedes peer/session mutation. The fixture uses a shared compile-time
  manifest key and is enabled only by the security launch profile.

## Failed executions preserved

The first AArch64 execution panicked at
`crates/charlotte-launch/src/lib.rs:619:5` with
`manifest keys are limited to eight bytes`. The newly introduced `auditprobe`
key was ten bytes. The guest was stopped after its panic; it has no passing
coordinator result. Kernel SHA-256:
`5beb2fe4da3eb3491e55c23c7fe4d77123eed938e66753a8b80f9758382f0499`;
signed relmsg SHA-256:
`bea90c85932a617a4252e82c34ae966432f6f3e6d2e0c71c6ce0f7d7925acfa8`.
The replacement key is the shared constant `relmsg_status::AUDIT_PROBE_KEY`,
which checks its packed length during compilation. The launch switch also
uses the actual `feature = "security_test"` predicate.

The next AArch64 execution completed **18 passed, one failed, zero pending**
(`passed_bitmap=0x3ffff`, `failed_bitmap=0x20000000`). The new naming fixture
waited five seconds for a nonexistent public `grantctl` registration and failed
at `el0_security.rs:170`. Kernel SHA-256:
`ff07f34c66594700f6ced30df90a77e6668eac3c657a6b987737da3ee1bfcba3`.
The controller is supplied through bootstrap authority, not public registration.
The corrected fixture targets the application's actual `secavail` publication
and runs after its publication/authorized controls pass. No deadline was raised.
Relmsg and oversized-upgrade diagnostics passed in that failed execution;
they do not replace a complete passing run.

A security-plus-HTTP execution reported **20 passed, zero failed/pending**
(`passed_bitmap=0x2023ffff`), but the host keyhole validation failed. Three
12-second `/metrics` attempts timed out at `client.recv`; the runner ended
with `guest HTTP keyholes did not return both expected JSON views`. Later
network counters progressed, but this does not establish the cause or turn
the runner into a pass. Kernel SHA-256:
`39dc6bdfa3a9c3df165ec776affddadad78c0eabb4e163caaa0982bec1541c73`.

A subsequent combined DHCP/security/HTTP execution reported **15 passed,
one failed, five pending** (`passed_bitmap=0x27ffd`,
`failed_bitmap=0x1000000`, `pending_bitmap=0x20218002`). The first socket
submission in the new accounting probe failed with `QueueFull` while the
security socket flood was active. Kernel SHA-256:
`48cd05d327a73bd2860a091fb55b43fd09e46af6439310a364f5c1bf300a26b7`.
That probe also incorrectly used a bare namespace without live domain/thread
authority. It now borrows the actual signed application's live root after the
flood and positive controls finish. Neither fixture rejection establishes a
memory-accounting defect or broad fairness conclusion.

The first two-node TCP execution failed on both guests with **18 passed,
one failed, zero pending** (`passed_bitmap=0x3ffff`,
`failed_bitmap=0x100000`). Each client stopped at stage 5/error `0xe005`;
TCP/IP rejected its first socket and the original 90-second exchange deadline
expired. Kernel SHA-256:
`6c84cc32a82b899d10605d6f1c63cedbdae73b7882488b050824b4cb1c0c8646`.
Source inspection identified the fixture's generic launcher as lacking the
trusted TCP/IP socket-owner inspection registration. It now uses
`supervisor::spawn_tcpip_with_manifest`; application-selected names cannot
grant this authority. A repeat with the trusted launcher still failed at the
unchanged exchange deadline: it exposed a second TCP/IP instance colliding
with the already launched appliance's name/router registrations and replacing
its inspection identity. Both guests again reported 18 passed/one failed
(`failed_bitmap=0x100000`); kernel SHA-256:
`d2a8aca4c939ea88c8fcc4cdc07998ebc281b51fac262143a5a22a0f5dc35825`.
The verifier now observes the single steady-state appliance and only launches
the client, as the DHCP/HTTP verifiers already do. No production authority gate
or deadline was weakened.

The single-appliance TCP repeat also failed before buffer exchange. The
connecting guest stopped at stage 6/error `0xe016` with zero IP transmit/receive
counters; its exchange deadline expired (18 passed/one failed,
`failed_bitmap=0x100000`). Kernel SHA-256:
`c46131ff2a3e5871f869c1e5b0a36dd74197f61e956ad0971b11b50655473e9c`.
The runner previously selected static appliance addressing only when a cluster
VIP was specified. Its raw two-guest Ethernet link supplies no DHCP server.
It now selects static addressing for every non-user network backend, and clears
that setting for SLIRP. The listening peer also expired its deadline: 18 passed,
one failed, zero pending,
with the same failure bitmap and kernel hash. Both failed captures remain
separate from the final paired execution.

Before guest entry, one paired attempt rejected the inherited sandbox-network
marker; it was rerun outside the sandbox with its explicit network override.
One x86 launch could not create host forwarding on the default deployment port;
the dedicated-port repeat below passed. Neither launch entered its test guest
and neither is counted as a kernel execution.

## Validation

The host suite passed **353 tests, zero failed, three ignored**, plus the signing,
boot-result, secure-boot-result and syscall-emitter script checks. Both signed
service bundles built; strict AArch64 service Clippy and AArch64 TCP/x86 kernel
Clippy passed. Rust formatting and whitespace checks passed.

| Execution | Result | Kernel SHA-256 |
| --- | --- | --- |
| AArch64 security, corrected live caller fixture | 19 passed, zero failed/pending; bitmap `0x2003ffff`. Both generations passed caller-accounting, cancellation, mapping rejection, legacy denial, relmsg and oversized-upgrade probes. This checkpoint precedes the UDP-before-copy and zero-size additions. | `d5d6f3ecd8892bbaecc7e3e7293be6a905c44e1592431a48bb0a0c2434e4a1e8` |
| AArch64 isolated HTTP/DHCP | 20 passed, zero failed/pending; bitmap `0x123ffff`. Host initial metrics, EOF recovery, abortive-peer recovery, idle-client timeout and both JSON keyholes passed. Transport resets were retried by the existing runner. This checkpoint precedes only the zero-size upgrade probe. | `059ef4606ebb51db2ede39d16e7861282d36ed26e95c4418f9fba234823a46a5` |
| x86 AMD and Intel, final size probes | 15 passed, zero failed/pending; bitmap `0x1bbff`. Zero-size/oversized rejection and subsequent persistent upgrade passed. | `efa1d8469ce3a15e02790364d5bf3b635f58631523f73509a367493e3c910b5d` |
| AArch64 TCP pair, final source | Both guests: 19 passed, zero failed/pending; bitmap `0x13ffff`; both runner exits zero. Each retained 32 caller-sponsored receive pages through 32 bidirectional exchanges. | `663a93ae125c23de87201b882bdb003f296a2e292986eb603fb2518547c069f2` |
| AArch64 final combined security/HTTP/DHCP | Kernel: 21 passed, zero failed/pending; bitmap `0x2123ffff`. All new probes passed, including both caller generations and zero-size input. Runner: **failed host HTTP validation**, as detailed below. | `562e273095cbe92d3497e03f427344d1a5dd723a785212094866c07014338770` |

The x86 runs do not execute the AArch64-only security/network fixtures and are
not evidence for those assertions. Strict Clippy also passed the final combined
AArch64 configuration and final x86 source. The final combined guest's host
test repeated the three 12-second initial-metrics timeouts and ended with
`guest HTTP keyholes did not return both expected JSON views` and
`guest HTTP keyhole was not validated from the host`. Both security generations
had completed by guest time 23.53 seconds; later HTTP/network activity resumed,
with `tx_ok=235`, `tx_err=200` at 100.33 seconds. Those send counters measure
client API calls, not emitted frames. Recovery afterward does not satisfy the
chosen host deadlines. A dashboard probe attempted after runner exit met a
closed host port and adds no evidence about the live stall. The combined
failure's cause remains unresolved; the isolated passing run does not erase it.

Reproduction uses `CATTEN_SKIP_EMBED_BUILD=1` after building both signed
bundles. Execute `scripts/run-aarch64.sh --security-test --http-test --dhcp-test
--instance NAME --fresh-storage --timeout 180` with dedicated HTTP/deployment
host ports for the combined profile, or omit `--security-test` for isolation.
The TCP pair uses `--tcpip-test --net-listen PORT` and
`--tcpip-test --net-connect 127.0.0.1:PORT`, distinct instances, MAC suffixes
`02` (listener) and `01` (connector), fresh storage and the same 180-second
runner window. Start the peer immediately after the first kernel build boots;
the peer may reuse that exact kernel with `CATTEN_SKIP_KERNEL_BUILD=1`.
Neither peer uses SLIRP/DHCP. Kernel verifier deadlines remain unchanged.

Signed AArch64 service SHA-256 values used for the final network/security runs:

| Service | SHA-256 |
| --- | --- |
| TCP/IP | `f5bb83a6952e3363aae796c9e130cd46d7787d3afa61da1f73f235e0825d60fd` |
| TCP client | `b8db78541df16ab98abd0de367219069ccd09f1b02f75e8acc9d06f5b90878bb` |
| Relmsg | `115624110587eb889a52d801f10527c56f1231710b58a9e8eb72668ad4b274f5` |
| Scoped security probe | `6a44e036052757fd3524de6b5a5bd630f3985f4b6ead289027b31da0492d94e1` |

## Disposition and limits

SEC-32 is corrected in the receive/status sponsorship scope, with the recorded
accounting, cancellation, mapping-rejection and paired TCP checks passing. SEC-07 still requires general kernel heap/principal aggregate
admission and the broader pressure/progress matrix. This protocol correction
does not redistribute unrelated service allocations or qualify all other
services' replies. Socket smoltcp ring/heap quotas are unchanged. Real hostile
packet flooding, sustained node-wide saturation, hardware faults and full
cross-LP recovery remain separate work. The frame probe uses the actual handler
with bounded private state inside the guest; it is not a hostile external NIC
campaign. Historical abort/stack failures retain their original limitations.
The repeated combined-profile HTTP timeout remains explicit SEC-06/R7-4/R7-5
progress qualification work; neither the passing isolated host check nor the
new protocol ownership proof establishes progress under all concurrent pressure.
