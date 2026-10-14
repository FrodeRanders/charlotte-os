# S08b2c4b: full-name NS generations and owned cleanup (2026-10-14)

This batch follows `e07a663` and implements the [NS interface 2 generation
contract](../../reference/name-service-generations.md). It removes packed
observation/retirement and raw pending unregister handles without weakening
registrar/manager authorization. This is scoped C18 progress, not closure of
production trust, physical cleanup or the historical IPC exit-watch failure.

## Implementation and limits

`charlotte-protocol-ns` supplies one strict CNSGEN01 record: exact total/name
lengths, a full 1..=256-byte name, and an explicit generation. Observation opcode
18 requires zero; retirement opcode 10 requires 1..=i64::MAX. Retired scalar,
bare-name and eight-byte generation records reject. The kernel NS endpoint and
SDK share interface version 2. Runtime ABI 3, DNS interface 4, relay tags,
catalog-v18 and Raft pair-v3 remain unchanged; no persistent layout changes.

NS receives generation transactions through one `IncomingMessage` adoption.
Mapping, complete decode, fallible name copy and explicit unmap precede policy
or registry mutation. Retirement still verifies the exact stored generation and
kernel-captured nonzero principal/roles against the registrar or service-manager
rule. A successful logical unpublish detaches its unique raw connection before
one owned adoption; best-effort close is not physical-release evidence. Other
raw NS registry/lookup adapters remain separate qualification work.

The checked owned observation helper returns rejection separately from confirmed
absence. Explicit DNS tombstoning submits no Raft command after observation
rejection. Its one accepted metadata RPC uses the original remaining five-second
monotonic budget; no connection authority is acquired. The agent convenience
wrapper keeps its existing readiness-only Option semantics. This bounded
synchronous NS observation can still occupy the DNS reactor; wider asynchronous
registration/metadata work remains open.

`Unregistration` owns its moved request's pending call, binds reply readiness to
the DNS CQ and carries one original deadline into the earliest reactor wait.
CallResult disposes unused returned capabilities. Exact generation success,
no-match, explicit rejection and uncertainty are distinct terminal outcomes;
no-match includes authorization denial and is not proof of prior removal.
Accepted work is never replayed. The existing publication claim is shared by
Raft-commit cleanup and the publication scan, preventing two submissions for the
same captured episode. Storage/preparation/submission rejection follows the
existing fail-closed domain policy with the containing transaction live; failed
CQ binding returns/retains the accepted owner through abort. The pending list is
bounded to 64; this availability limit is not a custody/retry owner. Runtime
pending close/cancel fallback retains its existing policy.

DNS lookup/catalog ownership does not grant retirement authority over somebody
else's NS binding. Dead third-party entries need an NS-owned endpoint-death
producer or explicit delegation contract; this remains open. The positive echo
fixture instead delegates a distinct `cluster.console.echo.primary.v1` endpoint
alias to DNS, making DNS its NS registrar. The optional byte-valued `cluster`
manifest key is a shared compile-time constant within the eight-byte ABI limit.
Its lookup/publication carry one five-second deadline, with owned calls and no
retry. Malformed types reject; ordinary echo launches have no extra dependency.

## Executed checks

| Check | Result |
| --- | --- |
| Final host suite | 474 passed, 0 failed, 4 existing ignored; signing/tooling/store/syscall controls pass |
| New codec controls (3) | Golden/full identity; 255/256-byte and trailing-zero names; action/generation semantics; invalid writers, every truncation, overflow, trailers, bad magic and retired frames |
| New ownership/authorization controls (4) | Exact principal/manager and stale denial; CQ binding and terminal results; retained accepted owner on bind rejection; unrelated turns, original deadline, timeout and Drop |
| Signed bundles | ARM and x86 complete after the signing process returned; independent NS/DNS/echo signature checks pass, with DNS/echo still class Service and no manager role elevation |
| Strict service/kernel Clippy | Both architectures pass with `-D warnings`, including the linked fixture feature set |
| Formatting | `cargo fmt --all -- --check` and diff whitespace check pass |
| Final linked new DNS controls | Both architectures execute full-name echo and all three signed variant-pinned rollout generations; hosting x86 observes actual long-name NS retirement |
| Final whole linked suite | x86 20/0/0; ARM has an IPC exact-exit failure and no authoritative complete result within the runner's 300 s budget. This is **not** a fully passing pair |
| Manual | Two final sequential successful TeX passes; 213 pages; final pages 89, 90, 91 and 106 rendered and visually inspected. A clipped existing audit link on page 91 was replaced with a short linked label |

In the final proof pair, x86 observed the full-name publication and rejected
retired/stale generation requests at 30.351297 s. Full-name echo returned 43 at
x86 30.361253 s and ARM 75.343048 s. x86 logged one terminal NoMatch at
31.322561 s for the separately published short binding; it did not replay.
Read-only named lookup observed the actual DNS-owned long-name NS tombstone at
31.930788 s, without the fixture submitting removal. Both guests completed
three exact signed variant-pinned generations with cooperative and zero-grace
forced retirement; protected greet forwarding remained denied. x86 finished
20/0/0 at 42.910315 s, passed bitmap `0xcfbbff`, failed/pending bitmaps zero.
ARM's DNS rollout success was at 87.576043 s, but its CQ fixture remained pending
after the IPC assertion; the runner returned failure at its 300 s limit. The
coordinator returned `arm=1, x86=0`. Both listeners were actually present at
peer boot publication; runner assembler-section checks passed.

These host controls use owning fake calls for the generic production state
machine, not live allocation/mapping exhaustion. Native NS retirement is positive
on the hosting x86 node; ARM performs full-name observation during rollout and
remote invocation. Full 256-byte publication, all live mapping/cleanup rejection,
physical quiescence and outer IRQ/Drop contexts remain unqualified. Other raw
receive/transport/watch interests still need their producer contracts; the whole
DNS reactor is not event-only.

## Failed runs retained separately

1. **Initial long-name control:** x86 successfully invoked the original
   `system.console.echo.primary.v1` publication but failed its 30-second local
   retirement observation at 61.127289 s (19/1/0). ARM later expired signed
   deployment readiness at 203.372619 s (21/1/0); both runners returned 1.
   Source inspection identifies the missing authority: original NS registrar is
   echo, while signed DNS is a service with no manager role. The original scalar
   completion result was not logged, so the exact returned status is not claimed.
   The fixture now explicitly delegates a separate alias to DNS rather than
   granting blanket manager authority. Third-party dead-binding cleanup stays open.
2. **Overlong manifest key:** the new echo path mistakenly evaluated a key above
   the eight-byte launch ABI limit. ARM logged `manifest keys are limited to
   eight bytes` at 8.456457 s and failed service-client exit at 18.545340 s
   (16 passed, 1 failed, 5 pending). x86 was stopped when the ARM listener was
   absent at peer boot publication; no connected guest result was obtained.
3. **Same-artifact repeat before diagnosis:** the same echo assertion recurred
   at 7.705868 s; ARM summary at 17.792562 s was again 16/1/5, and the peer
   listener check stopped x86. This was the same implementation mistake, not
   unexplained scheduling intermittence. Shared compile-time `echo_config::CLUSTER_NAME`
   now uses seven-byte `cluster`, so a future overlong constant fails compilation.
4. **Premature launch during signing:** ARM loader rejected
   `SignatureVerificationFailed` before tests. The staged store included unsigned
   late images (the extracted ARM DNS image independently rejects as `Unsigned`);
   x86 also ran a stale echo and reproduced the key assertion at
   27.417308 s, with service-client summary 14/1/5 at 37.661170 s. This was an
   orchestration failure: build sessions had not finished before copying inputs.
   An ARM signer was then found stalled before main: a one-second stack sample
   showed only `_dyld_start`, with no application stack. The task-owned process
   was stopped; an outside-sandbox rebuild/sign completed. That observation does
   not prove the host stall's cause. Final validation waits for script completion
   and independently verifies current signatures/complete store bytes.
5. **Fully signed verified pair, historical failure reproduced on x86:** at
   21.321480 s the IPC fixture reported `tid=37 generation=410 returned=1 stage=4
   source_closed=true source_registered=0 live=None staged=Some(root=None,
   lp=Some(2), started=false, error=None)`, then asserted `exact-generation IPC
   close worker exit did not complete`. Raft persistence failed at 28.191274 s.
   ARM later expired membership at 131.972258 s (20/1/1); x86 produced no
   authoritative complete summary. Both runners returned 1. This does not
   establish a causal connection between exit notification and persistence.
6. **Final proof pair, historical failure reproduced on ARM:** at 13.952917 s
   the same IPC assertion had `tid=30 generation=438 returned=1 stage=4
   source_closed=true source_registered=0 live=Some(((Running, Some(2)), Some(2),
   Some(2))) staged=None`. The new DNS controls and rollout then succeeded as
   described above. x86 completed all 64 scheduled close/exit episodes and the
   whole suite; ARM never produced the complete summary. An unchanged-artifact
   successful DNS path does not close either IPC failure or authorize stack/root
   cleanup from a worker-return marker, wake or table absence.

The original five-second exit budgets, four LPs, eight pressure rounds and exact
quiescence requirement were not weakened. This new reproduction is linked from
[the IPC investigation](2026-10-14-ipc-close-exit-investigation.md). It is the
next investigation priority; no speculative scheduler/physical cleanup fix is
included here. All failed execution attempts in this batch are represented above.

## Artifact identities

The strict read-only object-store v4 inspector verified FNV checksums and complete
final NS/DNS/echo bytes against the signed staged ELFs. Both final runs used the
same six signed artifacts. Digests below are SHA-256; original captures/stores
are local ignored artifacts, so relevant diagnostics and identities are inline
rather than dependent on temporary log links.

| Final signed artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| arm ns | 160216 | `ddee7020a232e254c004cd66ed9c10d92cd9751ed3a1876727d3703ba8bf77ff` |
| arm dns | 920176 | `068f75b80e9431bcb608771bc7707d8bfc749243c0fc539257a376087929fc02` |
| arm echo | 96160 | `6d63c48da9569893cf3116ca86270ff57d774c6a57d707a1ae4dc34060b48358` |
| x86 ns | 105040 | `e337d0c6ce7f0768d00c4058fb4b01e4dadc9a6070ac3fc619c9a13aa60a93cf` |
| x86 dns | 942504 | `072ee66660b3082b18a1142fe34cdee1422b9effaea0576b7d40883205de8531` |
| x86 echo | 32256 | `2e8bd4f592cb9aee3865e64b19a6e93b776d7cc972e1550c289a4ef91ee9e665` |

| Kernel (boot-time captured ELF) | SHA-256 |
| --- | --- |
| Final ARM, verified/proof runs | `083b987268a60a1caf180453d7b14d9d68add172e199ce0c1d55ce2a5c0671dc` |
| Final x86, verified/proof runs | `6dcc3e71d7d81d46615fa27ff7c29403a3d69c065a268d31abd41276b0f43fed` |
| Premature-signing ARM | `baac9e64b9a0a9f1699f80285d7bd8bb42844d70c46ee07b99b1f26ca628cad6` |
| Premature-signing x86 | `3194b51eac7be0233b4529d2af9190a177352f069e07297ea941cd374fa9033e` |
| Initial ARM | `934db2d0da098909839a1feced6cbf265da2230729de0de6ef11f5432e72eb10` |
| Initial x86 | `da404021f2851e95124eddd65587cc4e1332f2e5ea99b6925a304fb08f703ea2` |
| Overlong-key ARM, both attempts | `5067250b29f8448a6bc084214f322dc92ded4ba49a41bffbe97fbd2038ed7946` |
| Overlong-key x86 built input, both attempts | `fa422320b2db5e9c0faf61c6561944bce2b7c10e7cf8a95f2274b3dfbec968d6` |

| Failed input artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| ns-generation arm ns | 160216 | `ddee7020a232e254c004cd66ed9c10d92cd9751ed3a1876727d3703ba8bf77ff` |
| ns-generation arm dns | 920176 | `b2b9181a06d059530b068c401430865e94cc85783788df5e72fa6ef130be919f` |
| ns-generation arm echo | 90688 | `8dad3121c5942865e5ae3e61fb4730fda911a3046fc72e25d04529fd8eead2ca` |
| ns-generation x86 ns | 105040 | `e337d0c6ce7f0768d00c4058fb4b01e4dadc9a6070ac3fc619c9a13aa60a93cf` |
| ns-generation x86 dns | 942504 | `5e8fefb89e165ac1ceaf010aff384732b7e1cf2b9ff8004ab8de8f8c74fb1e3a` |
| ns-generation x86 echo | 26808 | `7a497c6b33d40b6af5e1af64b1e1a9ab8a3636e60a25583f15e547dd6b649ed9` |
| ns-generation-final arm dns | 920176 | `068f75b80e9431bcb608771bc7707d8bfc749243c0fc539257a376087929fc02` |
| ns-generation-final arm echo | 84752 | `ef356049b3c6ed230eea9117ab2f78f9f94aad763d96f757bf0177ba941098f2` |
| ns-generation-final x86 dns | 942504 | `072ee66660b3082b18a1142fe34cdee1422b9effaea0576b7d40883205de8531` |
| ns-generation-final x86 echo | 21496 | `2e3e44def54d77cc2e41fa29066460b6c93683a8369eca746844322b1adc0a66` |
| ns-generation-corrected arm dns | 919288 | `11efa26e00e3150e62b346615db9ffa6a35a9b1d3e5ed77cd20cdf97348f3d9c` |
| ns-generation-corrected arm echo | 96160 | `6d63c48da9569893cf3116ca86270ff57d774c6a57d707a1ae4dc34060b48358` |

Final manual: 1018960 bytes; SHA-256 `623ee4e30d510f7b439bcff57f7436d1646bcabd28e1cfdf7ca54d75387e2263`.
