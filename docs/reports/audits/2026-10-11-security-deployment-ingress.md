# Deployment ingress and scoped publication progress

This continues `2745f501` and the [HTTP listener progress report](2026-10-10-security-http-progress.md), addressing another concrete SEC-06 availability gap. The deployment HTTP adapter still terminated its service thread on socket allocation/setup, accept submission, or unexpected accept results. Its pre-authentication parser also used infallible request growth, path allocation and a second whole-body copy. These are source-confirmed paths; no historical deployment-service crash is attributed to them without an execution capture.

## Correction

Deployment socket setup now returns an error to its connection loop. Failed preparation drops its owned socket/backing and retries after the existing 50 ms interval, checking lifecycle shutdown. Active accept rejection with exact `QUEUE_FULL` status retains the same `OwnedSocket`. Other errors leave that connection; no accepted request is retried because reply observation failed. Ordinary network errors no longer call the service's terminal `fail()` path. Launch-contract failure still terminates, and unavailable controller RPCs remain a separate progress boundary.

The shared `OwnedSocket::listen` wrapper borrows its socket owner through port-only setup and consumes its moved request backing exactly once. Both HTTP services use this wrapper. Allocation, mapping and submission rejection leave the socket owned, and normal cleanup uses the existing bounded close-admission policy. No bare owning kernel capability or new registry is introduced.

`deployment_http::RequestBuffer` owns the sole accumulated request allocation. Growth and percent-decoded path storage use fallible reservation. Completed POST bodies remove headers in place and transfer that same allocation into the typed request variant; no second attacker-sized body allocation is made. An allocation rejection leaves the accumulated prefix intact until its owner is dropped. Parser and allocation errors use static 400/503 responses, so reporting rejection does not require another string allocation. Sending still requires its ordinary fallible caller-owned transmit page; inability to send closes that connection rather than terminating the service.

Framing rejects duplicate Content-Length, transfer encoding, nonnumeric/trailing length text, overflow, invalid header names and unsupported HTTP versions. Per-route body limits and the 1024-byte header limit remain. The total request ceiling is 1,049,088 bytes (1024 + 1,048,064). Unsupported trailing bytes already present in the completing chunk reject; later unread bytes are discarded by connection close. Signature/descriptor checks remain in the controller path and are not replaced by framing validation.

The existing five-second header and thirty-second total request deadlines are unchanged. The adapter still serves one connection at a time. This correction does not provide bounded concurrent clients, guaranteed admission during a sustained connection flood, a deadline for controller RPCs, transport authentication or aggregate kernel/principal heap admission. Controller-result decoding and dynamically formatted status/control responses still have separate allocation paths; this report qualifies request framing and static parser-error responses, not every downstream allocation. Local Vec deallocation is userspace allocator reuse, not evidence that physical heap charges were refunded; root backing follows its existing account/retirement contract.

## Shared identity startup

The first x86 HTTP/DHCP boot exposed another startup boundary: DNS logged `[identity] object write failed`, then scalar DNS fatal 6 and domain abort; deployment ingress never reached its listening marker. Review found that DNS and discovery could both observe a missing shared-read identity object. The old helper treated `ERR_EXISTS` as successful creation and both callers attempted a write, although only the original creator held mutation authority. This is a source-confirmed race. The captured write diagnostic did not include the exact error code, so the failed run alone does not establish that ownership rejection, rather than admission pressure, caused that particular write failure.

Identity initialization now borrows launch-owned name-service authority, owns its object-store connection, and moves owned request pages through typed pending calls. Only exact `ERR_OK` creation authorizes one size/write/flush sequence. A creation loser, or reader of unfinished zero-filled storage, waits for a complete identity encoding without writing. Existing malformed state is never repaired by a foreign caller; a valid foreign mnemonic rejects. The same connection transaction bounds admission retries and incomplete-publication polling to ten seconds. Name lookup and already accepted RPC observation remain unbounded; this is not a controller progress guarantee. Rejected move admission retains and retries its exact returned owner; accepted calls are never replayed. Encoding and decoding allocate fallibly within a 4096-byte identity limit. Shared-read object-store mutation checks remain unchanged.

Three production state-machine regressions exercise a creation loser across missing, empty and resized-zero storage; initial observation of resized-zero storage; successful creator write/flush; failed writer without replay; and corrupt, foreign, unfinished or missing-without-MAC state without unauthorized writes. These controlled storage schedules establish the handoff rule, not a forced guest scheduler race or physical cleanup proof.

## Catalogue verification stack

The first signed-release fixture passed all 20 kernel checks (`0x303ffff`, zero failed/pending) but did not realize its release. Its kernel SHA-256 was `cc9d9fee9712b47423ea7dad3402c5f96c9b66537c8e042842d872e3488efc3f`, signed release SHA-256 `9dfe6a2b8329a43052df40e60b40e086884147c4263853e9733c891d91f8d7ab`, and runner output SHA-256 `76eaf18d023bde5523d4e8ff9c36a46605e25db9b160f3713b2a74885e57df28`. The unchanged 240-second runner ended with `signed RustFS deployment did not become ready` and exit 1. The TLS S3 PUT/HEAD/GET/DELETE round trip passed independently; that does not turn the failed rollout into a pass. DNS faulted at guest 12.943 seconds: `FATAL EL0 DATA/INST ABORT: ASID=111 ESR=92000046 ELR=90e40 FAR=ffff60`. The fault address is below the user stack base `0x01000000`. Symbol lookup in the exact signed DNS ELF locates `0x90e40` at `ed25519_compact::edwards25519::GeP2::double_scalarmult_vartime + 28`: its prologue saves 96 bytes, subtracts `0xf90`, then faults on `str xzr, [sp]`. Subsequent signed-release client attempts report connection reset and no rollout readiness. This establishes the failing verification frame and guard boundary; it is not evidence of a malformed signature bypass.

The bootstrap catalogue now selects a bounded 32 KiB per-thread stack through `spawn_with_manifest_and_limits` and the existing stack account, rather than the ordinary cold-bootstrap 16 KiB default. This does not change signed deployment descriptor limits, thread quotas, stack reservation/growth charging or retirement policy. Validation of this adjustment is recorded with the signed fixture below.

## Host response observation

The host management client previously used `read_to_string` until transport EOF for all seven deployment/control HTTP operations. That can report a reset after an already complete response as failure. It now requires one unambiguous Content-Length, rejects transfer encoding, overflow, oversized headers/bodies and truncated data, and returns only after the complete declared body. It retains the 8192-byte response ceiling and existing five-/thirty-second read budgets, now as absolute response deadlines across fragmented reads. It neither accepts an incomplete response nor extends guest request deadlines. Host connect and whole-command/RPC progress remain separate boundaries.

Five regressions exercise complete response return without a second read (which would encounter peer reset), one-byte fragmentation, incomplete EOF/reset rejection, missing/duplicate/invalid/oversized length or transfer encoding, non-UTF-8 and an unterminated oversized header. The prior signed fixture accepted generation 1 and launched the verified artifact but timed out on readiness; the capture does not establish what each status read contained. The source defect and controlled framing tests stand independently of attributing that timeout.

## Protected publication readiness

Even after the stack correction, signed fixtures accepted the release and launched its verified artifact but timed out on readiness. A further repeat with the framed host client showed the same pending `greet` result. Source inspection found a control-plane mismatch: the agent used an ordinary legacy lookup to test registration, although scoped applications publish through the policy-mediated namespace path. That denial was correct. The catalogue also tried legacy resolution to recover the local publication generation.

`OP_PUBLICATION_GENERATION` now observes naming metadata without minting an invocation connection, altering policy or adding a waitlist entry. It accepts a packed short name or a caller-owned full-name copy, rejects malformed input, and checks an owning temporary endpoint-close watch before returning the live registry generation. Its reply contains only the scalar generation. The namespace adopts the receive exactly once at its raw ABI boundary, and borrows the existing single-threaded registry source only for the query; mapping and watch cleanup complete before reply. The borrowed `ConnectionRef::watch_closed` API owns only its completion, with success/drop and rejected-submission host coverage.

The agent uses this metadata observation for scoped application readiness. For scoped catalogue registration without an attached endpoint, DNS observes the actual local generation before submitting any Raft mutation; missing, closed or rejected observation cannot fabricate a generation or publish readiness. Existing legacy connection/watch handling is unchanged. Application lifecycle ownership stays in the existing `DeployedArtifact` held by the agent; this query is a point-in-time observation, not a root lease or new close-watch custody for scoped names. Independent crash/shutdown races and complete publication lifetime qualification remain separate work.

The security guest checks packed and memory-carried queries against the real protected `secavail` publication, requires the exact live scalar generation with no returned capability/memory, then repeats all five denied legacy lookup opcodes and queries again after endpoint retirement to require not-found. The read-only operation exposes metadata to a caller already holding namespace access, like its existing registry-status view; it grants no application discovery/invocation authority and does not weaken publication or lookup authorization.

## Host regression evidence

Production parser tests cover:

- A fragmented maximum-size operations body completes with the same allocation pointer and identical bytes.
- Actual host allocator/reallocator rejection returns `AllocationFailed` and preserves the existing prefix; a later owned retry completes. Percent-decoded name allocation rejection is exercised independently.
- Duplicate length, transfer encoding (including whitespace in the header name), trailing text, empty/overflowed length and invalid versions reject before body waiting.
- Header/aggregate/per-route limits and already-received trailing bytes reject.
- Fragmented headers and valid percent-decoded names complete; malformed percent escapes and invalid decoded names reject.

The allocator rejection hook exists only in the host test allocator, is local to its test thread, and exercises the real `Vec::try_reserve_exact` path. It does not claim a guest physical-allocation rejection. The maximum-size body is opaque test data, not an authenticated operations envelope.

The live deployment probe selects deadlines before execution: three 12-second normal request attempts, three-second pauses for serial listener handoff, a 35-second trickled-body observation against the existing guest thirty-second budget, and a 120-second total host probe budget. Its total-budget exception escapes transport retries. It checks missing-deployment status (404), malformed/oversized framing (400), a length-valid 3584-byte zero-filled invalid descriptor body (403), EOF, an abortive close after sending a status request, an idle peer, and one-byte-per-second incomplete-body recovery. Each rejection is followed by a healthy status control. The exact guest phase of the abortive close is not established.

Both QEMU runners include this probe under `--http-test`; the host check fails even if kernel self-tests pass. The deployment launch manifest enables admission tracing only for that test profile. A startup listening marker permits readiness checks without guessing a boot delay.

## Execution results

All qualifying runs used fully rebuilt/signed service bundles and fresh instance storage. Hashes identify the original local artifacts; the inline diagnostics/results are the evidence and do not depend on temporary logs.

Failed executions remain separate from passing repeats:

| Run | Artifact SHA-256 | Retained result |
| --- | --- | --- |
| Pre-identity-fix Intel HTTP/DHCP | Kernel `b13c205269436fd9249b2410f2aa13d32b2db77c98c2e45ebca8f5c978201559` | Kernel 18/0/0, `0x123bbff`, at guest 36.034 s; DNS identity write failure/fatal 6 at 12.943 s, deployment never listens, host startup timeout and cluster 503; exit 1. No packet capture or original write error code. |
| First signed fixture, 16 KiB catalogue stack | Kernel `cc9d9fee9712b47423ea7dad3402c5f96c9b66537c8e042842d872e3488efc3f` | Kernel 20/0/0 and S3 round trip pass; DNS guard fault in Ed25519, rollout fails at the unchanged 240 s window; exit 1. Exact disassembly, release/output hashes are above. |
| Signed fixture after stack correction | Kernel `7216727a97325810ecf726e8c73010d632e42a5ca39acec3739a9463b9c5fe9e`; output `2f8e64a94324ea9afddb43f4f8c5c533f4953b8084bf58c2163f0576f24c8cef` | Kernel 20/0/0; one read timeout, accepted generation 1, agent verified/launched 94976-byte greet; no ready result in 203 s; exit 1. Status response contents were not captured. |
| Signed fixture with framed client, before publication query | Kernel `ae74f48a072fe22eea8d8441aedc59160af9a3b42caf3434cb2292647024c8d6`; output `8123844e557b10b0baa95655a360a5ae40397e8a6bb5bae08f217415f6f85e6c` | Kernel 20/0/0 at guest 13.640 s; accepted generation 1, verified artifact/launch at 17.913/17.933 s; pending greet for 234 s; exit 1. The unchanged readiness failure is distinct from the host framing defect. |
| AMD HTTP/DHCP startup failure, before final publication bundle | Kernel `960a7fa802771b3478d1fa8b89eb2ec117746ce15fb965a1672fa28b07e6771e`; output `0ea150d99d8c701ab852436550aa7c69a66e06e6134d70eb77057846fbead404` | `el0_net.rs:105`: self-test deadline expired waiting for frame-router startup. At guest 42.121 s: 15 passed / 1 failed / 2 pending, passed `0x1bbff`, failed `0x20000`, pending `0x1200000`; HTTP never validates; exit 1. |

An unchanged-kernel, fresh-storage AMD repeat passes all 18 checks and the complete deployment/node/cluster suite. This establishes intermittence, not its cause or remediation. Frame-router startup remains an unresolved SEC-06/07 target-context qualification; the passing final bundle below does not erase that failure.

Final production-source validation:

| Run | Kernel passed / failed / pending | Bitmap | Host controls | Runner |
| --- | --- | --- | --- | --- |
| ARM security/HTTP/DHCP | 21 / 0 / 0 | `0x2123ffff` | Existing HTTP EOF/reset/idle suite, deployment suite, node/cluster JSON | exit 0 |
| x86 Intel HTTP/DHCP | 18 / 0 / 0 | `0x123bbff` | Deployment suite and node/cluster JSON | exit 0 |
| Same x86 kernel, AMD HTTP/DHCP | 18 / 0 / 0 | `0x123bbff` | Deployment suite and node/cluster JSON | exit 0 |
| ARM signed release/S3 fixture | 20 / 0 / 0 | `0x303ffff` | TLS object round trip and upload/atomic release/pull/verification/launch/exact-generation readiness | exit 0 |

The security/HTTP ARM kernel is `b0903fe5432218924578d07b71b3c5222661fda3f585e1c8e56a4f3f63f9d495`; Intel/AMD use `5c1b7a7b22514baebea305b0da7637903febb59f4935754f0868735db230016d`. The signed S3 fixture has a different profile and kernel `8750f77c7ce04ab969ebb7f6309e91be2b86641cf195fd47cb6d690fb9761ab1`. Never reuse those ARM kernels across profiles. Kernel completion occurred at guest 24.074 s (ARM), 18.423 s (Intel), 20.704 s (AMD), and 13.406 s (signed S3). Trickled bodies reject after 30.12, 30.19 and 30.20 host seconds respectively, followed by healthy status controls. The selected transport reset/timeout retries remain visible; passing does not mean every transport attempt succeeded. The x86 runner checks node/cluster JSON but does not invoke the older HTTP EOF/reset/idle script. Final ARM packet-capture SHA-256 is `3023e08fecfa0771ed326f16b426bd7ac2e89c1ac098b5390c7b50361f42ac0c`. Six exact queue-full accept rejections retain deployment socket 1 between guest 9.949 and 10.786 s, followed by the complete host suite. Both x86 boots log losing shared identity creation (Intel 12.402 s; AMD 14.648 s), then successful deployment listening (15.440/17.664 s).

In the successful signed fixture, the agent starts GET at guest 13.409 s, verifies 94976 bytes at 17.557 s, launches generation 1 at 17.576 s and publishes deployment/service generation 1 at 18.230 s. The host observes `state=ready`, `deployment_generation=1`, `service_generation=1`, `desired_replicas=1`, `ready_replicas=1`, node key 1122115291. Signed release SHA-256 is `84cb56d36ba029ba9fbf131ca8450f455ffd951e98df90f0e50bfacc8feae5e0`. One startup connection refusal is retained before acceptance. This fixture uses one valid signed component and public test trust; it does not qualify maximum-size authenticated operations or production key custody.

The combined security guest observes exact live generations through both query encodings, no returned authority/memory, all five denied legacy lookup paths, and rejection after endpoint retirement across two probe generations. Query success does not authorize invocation. Intermediate ARM execution also recorded actual listener setup rejection (`Ipc(Status(1))` and opaque `Ipc(CreationFailed)`) followed by successful listening and retained-socket queue-full accept progress. Those observations are not guest physical-allocation failure injection or a close guarantee under indefinite pressure.

Final signed ELF SHA-256:

| Artifact | ARM | x86 |
| --- | --- | --- |
| deployd | `0da63242da103c135a3dcb892f7f0c51203d0d798c0cc4839689e20e1b07e20c` | `2676f1cd5ee5780bd20779cbdd9e03eea703ffe99ddbdd99178728f995f59be8` |
| dns | `c2ab840cea3045ae899a1db8410ad30534e7c0f2fdcf1fb45c0267d392665d18` | `8b72e8a96ee9533223eff77f4a72b48c2b1c75b958f7230d461dd8c081652680` |
| ns | `237fe7b66de742acade9f320080b30b6fa95c30d0b7b9a46b3db8242ebabb1d7` | `8383258d1c58d7cb11f88048b41cccce22f238a034542131e47933d09019c27a` |
| agent | `2fe174f84021f537c8360d4eb526a5ea510041084164b2de5e4dfa8acab0d0f2` | `6d745464238d277d008c19a17679791503806c552c7e69b8e20f82ea08471783` |

The final host suite reports **368 passed, zero failed, three ignored**. Signed ARM/x86 builds, strict services/ARM-kernel/x86-kernel/host-CLI Clippy, workspace formatting, diff whitespace, runner shell syntax, probe CLI and total-budget exception checks pass. Development checks separately rejected an unused helper-extraction import, a collapsible identity conditional, two publication imports/qualifications, and Rust 2024 let-chain syntax in the Rust 2021 host tool; these were corrected. Direct host Cargo commands from the kernel-configured repository directory also failed with duplicate core language items; the documented host runner and host CLI Clippy from an external directory pass without that freestanding configuration.

## Reproduction and remaining acceptance

Build complete signed service bundles before guest launch. For ARM use:

```sh
scripts/build-catten-services.sh --embed
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=19090 CATTEN_DEPLOY_HOST_PORT=18390 \
scripts/run-aarch64.sh --security-test --http-test --dhcp-test \
  --instance audit-deployment-ingress --fresh-storage --timeout 180
```

For x86 build `scripts/build-catten-services-x86_64.sh`, then use `scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel` or `--iommu amd`, with separate host ports/instances, fresh storage and the chosen 180-second runner window. Reuse a kernel only for the identical feature profile and its fully prepared service bundle. The signed positive fixture uses `CATTEN_SKIP_EMBED_BUILD=1 scripts/run-aarch64.sh --deployment-ingress-test --instance audit-signed-release --fresh-storage --timeout 240` after the bundle build; it starts the disposable TLS RustFS fixture. The new probe can also run independently with `python3 scripts/tests/test-deployment-http-liveness.py --port PORT --ready-log SERIAL_LOG` while a guest remains alive.

SEC-06 remains partial. The serial body budget can delay a healthy client for up to thirty guest seconds. Concurrent hostile peers, SYN/connection floods, bounded controller RPC completion, NIC recovery and shutdown under sustained pressure require separate evidence. The intermittent AMD frame-router startup, independent scoped publication crash/shutdown races, maximum-size authenticated verification/stack measurements, SEC-07 R7-1 through R7-5 and wider cleanup/context gates remain open.
