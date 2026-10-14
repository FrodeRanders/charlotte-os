# S08b2c2: complete generation-fenced DNS invocation

Date: 2026-10-14. Baseline: `47e79fd8` (S08b2c1). This batch replaces the
remote invocation grammar and updates every producer/consumer together. It
retains the containing pending-call/dedup family; registration/retraction and
other raw API replacements remain separate work. See the
[current contract](../../reference/dns-invocation-relay.md) and
[batch ledger](../../reference/api-simplification.md).

**Current status:** implementation, host controls and both architecture builds
and strict checks pass. Final-source linked DNS/deployment execution remains
unqualified: both attempts failed during network startup. The earlier passing
pair predates the deadline correction and is not a substitute for final-source
execution evidence.

## Changes and review boundary

- Request/reply tags `0x26/0x27` replace retired `0x10/0x11`. Request caller
  and name have complete u32 lengths, are nonempty and each fit 4,096 bytes.
  Both decoders check the tag and exact frame termination. The reply is exactly
  33 bytes including its tag; prefix and trailer acceptance is removed.
- Request writers validate identities before fallible allocation. Decoders
  validate the whole record before fallible identity copies. Both outgoing
  paths prepare bytes before publishing in-flight state; rejection creates no
  outgoing invocation and returns the existing error status.
- After checking source-MAC/registered-peer/caller equality, the target prepares a fixed
  `PreparedReply` before dedup admission or local submission. The existing
  remote local-call destination retains that buffer, replacing its redundant
  generation field. Ordinary submission rejection returns the complete
  destination. Completion consumes the original buffer and changes only the
  result bytes, without a new codec allocation. Reply-preparation failure logs
  once, admits no target operation and sends no fabricated completion.
- The raw scalar local lookup rejects empty names, names above eight bytes
  and trailing-zero names before submission. It cannot route a complete
  long name to an eight-byte prefix. Its full-name owning asynchronous
  replacement remains pending; full transport identities do not widen local
  publication or invocation policy.
- Review identified a fresh invocation budget after a follower query. The
  final source carries the original query deadline into local/remote
  invocation, rejecting an expired routing result before submission with
  `ERR_NOT_LEADER`. Once an invocation is sent, timeout remains
  `ERR_UNCERTAIN`; the target uses its own local progress clock/budget.

Source/session/call-ID/expected-peer/generation fences and the existing bounded
dedup window with per-peer reply ACK settlement remain. No accepted invocation
is replayed. Catalog-v18, Raft pair-v3, DNS interface 3 and runtime ABI 3 are
unchanged. No new registry, readiness source, polling path, capability custody,
cleanup receipt or physical retry authority is added. Source-MAC/registered-peer
equality does not newly qualify cryptographic node identity. C18 records the same
containing family in [cleanup/recovery](../../reference/cleanup-recovery.md).

## Host, build and static evidence

`scripts/run-host-tests.sh` passed **452 Rust tests / 0 failed / 4 ignored**,
including three new invocation controls. Existing Python signing, secure-boot,
boot-input, syscall, tooling and object-store controls also passed. After the
deadline correction and additional complete oversized/empty decoder cases, the
services host repeat passed **120 / 0 / 0**. The other host packages were
unchanged; this repeat is not represented as a second whole-workspace run.

Codec controls cover the complete request layout, signed scalar argument,
prepared reply correlation and preservation of its original allocation,
identities sharing the former 255-byte prefix through exactly 4 KiB, oversized
caller/name writers and complete oversized frames, empty identities,
wrong/retired tags, complete former grammars, relabelled old requests, every
truncated prefix, overflowing declared lengths and trailers.

Both final architecture-specific signed service builds passed. Strict service
and kernel Clippy (`-D warnings`) passed on ARM and x86, with kernel features
`acpi,virtio_net_test,disco_net_test,disco_cross_node_test,dns_net_test,deploy_net_test,clusterctl_test`.
Native runners enforce executable/read-only assembly sections. Workspace
rustfmt and whitespace checks passed. The final manual was compiled twice,
with no remaining undefined-reference/rerun diagnostic; pages 89–90 were
rendered and visually inspected. It remains 212 pages. The stale manual claim
that automatic endpoint-death retraction was future work was corrected to the
already implemented, exercised behavior.

## Earlier linked run, before deadline correction

The first invocation-codec source passed the linked four-LP, fresh-storage,
signed deployment fixture on localhost port 19137. Both runners exited 0;
QEMU PID 7990 was confirmed listening before x86 boot.

| Observation | ARM | x86 |
| --- | --- | --- |
| Final selftests | 22 / 0 / 0 at 87.377498 s; bitmap `0xcfffff` | 20 / 0 / 0 at 40.140062 s; bitmap `0xcfbbff` |
| DNS success | 87.376434 s | 40.138369 s |
| Signed deployment success | 87.375652 s | 40.137586 s |

This run exercises the codec and prepared reply changes, but **not** the final
original-deadline handoff. Its staged DNS object bytes were retained and checked
with the read-only v4 inspector, separately from the final artifacts:

| Initial staged DNS ELF | Bytes | SHA-256 |
| --- | --- | --- |
| ARM | 899,816 | `b6f49386b47ab58c5d891e32d8514cf18a730d1f845b3d461a4e32749571e343` |
| x86 | 931,520 | `81f76a1bb197ab798d0ef8d112b863ac1579e39654b8739f00251ac21f62cf8b` |

## Failed final-source linked run, retained separately

The first final-source pair used localhost port 19138, fresh stores, four LPs
and the same 300-second guest budget. QEMU PID 12436 was confirmed listening
before x86 boot. Both runners exited 1; the coordinator exited 1. No fixture
was manually cancelled: ARM subsequently reached its own discovery deadline.

At 13.881311 s, the x86 NIC reported:

```text
[net] DMA map rejected pages=3 direction=Bidirectional DmaMapError { status: 14, phase: 3, cause: 12 }
[net] startup rejected stage=4 error=DmaDomainMap detail=0xc030e
```

Further 3-page Bidirectional and 128-page DeviceWrite/DeviceRead preparations
reported the same status/phase/cause. Current ABI names are `DMA_INVALID`,
`BACKEND` and `OPERATION_IN_FLIGHT`. These diagnostics do not identify the
contending owner, prove rollback or authorize map replay. The frame router
remained at stage 1; at 44.444931 s the network fixture ended **15 passed /
1 failed / 4 pending**, bitmap `0x1bbff`, failed `0x20000`, pending `0xcc0000`:

```text
panicked at crates/catten/src/self_test/el0_net.rs:105:22:
self-test deadline expired while waiting for EL0 frame router startup
```

ARM observed no discovered cross-node peer and ended **19 / 1 / 2** at
128.929478 s, bitmap `0x43ffff`, failed `0x40000`, pending `0x880000`:

```text
panicked at crates/catten/src/self_test/el0_disco.rs:116:26:
self-test deadline expired while waiting for EL0 disco cross-node peer
```

The observations are consistent with a failed peer network startup, but do not
establish its deeper cause. This run did not qualify final DNS routing or
deployment. Its kernel hashes were ARM
`44060030fb9ae65f1a87ecebe123cf530aa1d9238de75bed8be0b15af95c1599`
and x86 `5aa2d551bbe625547621d58bb98e95d191dbdd990a67a96f01a1ae462292fc8a`.
The staged DNS ELFs match the final signed hashes below. The unchanged-source
repeat also failed, as recorded separately below.

## Unchanged-artifact final-source repeat

The second final-source pair used fresh stores and localhost port 19139 with
the same four-LP/300-second guest configuration. QEMU PID 16781 was confirmed
listening before x86 boot. Both runners and the coordinator again exited 1.
Kernel, signed DNS and signed fixture hashes were identical to the failed
final-source pair above; no production source changed between these attempts.

At 13.915057 s, x86 again rejected the first 3-page Bidirectional NIC DMA map
with `status: 14, phase: 3, cause: 12`; startup at stage 4 recorded
`DmaDomainMap detail=0xc030e`. Subsequent 3-page Bidirectional and 128-page
DeviceWrite/DeviceRead preparations reported the same rejection. This time the
first recorded fixture deadline was discovery startup:

```text
panicked at crates/catten/src/self_test/el0_disco.rs:44:22:
self-test deadline expired while waiting for EL0 disco service startup
```

At 44.456698 s, x86 ended **15 / 1 / 4**, bitmap `0x1bbff`, failed
`0x40000`, pending `0xca0000`. ARM again expired waiting for its cross-node
peer at `el0_disco.rs:116:26`, ending **19 / 1 / 2** at 128.179539 s,
bitmap `0x43ffff`, failed `0x40000`, pending `0x880000`. No final-source DNS
invocation or deployment success is claimed from either failed pair. The
repeated backend rejection is a priority admission/readiness gap; the exact
contending owner and causal relationship to prior intermittent failures remain
unverified. No map retry or timer-poll workaround was added.

## Final artifact identity

SHA-256 values identify the final-source artifacts from both failed attempts.
The read-only v4 inspector verified complete staged DNS object
`0xfffe2f18f4769a5c` in all four final-source guest data images and matched its
bytes/hash to the corresponding signed ELF. Evidence is recorded here instead
of depending on temporary captures or ignored image/build paths.

| Final artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM debug kernel | 61,800,744 | `44060030fb9ae65f1a87ecebe123cf530aa1d9238de75bed8be0b15af95c1599` |
| x86 debug kernel | 58,678,776 | `5aa2d551bbe625547621d58bb98e95d191dbdd990a67a96f01a1ae462292fc8a` |
| Signed ARM DNS ELF | 899,816 | `2591378fb0313eaecbc126665e46e3de6e95d163ba16450528edc4704e65c5c1` |
| Signed x86 DNS ELF | 931,520 | `26cdd32e622845949242b79c5f412c7e61a6c5036d04b6701a43ba7cce9f7859` |
| Signed local greet bundle, identical on both architectures/attempts | 1,896 | `18214e99b1ca32221916450bbded6820d1ea5947849c61c83a209385e878501b` |
| Tracked manual PDF | 1,015,009 | `4c78076f3e1394630c8bc696f67e5afe4925f172c5dcb0467df0b1f81c450fdb` |

## Qualification limits and remaining work

The host controls do not inject allocation exhaustion, delayed routing at the
deadline edge, all live transport/dedup pressure or every raw cleanup failure.
The packed-name rejection gate is source-reviewed and compiled; the linked
fixture uses representable short names and does not execute adversarial long
local lookup names. Long identities have host codec evidence, not a native
long-identity cluster qualification.

Transport/dedup metadata allocation and destruction, the raw local completion,
connection and attachment adapters, wider IRQ/guard/Drop contexts and physical
quiescence remain separate. No HTTP, S3, Kafka or new pressure guest fixture
ran in this batch. Prior intermittent ARM stalls, DMA-map admission contention,
HTTP reset/EOF failures, shared-CQ per-waiter readiness and production
trust/custody boundaries remain unresolved. Neither linked success identifies
their historical cause or supplies new root, stack or backing release proof.

The immediate priority is diagnosing the repeated NIC DMA-map admission
rejection and defining readiness only with the exact public operation/root and
qualified IRQ context. Final-source linked validation must be repeated after
that boundary is addressed; an error or retained backing is not a replay owner.
The remaining simplification candidates are registration/retraction
`0x14/0x15/0x16`, then coherent owning
scalar/named and local packed-name replacements, operational release
representations, physical-address queries and mailbox word/capability contexts.
Current connector-binding and serialized boot/IRQ policies cannot be removed
merely by matching a legacy-looking keyword.

## Later qualification: DMA-map readiness follow-up

The [2026-10-14 follow-up](2026-10-14-dma-map-readiness.md) adds qualified
kernel map admission and passes two linked ARM 22/0/0 / x86 20/0/0 pairs with
the same signed final DNS ELF hashes recorded above. Its final pair also checks
real VT-d/SMMUv3 ordinary and exclusive maps parked behind complete-unit
ownership. Final DNS invocation and three pinned rollout generations now have
execution evidence under that corrected kernel. Original failed captures and
the verification limitations above remain historical evidence; this does not
identify their exact contending owner or qualify every DMA/IRQ context.
