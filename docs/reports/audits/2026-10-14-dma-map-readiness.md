# DMA-map readiness and linked invocation qualification

Date: 2026-10-14. Baseline: `45e9e043`. This follow-up adds readiness admission
to DMA map/map-exclusive syscalls and supplies final native execution for the
preceding DNS invocation batch. See the [current contract](../../reference/dma-map-readiness.md).

## Trigger and retained failures

Both final S08b2c2 linked pairs failed during x86 NIC startup with
`DmaMapError { status: 14, phase: 3, cause: 12 }` for 3-page Bidirectional and
128-page DeviceWrite/DeviceRead preparation. Startup recorded stage 4,
`DmaDomainMap detail=0xc030e`. The first x86 capture ended 15/1/4 on frame-router
startup; the repeat ended 15/1/4 on discovery startup. ARM ended 19/1/2 on its
missing cross-node peer in both attempts. Distinct panic locations, timestamps,
bitmaps and hashes remain in the [original report](2026-10-14-protocol-simplification-dns-invocation-relay.md).
No failed run is replaced by the passing results below.

ABI names are `DMA_INVALID`, `BACKEND`, `OPERATION_IN_FLIGHT`. Review found
rejecting map admission when another operation owns the complete unit or its
actual command engine. The fixed slot already had an owned source and producer
restoration notifications for grant admission. Maps did not use that contract.
Historical logs do not identify the original contender; no attribution to a
particular NVMe operation or fixture is made.

## Change and review boundary

The shared admission helper borrows a typed containing owner: `DmaCreation`
for grants/cancellation or `DmaOperation` for mapping. It preserves the fixed
slot, charged source and producer-LP deferred delivery. There is no new registry,
compatibility branch or per-instance wait mechanism.

Map preparation captures policy from its retained exact root before backend
access and retains its original `PendingPin` and transient interest in the same
public operation. Fallible source/disposal preparation precedes backend guards.
After wake, one callback takes the recorded nonretiring domain, actual engine
and original pin together under the backend hold, returning `MappingMaintenance`
before guard exit. Mapping and maintenance run once. Ready admission requires
an actual engine and all domain cells; abandoned claims remain fenced.

Unstarted rejection explicitly releases the original pin outside backend guards,
interest outside the device guard, then the exact root. Completion cannot
discharge a public claim while pending preparation remains. Pre-claim fallback
retains pin, root, claim and interest without allocator, callback, registry,
physical cleanup or logging work. Started-error quarantine, original charges
and physical completion proofs retain their existing policy.

Only syscall dispatch opts into scheduled admission; raw backend and
`device::dma_map` kernel fixtures remain rejecting. The unused exclusive-only
wrapper is removed. Waiting requires an already IRQ-enabled scheduled context;
no IRQ enabling is added. x86 SYSCALL dispatch supplies that context. Currently
masked ARM SVC can use immediate admission but rejects busy waiting; enabled
kernel fixtures separately qualify scheduled backend admission on ARM.
Arbitrary outer kernel guards are not qualified. Register layouts, syscall
numbers and runtime ABI 3 are unchanged.

Backend status/cause remains diagnostic and authorizes no caller map replay or
hardware/physical cleanup. Unmap's admission/completion distinction and the
restricted failed shared-DMA CPU-mapping owner remain unchanged. Readiness is
no completion or release proof. C13/C14/C18 document the same containing families
in [cleanup/recovery](../../reference/cleanup-recovery.md).

## Host and static checks

- `scripts/run-host-tests.sh`: **452 passed / 0 failed / 4 ignored** Rust tests;
  existing Python signing, secure-boot, boot-input, syscall, tooling and
  object-store controls passed. New kernel probes execute in guests; the host
  count is not kernel-wait coverage.
- Both architecture-specific service build/sign scripts passed. Services did
  not change; signed DNS hashes below match S08b2c2's failed final artifacts.
- Final strict kernel Clippy (`-D warnings`) passed on both architectures with
  `acpi,virtio_net_test,disco_net_test,disco_cross_node_test,dns_net_test,deploy_net_test,clusterctl_test`.
  VT-d, AMD-Vi and SMMUv3 compile; AMD-Vi runtime is not claimed.
- Workspace rustfmt and whitespace checks passed. Native runners enforce
  executable/read-only assembly sections. Contributor instructions, reference
  contracts, testing guide and LaTeX manual were updated together. Final PDF
  passes and visual checks cover changed pages 85-86, not a whole-manual repair.

## Deterministic containing-owner controls

Private-slot probes use the real public operation and two-page memory pin:
injected source preparation/waiter rejection, captured ordinary classification,
producer-before-registration readiness, detached-cell fencing despite engine
availability, masked rejection and one admitted claim. Root, DMA-cap and memory
close reject while preparation is held; ordinary explicit disposal releases
those obligations. Pre-claim Drop under existing allocator/table/device guards
retains all fields on the existing abandonment fixture root. Source closure
supplies no cleanup proof.

The scheduled private-slot fixture verifies an actually blocked thread, original
root/claim/pin retention, one restoring wake/claim, exact-generation exit
observation and final explicit release. These are containing-owner controls,
not invented hardware acknowledgements.

The final pre-driver real-unit probe uses the existing complete-unit preparation
owner without creation/reset or descriptor mutation. Its caller retains the
existing real domain root. Ordinary/exclusive workers enter the same scheduled
adapter as syscalls, register and block with their original public claim/pin,
then resume on exact unit restoration. Actual backend unmap precedes memory
release. No duplicate domain/engine is minted and no queue/epoch is reconstructed.
The controlled blocker is the probe's owned complete unit; it does not identify
the historical NIC contender.

| Real probe | ARM / SMMUv3 | x86 / VT-d |
| --- | --- | --- |
| Root / cap / domain | `105:2` / 4 / 10 | `105:2` / 4 / 9 |
| Ordinary wait -> admission | 6.926362 -> 6.928179 s | 13.208691 -> 13.211038 s |
| Exclusive wait -> admission | 6.930692 -> 6.932361 s | 13.222591 -> 13.224476 s |
| Real completion control success | 6.933398 s | 13.225931 s |
| Scheduled private-slot success | 9.183931 s | 14.773733 s |

Bounded one-millisecond fixture observation checks registration, whose
publication has no separate notification. The operation under test parks on
its owned resource source; this is not a production polling wait.

## Linked guests and final DNS qualification

Both pairs used fresh stores, four LPs per guest, `--deploy-test` and a
300-second guest budget. A stdout-readiness coordinator started x86 after ARM
boot publication and confirmed the actual ARM listener before x86 boot. Every
guest/coordinator exited 0; no guest panic or NIC map/startup rejection occurred.
The first pair, before adding the real-unit probe, used localhost port 19140
(listener PID 23195): ARM **22/0/0** at 92.472215 s and x86 **20/0/0** at
41.541360 s. Its kernel SHA-256 values were ARM
`dc10301402f2bfe360fdafd6fe97c3f859bf9a4a7cf146b13cefb6058b0b0ced`
and x86 `f8e2762f3f41a997b608b3c126f6f2057a770cc425790fd96fc5a496c21ac24f`.

The final pair with the real-unit probe used port 19141 (listener PID 26996):

| Observation | ARM | x86 |
| --- | --- | --- |
| Final selftests | **22/0/0**, 88.732120 s, bitmap `0xcfffff` | **20/0/0**, 40.802702 s, bitmap `0xcfbbff` |
| NIC online / real TX completion | 7.970800 s | 13.829406 s |
| DNS success | 88.731085 s | 40.801700 s |
| Signed deployment success | 88.730238 s | 40.800938 s |
| Controller controls | 13.681033 s | 19.742533 s |

ARM reported follower state at 75.270317 s; x86 served its remote invocation at
28.725641 s. DNS success includes replicated catalog, remote invocation and
generation-fenced endpoint-death retraction. Controller controls include upload,
retired/malformed inputs and fixture key ceremony. Both observed exact signed
variant pins:

| Generation | Node / variant | ARM status | x86 status |
| --- | --- | --- | --- |
| 1 | ARM `0x42e278ea`, variant 0 | 79.794554 s | 31.846521 s |
| 2 | x86 `0x42e27737`, variant 1 | 84.272018 s | 36.217305 s |
| 3 | ARM `0x42e278ea`, variant 0 | 88.556773 s | 40.674487 s |

Deployment success includes cooperative and zero-grace forced retirement and
legacy-forwarding denial. Unchanged signed DNS bytes now have final-source
S08b2c2 linked execution under the corrected kernel. NIC used immediate
admission in these passing pairs; no actual NIC wait was observed. The real-unit
probe separately establishes contention behavior. This is not a statistical
claim eliminating every startup failure or identifying historical causality.

## Final artifacts

The read-only v4 inspector verified complete DNS object `0xfffe2f18f4769a5c`
in both final guest stores and matched its bytes/hash to the signed ELF. Kernel
hashes also match the runners' serial artifact markers. Evidence is inline,
without dependencies on temporary logs or ignored build/image files.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM debug kernel | 62,331,776 | `984d37c2112ae049bc4c0f9e62bc55b2017e807488f6b814c59d04bc4ab13f3d` |
| x86 debug kernel | 59,261,424 | `b399ca05f6c8b0e021287aa8132fe46d29e71a4f38c8485a2f0378579307bf9e` |
| Signed ARM DNS ELF | 899,816 | `2591378fb0313eaecbc126665e46e3de6e95d163ba16450528edc4704e65c5c1` |
| Signed x86 DNS ELF | 931,520 | `26cdd32e622845949242b79c5f412c7e61a6c5036d04b6701a43ba7cce9f7859` |
| Signed greet bundle, identical on both architectures | 1,896 | `18214e99b1ca32221916450bbded6820d1ea5947849c61c83a209385e878501b` |
| Tracked manual PDF (212 pages) | 1,015,827 | `d6c98d92407ed3eba9c2ee47b3aa7e03b2742f31b0a80e352db838c116c9ad3c` |

## Remaining qualification

Masked ARM SVC contention remains explicitly rejected; qualified IRQ entry
support is separate work. Public unmap/close and whole-domain teardown readiness,
indefinite abandonment, wider allocator/metadata/guard contexts, AMD-Vi execution
and broader live admission pressure remain open. This adds no general physical
recovery, custody or replay owner. Earlier ARM stalls, HTTP EOF/reset failures
and production trust boundaries are not closed. No HTTP/S3/Kafka fixture ran.

Next is registration/retraction `0x14/0x15/0x16`, then the owning full-name/raw
API replacements in the ledger. Historical NIC contention attribution remains
unresolved although scheduled map admission and final linked DNS now have evidence.
