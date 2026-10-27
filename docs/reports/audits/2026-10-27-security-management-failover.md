# Management leader-loss qualification and proportional election jitter

Date: 2026-10-27. Parent: `c5a4e987`. Follow-up to
[native certificate validity](2026-10-27-security-management-validity.md).
SEC-10 remains partial; the ledger remains **29 corrected in scope, seven
partial and two open**. Production remains disabled.

The campaign exposed and corrects an election-liveness defect: a fixed 150 ms
jitter window was independent of the native 2000 ms election timeout. Surviving
voters repeatedly voted for themselves before receiving each other's requests.
The corrected native cluster replaces its failed leader and accepts another
signed policy through a follower. **Both the full campaign and a direct-node
repeat still fail** with post-failover TLS handshake timeouts. The successful
Raft/admission phases do not qualify management availability or the complete
client-rejection matrix.

## Election correction and configuration

Initial deadlines and subsequent resets now derive jitter in `[0, timeout)`
from `RaftNodeConfig::timeout_millis`. Existing retry backoff derives from the
same timeout; its arithmetic, attempt count and deadline addition saturate
instead of wrapping. The native launcher still supplies 2000 ms through DNS's
`elect-ms` manifest entry. No handshake or request budget is raised, and vote,
quorum, membership, replay and current-term commit rules are unchanged.

Changing the native launch value requires rebuild/restart. Review heartbeat
frequency, persistence/link latency, bounded-read freshness and original client
budgets together. Election scheduling jitter is not cryptographic entropy or
authenticated time. The manual Configuration chapter and
[Raft contract](../../reference/raft-conformance.md) document these relationships.

A deterministic host test uses the same survivor identities, a three-voter
configuration with one voter absent, and a 250 ms one-way link. It delivers real
vote requests/responses through the production node handlers without fabricating
votes or resetting membership. The parent policy fails to elect within 20000
simulated milliseconds. The correction elects survivor 0 at **7560 ms, term 2**.
An additional control checks deadline/backoff saturation at the integer boundary.

## Native fixture and scope

`scripts/run-management-failover-test.py` builds one ARM kernel and boots three
isolated NVMe-backed nodes with the same complete signed/encrypted management
profile. It starts the lowest durable identity first and waits for each explicit
stable membership change. All three guests keep their ordinary network driver,
CQ transport, TLS engine, UTC service and signed controller admission.

A separate, CPU-less QEMU process connects its user-network backend and a stream
backend through hub ports. This supplies one DHCP/NAT peer and loopback-only
forwards on the shared Ethernet network, allowing standard verified Python TLS
clients to reach both management interfaces. It neither terminates TLS nor
injects guest time. The arrangement uses QEMU's documented
[hub/backend connection mechanism](https://www.qemu.org/docs/master/system/devices/net.html#hubs).
The guest's ordinary SNTP/holdover source remains unauthenticated.

The fixture's global service certificate is deliberately shared by these three
development nodes. This qualifies operator authentication and role enforcement;
it does not supply per-node identity enrollment or authenticate the L2 consensus
channel. Public development trust remains separate from protected provisioning.

The observer requires fresh, complete, agreeing term/leader/membership snapshots
and exactly one observed leader. Each node first passes its **18/0/0** kernel
suite, DHCP/time readiness and verified management access. Read-only readiness
observations retain one phase deadline and record unconfirmed transport failures;
certificate-verification failures remain terminal. Signed mutations are submitted
once and never replayed after uncertainty. A reader is denied the same body that
an enrolled administrator may submit, and all live nodes must observe the signed
policy sequence after acceptance.

The default campaign includes VIP controls. `--direct-only` explicitly limits
the campaign to surviving node endpoints and leaves VIP reconnection
unqualified. Both scopes failed in this batch.

```sh
scripts/build-catten-services.sh --embed
python3 scripts/run-management-failover-test.py --firmware /path/to/ARM-UEFI.fd
# Separate scope after retaining any full-campaign failure:
python3 scripts/run-management-failover-test.py --firmware /path/to/ARM-UEFI.fd --direct-only
```

## Failed and limited runs

The [tracked evidence](2026-10-27-security-management-failover-evidence.json)
retains artifact hashes, concise native diagnostics, vote metadata, policy
receipts/readbacks and the separately scoped final result. It needs no temporary
log links.

| Run | Observed result |
| --- | --- |
| Original timing | Three voters and first signed follower admission succeed; after leader termination both survivors repeatedly campaign before a replacement is observed at term 25. A later read ends at EOF. Kernel `9796b9f8731dceb8aa149ee729de2cfb59138e58cbc075013342c26443744ca5`. The EOF's cause is not established. |
| Original timing with bounded vote trace | Request/response frames show repeated same-term denials between both live voters. No replacement leader within the original 150 s phase budget. Kernel `0d8f6bd93f5e84a56599b4406e6aad89fb5e58294e2b6dba26634ac74836fe1b`. |
| Corrected timing, firmware stall | No kernel entry before the node-readiness deadline. Kernel `6ba101b656346bffcf60cec86fbb010bbce437907a178b134eab9902c023032c`. The image contains an ARM loader/ELF; the firmware cause remains unresolved. This run says nothing about corrected Raft behavior. |
| Corrected timing, explicit boot priority | Stable three-voter term 1 becomes a survivor-led term 2; follower mutation and sequence-2 readback succeed on both survivors. The full campaign then fails during a VIP TLS handshake. Kernel `ffcf50ae4f1fd0767e0245ead6ec87a2666d5030943f9d5772d4812cf0d1336b`. |
| Corrected timing, direct-node repeat | Stable three-voter term 1 becomes a survivor-led term 2; generation/sequence 2 is read back on both survivors. Unknown/missing-client controls and healthy-reader recovery pass on node 1's status interface. The next healthy-reader handshake to node 2's status interface times out, before that endpoint's rejection controls begin. Kernel `021defe0efbc1118ae87113838b3eaf58673c6c18c680098899d720b2d4e3518`. |

The traced parent run includes both live voters sending same-term requests and
returning `granted=False`; packets do traverse the live peer link. After the
correction, the term-2 request receives `granted=True`. The controlled host
regression isolates the fixed-window failure; this does not prove every earlier
startup/election delay had that sole cause.

At the VIP failure, the new advertiser still publishes a three-backend table
despite one killed node. That observation is consistent with selecting an
unavailable backend, but the timed-out flow's exact backend is not established.
The later direct-node failure shows that availability investigation cannot be
limited to VIP selection. Neither failure's transport cause is established.
The timeout is **not** authentication rejection, closure or quiescence evidence.
No retry of an accepted mutation, larger handshake budget or plaintext fallback
is introduced to make the case pass. VIP recovery and firmware intermittence
remain outstanding. The runner now journals the exact endpoint and probe step
before each control to localize future failures. That diagnostic addition and
local loader verification were made after the captured native runs; they are
host-checked here, without claiming a new native result.

The direct-node repeat retains the complete post-admission checkpoint: reader
mutation denial, administrator receipt generation **1** through node 1, then
receipt generation **2** through node 2 after node 0 is terminated. Both survivors
read back policy generation/sequence **2**, two assignments and identical
validity bounds. All three initial kernel suites report **18/0/0**. Only the two
recorded node-1 status rejection controls complete; the remaining six direct
controls and all VIP controls are unqualified in this repeat.

## Validation and remaining gates

The host Raft suite passes **55 existing tests plus two new integration tests**.
Strict Clippy passes for the core and the actual ARM/x86 DNS/management service
targets. Both service bundles are rebuilt; native assembly permissions, rustfmt,
Python/shell syntax and whitespace checks pass. **35 targeted Python tests** cover
coherent failover observations, bounded vote parsing and the shared strict
rejection probe, startup/rotation/runner regressions and CLI deployment status.
The new observer/trace suites are registered in the host runner;
no fresh full host-suite or TLC run is claimed. Dependency versions are unchanged.

The rebuilt manual has 254 pages. Configuration pages 200-203 are rendered and
inspected, with no chapter overflow or undefined references; the 190 existing
global overfull warnings remain elsewhere.

This scope does not qualify uninterrupted TLS sessions, accepted mutations
interrupted at leader loss, arbitrary node substitutions, platform anti-rollback,
authenticated time, or cluster-peer encryption/authentication. SEC-04/08/09/10
retain those separate boundaries; broad fault/pressure qualification remains.
No numbered finding is closed.
