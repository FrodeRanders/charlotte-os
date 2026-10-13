# Canonical registration and S3 launch profiles (S08b1)

Date: 2026-10-13. Continues [S08a](2026-10-13-protocol-simplification-object-store.md)
and the [ordered simplification ledger](../../reference/api-simplification.md).

## Current contracts

Catalog register opcode 0x01 always carries name, node and deployment-generation
u64. Zero explicitly identifies a system service; a nonzero application value
still requires the exact committed desired generation and replica node. Missing,
partial or trailing bytes reject before generation reservation or mutation.
One `encode_register(name, node, deployment_generation)` replaces the two
encoders and removes the suffix-free branch from local/remote DNS submission.
Preparation and exact-generation activation remain separate. The remote relay
already required its generation field and retains that current grammar.

The snapshot tag advances to catalog-v18 (`CATALOGI`, numeric
0x434154414c4f4749, little endian); both persistence tags advance together to
`CRFTSTA3`/`CRFTLOG3`. Catalog versions 1–17 and pair revisions 1/2 reject
before restore/replay, including old logs with no application snapshot. Record
widths and object IDs are unchanged. No old decoder, tag rewrite or automatic
store reset is added. Object-store v4 and runtime ABI 3 remain unchanged.
Future deployed-store changes still require explicit, interruption-tested
migration preserving election and committed state. All current nodes/artifacts
must be refreshed coherently for this internal protocol change.

S3 static provisioning and encrypted operational pickup already supply immutable
`CHS3PF1` through read-only launch memory. The manifest fallback, per-key exports,
region/port defaults and duplicate validator are removed. Missing profile memory
or mapping/decoding failure follows the existing failed-start path before
connector publication. The shared codec validates complete fields, exact lengths,
known flags and reserved zeros; the service retains its existing zeroizing secret
owner. Explicit empty prefix/namespace remain valid current choices. Kernel
provisioning and operational pickup are current callers, not compatibility paths.

The unused `find_signature_desc` export is removed; every producer/consumer
already uses `signature_location`. CLS2 format, name-bound verification and
signature policy are unchanged. Living Markdown/LaTeX, contributor instructions
and C18 [cleanup evidence](../../reference/cleanup-recovery.md) are updated.
This adds no custody registry, cleanup receipt or physical retry authority.
See [catalog contract](../../reference/catalog-snapshots.md),
[persistence](../../reference/raft-persistence.md) and
[S3 configuration](../../reference/s3-client.md).

## Failed checks retained separately

Initial service builds fail E0432 on DNS's stale import of the removed
`encode_register_deployment`; the import is removed with the migrated calls.
The first host service test result is **108 passed / 1 failed**: the new
registration test unwraps a lookup before activating the prepared generation.
Preparation correctly does not publish a live registration. The fixture now
explicitly activates after successful submission; rejection assertions and
production activation policy are unchanged.

### First x86 TLS ingress/HTTP run

Instance `s08b1-ingress-x86`, fresh storage, four LPs, ports 19125/19124,
--deployment-ingress-test --http-test --timeout 240. Guest **19/0/0** at
19.715143 s, bitmap 0x323bbff. S3 serves TLS at 15.111860 s; the agent selects
x86 variant 1/ABI 3 at 28.970381 s, verifies 33,072 bytes at 32.112659 s and
launches at 32.132194 s. CLI readiness and the committed cluster response both
report greet generation/service-generation 1, one desired/ready replica.

The overall runner exits **1**. Deployment status, malformed/oversized framing
and complete-invalid-body controls pass, but the healthy status request after
an EOF peer exhausts its two transport retries:

```text
test-deployment-http-liveness.py:154 main -> status
line 72 status -> line 60 request -> line 34 read_response
client.recv(4096): ConnectionResetError [Errno 54]
error: guest HTTP keyhole was not validated from the host
```

Later abortive/idle/trickle controls are not qualified by that failed run.
The release worker records initial HTTP 503/-2 and repeated response resets
before accepted generation 1 and readiness. The final serial capture reaches
303.373470 guest seconds: the runner's tick-count window excludes time spent
in synchronous host probes and is not a hard wall-clock deadline. This is
retained as a harness limitation, not a claimed 240-second wall-clock bound.
No contending-owner or TCP teardown capture identifies the reset's cause;
successful rollout/guest tests do not turn this into a passing HTTP run.

## Final validation

- Host suite: **439 passed / 0 failed / 4 ignored**, plus existing Python and
  signing controls. Registration tests cover every truncated prefix (including
  the complete old suffix-free form), extra bytes, no mutation on rejection,
  and subsequent exact activation. Existing deployment-generation/replica
  fencing and snapshot roundtrips pass. Rejection tests cover revisions 1/2
  of complete, mixed and log-only persistence pairs without writes. All older
  catalog tags, including v17, reject without changing live state.
- New S3 shared-codec tests retain nondefault explicit values and empty
  prefix/namespace; missing region/port/credentials/rights, invalid paths/headers,
  TLS without a CA, unknown version/flags, reserved bytes, truncation and
  declared trailing input reject without defaults. Placeholder CA bytes in
  these host tests do not constitute TLS authentication evidence.
- Both signed service bundles rebuild; strict service and mixed-profile kernel
  Clippy pass on both targets. Rust formatting and source diff checks pass.
- The manual rebuilds to **212 pages**; changed pages are rendered and visually
  checked. Inline hashes retain each stated artifact/run profile.

### Linked ARM/x86 deployment

Fresh instances `s08b1-arm/x86`, four LPs, --deploy-test --timeout 300, socket
port 19121, MACs 52:54:00:12:34:01/02. A coordinator registers stdout pipes,
starts the x86 runner after ARM boot publication and confirms ARM QEMU PID
66840 listening before x86 boot. Both runners exit **0**:
ARM **22/0/0** at 88.396202 s (0xcfffff), x86 **20/0/0** at 40.552107 s
(0xcfbbff). Three signed pins move ARM variant 0 → x86 variant 1 → ARM variant 0;
exact-generation readiness, cooperative/zero-grace forced retirement and
protected-namespace denial pass. Cross-node registrations and DNS/join pass.
ARM Sitas reports length 3 / 1,024 mailbox records / join sum 81 at 7.883548 s.

### ARM TLS ingress

Instance `s08b1-ingress-arm`, four LPs, fresh storage, port 19123,
--deployment-ingress-test --timeout 240: **20/0/0** at 13.658518 s
(0x303ffff), runner exits **0**. Mandatory-profile S3 serves real TLS at
9.741841 s; signed release selects ARM variant 0/ABI 3 at 16.946096 s,
verifies 97,520 bytes at 21.083456 s, launches at 21.103378 s and reaches
one desired/ready replica. Initial connection refusal and three HTTP 409/-23
unknown-profile rejections precede accepted release; this is eventual validated
admission, not clean first-attempt success or permission to replay accepted work.

### x86 TLS ingress/HTTP repeat

Instance `s08b1-ingress-repeat-x86`, ports 19127/19126, same source, feature
profile, four LPs, fresh storage and --timeout 240. **19/0/0** at 19.488123 s
(0x323bbff), runner exits **0**. S3 serves TLS at 15.223201 s; the agent selects
x86 variant 1/ABI 3 at 31.457361 s, verifies 33,072 bytes at 34.037057 s and
launches at 34.060507 s. One desired/ready replica is confirmed. All deployment
HTTP framing/invalid-body/EOF/abortive/idle/trickle controls pass; trickle rejects
in 30.23 s. Bounded transport resets still require up to 2/2 retries. CLI
connection refusal, HTTP 503/-2, three 409/-23 unknown-profile responses and
eight response resets precede accepted release/readiness. This is eventual
validation, not exactly-once remote invocation or resolved TCP teardown.
Ephemeral TLS/descriptor inputs change between runs, so this is not an exact
guest-artifact causal comparison with the failed control.

## Remaining qualifications

S08b1 closes no numbered security finding or general physical quiescence proof.
The earlier ARM stall and DMA-map contention remain unresolved. The new failed
x86 EOF-recovery control remains separate even if a repeat passes; fixture CA,
port, nonce and signed-envelope inputs can change between repeats. Shared-CQ
per-waiter readiness, HTTP teardown/intermittence and production trust remain
separate work. Optional operational release tails represent current releases
with/without connector bindings; deleting one requires a coherent explicit
replacement. Other DNS encodings, Kafka/profile extensions, raw physical queries
and mailbox adapters remain S08b2 candidates with distinct ownership contexts.

## SHA-256 artifacts

| Artifact | SHA-256 |
| --- | --- |
| Linked ARM kernel | `e00d8945adc1a2170cf78e0c0c366ff9bc7611f282695f7417cf36f8565fe61f` |
| Linked x86 kernel | `632b169c044cc10056e16e65eaf0036422b4b2ff632fd5dabecf4bceb7d0098c` |
| Linked signed fixture bundle | `f83cf359ed227a26db4125066106d0e623f4ac204cf61a04aa7e50bfdc57d93f` |
| ARM TLS ingress kernel | `698be28017d1cb0ab34799370d9ffefce5d7fc4a74afbeb177ccc49ed458cde0` |
| First x86 TLS ingress/HTTP kernel, failed control | `22082cba0dd96641f6972f444bd37757217893aaf0d292fda1ebab5819d873f2` |
| Repeat x86 TLS ingress/HTTP kernel | `45465ef78873b0d51be4ed0ea59f16e598278e27bb67407d52c40a085e896893` |
| ARM signed ingress descriptor | `f765ff71639461a87bf7565e436f893df6294f7d8d1763cb27e3de3059ea75c7` |
| ARM signed ingress release | `8dfdeb1f3d761f9c83c4280728aa78fdec9b4d319c13558c4ddc56fe59ac2c99` |
| First x86 signed ingress descriptor | `3ad1e3e24d229db8a64042b98fa27526f05b9b1bea8e19caf90c14c779ac8208` |
| First x86 signed ingress release | `c78554eaab384e69b39487ba4b72a5bf76eb09e18f42206a1673d345e6a4a6c8` |
| Repeat x86 signed ingress descriptor | `1a7822a3462c4db44bc94c5d94edaf537d8d4b1fa8361241d9f80a0feb8df057` |
| Repeat x86 signed ingress release | `74612279e6a8d214f58511a092a2056ec1f7a2f760b344a3fc3c9286e1597854` |
| Signed ARM DNS | `92a60b6dab02d838de7c9e85771db76e9e35fec3052c017026015bd1dbb4fa0c` |
| Signed x86 DNS | `b243c2af847481e396c5fef844b66f9bf2b26702a5fb4ceac38dd6cfcece0f7d` |
| Signed ARM S3 | `9b419c5ba5eccb0da932bf901e60a517ea28f65d2197e01cd14aaf0f7eeb9253` |
| Signed x86 S3 | `4f09aed325f9552423db6433ddccdf8b5bf4070fc326237bd98c91fa1d81e4f2` |
| Final 212-page manual | `873236fc05a26fbaefb9b94fddb4ccc59e901381ed5f43f7e9d8798ff50cb70d` |
