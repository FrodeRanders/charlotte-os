# Checked CQ waits and retained reactor deadlines (S06)

Date: 2026-10-13. Continues the ordered [API simplification](../../reference/api-simplification.md)
after S05. Results, failures and hashes are retained inline. Temporary captures
and ignored build paths are not required to interpret this report.

## Current contract

Unchecked CQ syscalls 12/42 and their SDK/kernel APIs are removed, including
the ignored `min_complete` argument and the one-wake timeout adapter that
collapsed admission rejection into false. Old timer/wake tracing entries with
no remaining producer are removed. Syscall 88 is the only CQ wait, with explicit
(timeout milliseconds, queue, timed flag) arguments and pending/status outputs.
None is (0, queue, 0); every Some value, including u64::MAX, has flag 1. Wide
queue IDs, invalid flags and noncanonical event-only input reject before waiting.
Runtime ABI advances to 3; all signed variants and fixtures declare that contract.
Current store/descriptor grammars are unchanged. ABI 1/2 target matching rejects.

`catten_rt::cq::wait` returns typed readiness/deadline or admission/invalid-request
errors. Ordinary service reactors share a fail-closed policy: rejected waiting
logs and aborts the domain while its pending resources remain governed by the
existing retirement owners. Timer-backed admission pauses also reject failed
submission, failed wait and non-successful timer results, preventing hot retry
loops. Specialized grant/security admission policies retain pending operations
and their original deadlines; failed backoff itself cannot silently return.

All in-tree old wait callers are migrated. Raft, DNS, discovery and reliable
messaging retain their watchdog deadline across ordinary readiness and start a
new period only after a real clock event or watchdog expiry. Controller idle
waiting is event-only; bound endpoint and kernel lifecycle publication signal
its default CQ. Existing hardware watchdogs and missing notification contracts
remain explicit; this removal does not qualify them as event-driven.

The kernel reuses its exact-list-identity checked source, owning subscriptions,
absolute-deadline scheduler watchdog and unused-interest cancellation. It neither
observes completion results nor adopts queued attachments. No source producer,
physical release, cleanup receipt, registry or custody family is replaced.
C16/C18 [cleanup evidence](../../reference/cleanup-recovery.md) records this scope;
wider untimed allocation and outer IRQ/callback qualification remains G1/G7.

See [CQ contract](../../reference/cq-wait.md) and
[execution targets](../../reference/execution-targets.md).

## Sitas dependency

The backend is in the sibling repository, outside the CharlotteOS writable
workspace. A two-file patch was prepared, tested, checked and applied with the
required filesystem approval. Local Sitas commit
`d27f7e7b7a6e3657c9b39e53da02cf0cd8f91a78` contains the source and architecture
update, based on `2c0f800` (x86 backend baseline). The exact
[version-controlled patch](evidence/2026-10-13-sitas-checked-waits.patch) makes this
coordinated change reviewable without an ignored artifact or unpublished URL.

Both ISA wrappers use 88 and the explicit flag. Reactor and sleep retain one
monotonic budget; unrelated completion/peer wakes cannot restart it or end sleep
early. Wait rejection logs and aborts the domain because the portable executor
retries backend errors and parker/sleep have no error channel. Public parkers
sharing a CQ retain their 5 ms backstop: another reader may consume the shared
work-generation cursor. This is timer recovery, not per-waiter readiness.
The previous impossible-count filtering claim was incorrect: the kernel ignored
that threshold. Private per-shard reactor queues remain isolated.

## Failed execution and qualification

### Initial missing-CQ fixture

Fresh linked instances `s06-arm/x86`, four LPs, port 19067, MACs
52:54:00:12:34:01/02, --deploy-test --fresh-storage --timeout 300, both exit 1.
At ARM 9.934 s / x86 17.233 s, both panic at timers/event_tests.rs:260:

```text
assertion left == right failed
left: Err(UnknownQueue)
right: Err(AdmissionRejected)
```

The timer fixture opened a completion namespace without a CQ. The old boolean
adapter returned false and its assertion could not distinguish the missing
source from rejected admission. Checked waiting exposes the setup error.
The correction admits a real two-entry CQ in that same serialized fixture,
without changing production admission or weakening checked status. This exact
caller has its private trusted sponsor installed before publication and event
capture; no live sponsor swap occurs.

Both 300-second runners lack an authoritative final result. Last progress:
ARM 17 passed / 5 pending, bitmap 0x436fff / 0x8c9000; x86 12 passed / 8 pending,
bitmap 0x22bfd / 0xcd9002. These are partial observations, not passing suites.
No deployment result was qualified. Initial kernels are hashed below.

### Corrected fixture but zero received frames

Fresh repeat `s06-repeat-arm/x86`, port 19069, same flags/MACs/LPs, both exit 1.
CQ readiness, timer/watchdog pressure and cancellation assertions now pass on
both architectures. Discovery expires at el0_disco.rs:116:26 waiting for an EL0
cross-node peer: ARM 128.089786 s, **19/1/2**, bitmaps
0x43ffff / 0x40000 / 0x880000; x86 136.595270 s, **17/1/2**, bitmaps
0x43bbff / 0x40000 / 0x880000. DNS/join remain pending.

ARM's late status at 103.709552 s is stage 6, peers 0, rx 0, tx_ok 9,
tx_fail 0, decoded 0, call 9, hb 974, send 5; router rx/fwd 0, routes 5.
x86 router samples likewise stay rx/fwd 0 with five routes. NIC startup and
hardware TX completion pass; those do not prove a host LAN connection.
The x86 client reaches boot before the ARM listener in this repeat. No TCP
handshake/port-owner capture was taken for the failure; startup ordering is
an explanation to test, not established guest-code causality.

### Listener-first validation before final clock refinement

Final-bundle instances `s06-final-arm/x86`, port 19071, same flags/LPs/MACs,
both exit 0: **ARM 22/0/0** at 143.798250 s (0xcfffff), **x86 20/0/0** at
40.931315 s (0xcfbbff). Before starting x86, host inspection confirms ARM QEMU
PID 15472 listening on TCP *:19071. Both replicas complete three signed variant
pins and cooperative/forced retirement. The ARM Sitas fixture returns basic_kv
length 3, verifies 1024 mailbox-index records and join sum 81. CQ readiness,
source/watchdog rejection, zero timeout, maximum finite timeout and cleanup pass.

This run also contains the timer-backoff fail-closed change; it is not a strict
single-variable comparison with the zero-receive pair. The subsequent retained
reactor-watchdog change is validated separately below. The failed runs
remain preserved, not replaced by later passing execution.

### Delayed peer and stalled final-clock repeat

The manual `s06-watchdog-arm/x86` run on port 19073 started its x86 peer only
after the ARM fixture had already ended. ARM exits 1 at 127.068540 s,
19/1/2 (0x43ffff / 0x40000 / 0x880000); late x86 exits 1 at 132.979047 s,
17/1/2 (0x43bbff / 0x40000 / 0x880000). Both discovery deadlines expire;
this attempt does not qualify a live linked pair.

An automated stdout-readiness barrier then launched `s06-verified-arm/x86`
on port 19075, checking ARM QEMU PID 29197 listening at x86 boot publication.
ARM output stops after cluster control at 14.454350 s; it produces no
terminal result within 300 seconds and exits 1. x86 cannot discover its peer,
exits 1 at 139.674935 s with 17/1/2 and the same x86 bitmaps above.
No ARM stack/register capture was enabled for that run, so the stopping point
is not proof of deadlock cause, lost IRQ, quiescence or ownership completion.
The debug repeat enables register/stack capture and is recorded separately
below. Its late capture is not used to diagnose the failed run. The host
coordinator registers stdout pipes and waits for
readiness instead of polling a log file or consuming the QEMU TCP connection.

### Final-clock instrumented repeat

Same final guest artifacts, instances `s06-debug-arm/x86`, port 19077,
stdout-readiness startup barrier, four LPs, same MACs, fresh storage, 300-second
window. ARM additionally enables --debug-snapshot --gdb-port 16517; no scheduler
trace or changed kernel feature profile. Host listener inspection before x86
boot confirms ARM QEMU PID 38725 listening on *:19077. ARM completes **22/0/0**
at 88.039866 s (0xcfffff); x86 completes **20/0/0** at 40.553839 s (0xcfbbff).
Both runners exit 0. Three signed architecture pins and cooperative/forced retirement pass on both.
ARM discovers one peer at 65.943958 s; x86 at 19.484936 s. CQ checks pass at
9.380568/16.826846 s and ARM Sitas at 8.020870 s, with length 3 / 1024 records /
join sum 81. The final watchdog fields are in the four signed service ELFs
hashed below; some storage-loaded service changes do not change the ARM kernel
payload hash. Do not treat that kernel alone as the complete boot input.

The prior 14.454350 s stall remains unresolved: this passing repeat has no
capture of that failure and cannot identify its blocking owner or IRQ state.
It qualifies final API/deadline execution, not general liveness under every
interleaving. The debug capture is of the repeat after successful validation,
not retrospective evidence for the failed guest.

## Validation

- Final CharlotteOS host suite: **430 passed / 0 failed / 4 ignored**, plus
  signing policy, boot/secure-boot result Python and syscall-emitter checks.
  New tests distinguish CQ rejection from readiness/deadline and reject retired
  numbers. Exact ABI 1/2 target rejection is covered on both architectures.
- Sitas package: **2 host tests passed**, pinned-toolchain x86 no_std cross-check,
  strict all-target Clippy, rustfmt and rustdoc pass. Tests retain sleep/reactor
  deadline through successive wakes, backward time and maximum budgets, and
  distinguish rejected/invalid/event-only-deadline outcomes.
- Both signed bundles and ARM catten-user rebuilt. Strict service and linked
  kernel Clippy passes on both architectures. Native runners check kernel
  executable/read-only assembly sections. Rust formatting and source syntax/diff
  checks pass. The exact Sitas evidence patch is excluded from whitespace checks:
  its unified-diff blank context lines have the required leading space, which
  `git diff --check` otherwise reports as trailing whitespace in the artifact.
  Final clock refinements receive the checks below.
- Before clock refinement, ARM real TLS RustFS ingress `s06-s3-arm`, port 19084,
  --deployment-ingress-test --fresh-storage --timeout 240: **20/0/0**, bitmap
  0x303ffff at 14.088911 s, exit 0. Signed atomic release selects ARM variant 0,
  fetches 97,520 verified bytes, launches at 21.470142 s and publishes readiness
  at 22.210114 s. Initial unknown-profile HTTP 409/-23 rejects before later
  qualified admission; accepted requests are not replayed.
- ARM security `s06-security-arm`, port 19086, --security-test --fresh-storage
  --timeout 240: **19/0/0**, bitmap 0x2003ffff at 57.278632 s, exit 0. Two
  generations preserve signed-root, role, lookup/publication and retired-policy
  checks; controller/reply CQ pressure controls pass. Concurrent cancellation
  retires after 14,852 requests. This run precedes the last clock refinement.

- Final-clock ARM TLS ingress, instance `s06-ingress-final-arm`, port 19088,
  fresh storage and 240-second window: **20/0/0**, bitmap 0x303ffff at
  14.075184 s, runner exits 0. Signed release selects ARM variant 0/ABI 3,
  fetches 97,520 verified bytes, launches at 21.905500 s and publishes
  readiness at 22.647610 s.
- Final-clock x86 TLS ingress/HTTP, instance `s06-ingress-final-x86`, deployment
  port 19090, HTTP port 19089, fresh storage and 240-second window: **19/0/0**,
  bitmap 0x323bbff at 19.664414 s; the runner reports all deferred tests passed.
  Signed release selects x86 variant 1/ABI 3 at 22.109567 s, verifies 33,072
  bytes at 24.080170 s and launches at 24.108118 s. CLI readiness confirms one
  desired/ready replica. Cluster schema `charlotte.cluster.v2` reports greet
  ready, generation/service generation 1, identical desired/ready node pins.
  HTTP status, malformed/oversized framing, complete invalid bodies, EOF,
  abortive replies, idle-peer deadlines and trickled-body rejection pass with
  healthy controls. Trickled input rejects in 30.13 seconds. Some transport
  probes need bounded ConnectionResetError retries (up to 2/2); initial CLI
  connection refusal, HTTP 503/-2 and two response resets precede accepted
  release. These observations remain qualifications, not clean first-attempt
  success or proof of exactly-once remote invocation. Metrics at uptime 75,293
  ms report five running and 23 blocked threads, zero ready/needs-LP threads;
  this is one snapshot, not a general absence-of-spin proof.
- The Markdown/LaTeX manual is rebuilt to **212 pages**. Changed pages are
  rendered and visually checked; the final PDF hash is retained below.

## Remaining boundaries

This adds no completed numbered security finding or physical quiescence proof.
Shared-CQ per-waiter notification, telemetry/archive reply notifications, hardware
watchdogs and wider IRQ/allocator contexts remain open. The earlier DMA map
admission failure still lacks its contending-owner/quiescence evidence; see
[S04b2b](2026-10-13-protocol-simplification-records.md). Bounded HTTP transport-reset
retries and distributed authorization remain separate qualifications.
The next ordered simplification is S07 tooling/export aliases.

## SHA-256 artifacts

Each kernel belongs to its stated feature/run profile. Common target paths may
later be overwritten; these inline hashes preserve the original runs.

| Artifact | SHA-256 |
| --- | --- |
| Sitas coordinated patch | `031b7525a6be5c72adbfe35d75d4cac0f4ba835467e84a64d9af85b806d6a9be` |
| Initial ARM kernel, missing CQ | `d9a44b1056357799ca5ccb9e3f6046923619d247fdeb3dcda2b704e1c7eb9940` |
| Initial x86 kernel, missing CQ | `551ebac345e48aa797c2f67f74608b84a7a5289aa7e0bb6c78ba8780dc3631ab` |
| Zero-receive ARM kernel | `156c423c4bdd143cd7dcf4c15a57af2cd2755119262c4a5568e6781f0b8a07f3` |
| Zero-receive x86 kernel | `35c57b6f10157174ac610f51e63a112686f2b364c7b49a263d7295872a1aa122` |
| Initial/corrected fixture descriptor bundle | `36176a16e39fbce29e4052ad41acfc70e3cdf27b2e51205f014f405fc0f94a68` |
| Pre-clock listener-first ARM kernel | `b1f0ddb5a5b11fa008908e26804a413e0017f789303ea142017b50116d5ba2c4` |
| Pre-clock listener-first x86 kernel | `4073584a11f75accc6ae93b8a93cf98aa4d14e0cf158d8bf84ceb5408646f411` |
| Final greet/variant fixture bundle | `86018b073e2a143e5f8fed747da730b1a2dc465be1a035e590edb05eddc33854` |
| Signed ARM greet | `4e35b4a7fbe60b63c083b98cfaddebb21ef95351e83fee93fe5a09278ef1eb7f` |
| Signed x86 greet | `a9bb27d0d53d73c35ac9eb877f8dd62773b7066b26eb96be4627810ae643da2b` |
| Signed ARM catten-user | `a01b596eab01adc9f8bf7ac45228b1b21da9dcdb00b9e38799850358d01ed564` |
| Pre-clock ARM S3 kernel | `d30daf5c9c87792465ad0594f939e26754d9c891d8786cfc8103817a54327547` |
| Pre-clock ARM security kernel | `9858f80eefe062a1aa961aa7c5f661cf750fa115ff911a0589b2694e22db5946` |
| Pre-clock ARM S3 descriptor | `9556f26441242f50c1a72bc9b40c4c57c76332aa5722eef946165ac79850d52f` |
| Pre-clock ARM S3 release | `549851337fa11fed36abb5e4dc63c726acae6a07e6821cd315eb46ce437ae31f` |
| Final-clock x86 linked kernel | `2e8db7794be61b841c62217a938e2f8072f5dd70762a55060c643566e9c52618` |
| Final-clock ARM S3 kernel | `a00746763f6231e7091bbd7939c9a4cac02974e67380b4f9d31051f9b15b475f` |
| Final-clock x86 S3/HTTP kernel | `afc1691d626d35e1f02185dbb455dbce866e03b7b2ff8b1e6132a49f454444ac` |
| Final-clock x86 S3 descriptor | `5e787f87a021cdfcb54e3f09c1d34eee8d8b19d1421628ab7a4edd12cd89fe24` |
| Final-clock x86 S3 release | `60bca6fccb1255507af9c7ad30e9d68f99f94fca56865a3bb44dc6707e0059a2` |
| Final 212-page manual | `358b4b1ba56f32462e893223027f2c7b92209e7c09fb4cb872c22435970dc765` |
| Final ARM DNS ELF | `28bcc42b51333cf361f30cf6b57dcd9cb358e7f9fe6b5caddb2b0c7352efa27f` |
| Final ARM discovery ELF | `dfc0925218468bd969e4b3380ca778070cf2616d27c31a73c5ece67728628cd0` |
| Final ARM Raft ELF | `fd02a446b8c9f6c69b786bb8a9a7da3988f6ce87ee2ac940ea71be415f9441b4` |
| Final ARM relmsg ELF | `579bd3d58b534387db04de66fd896addf2dec48a7e2d38175eefc3adcb32f9de` |
| Final x86 DNS ELF | `a6075108d2915924a649d877e99a67f4b1b46a2830b04c20000e85f9b82359e7` |
| Final x86 discovery ELF | `13dd912297b3b33eebade346100a693de31e46e64cbe636ffe0681783b18ac47` |
| Final x86 Raft ELF | `0a7bd54e159427fd0a608f5892adde7a2b9595da94d3096da273a35f4e86bab0` |
| Final x86 relmsg ELF | `91f90a789ea97265d30a154b7bc7cc002060b8b0aae6496c7f6535117b16c9b0` |
