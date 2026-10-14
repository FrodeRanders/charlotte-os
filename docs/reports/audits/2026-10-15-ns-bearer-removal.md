# Removal of NS bearer-key compatibility

Batch S08b2c4c. Started 2026-10-14; completed 2026-10-15. Baseline `39b3e5dd`.
Scope: unsupported NS opcodes, incoming ownership and security-fixture grammar.

## Removed surface and preserved authority

Source/caller inventory found no active service callers of NS bearer-key
publication/lookup (opcodes 5/6). Their handlers, SDK exports, registration and
pending-lookup key fields, mismatch branches and `read_access_key` decoder are
removed. The old decoder treated missing/mapping-rejected key memory as zero;
there is no replacement default-to-public or compatibility branch. Other
protocols' access keys, including S3 credentials, remain distinct external inputs.

Unsupported/retired requests adopt their complete IncomingMessage once at the
receive ABI boundary before replying `ERR_BAD_OPCODE`. Rust owners dispose the
unused memory/connection/reply authority; scalar attachment cleanup ladders are
removed from that rejection path. No body is decoded or publication admitted.
Existing publication connection/watch owners, quotas and exact-generation
retirement remain. This does not qualify physical cleanup or every raw adapter.

Public bootstrap naming remains an active separate namespace, renamed Public
in the pending lookup model. Immediate and deferred public resolution deny
policy-published bindings. Public registration remains registrar-bound and
cannot replace or downgrade a policy-published entry. Authorized lookup uses the
kernel-captured principal/generation and explicit rights. No manager role,
application grant or policy fallback is added. NS interface 2, CNSGEN01,
runtime ABI 3 and persistence grammars remain unchanged; retired slots are not
reused. Packed/public naming still has callers and remains separate work.

## Corrected opt-in fixture

The security fixture still sent scalar/bare-name publication observation and
expected the old reply after the CNSGEN01 replacement. It now submits the
current exact frame before and after endpoint death, requiring scalar metadata
only. The two earlier shapes remain negative controls expecting ERR_INVALID.
Both retired bearer opcodes are submitted with copied memory and connection
attachments, requiring ERR_BAD_OPCODE, no returned authority/resources and an
unchanged, unloaned source memory object. Current public lookups 2/4/7/16 still
require ERR_ACCESS_DENIED for the already policy-published secavail binding.
The containing live kernel fixture namespace retains its original resources
until explicit teardown; these assertions supply no physical-release proof.

The x86 runner now exposes `--security-test`, using the same existing fixture
generator as ARM with its architecture-specific probe ELF, fresh independent
artifact/deployment roots and signed policies. It preserves network requirements
and rejects isolated shutdown/upgrade combinations. No production enrollment
or key material is configured; generated private key bytes are never printed
or published as evidence.

## Validation

Host suite: **479 passed, 0 failed, 4 ignored**, plus existing signing/boot/input/
object-store controls. Strict service Clippy passed on both architectures;
strict kernel Clippy passed with acpi/security_test and independent fixture
inputs on ARM/x86. Workspace rustfmt, shell syntax and diff checks passed.
Both signed service builds completed before guests; NS/DNS/Echo independently
verified as Bootstrap/Service/Service with no manager grant.

| Run | ARM | x86 |
| --- | --- | --- |
| Opt-in security, fresh v4 stores, four LPs, 300-second budgets | **19/0/0** at 55.970308 s, bitmap `0x2003ffff`; runner 0 | **17/0/0** at 61.160281 s, bitmap `0x2003bbff`; runner 0 |
| Linked discovery/DNS/deployment, fresh v4 stores, four LPs, 300-second budgets | **22/0/0** at 94.342902 s, bitmap `0xcfffff`; runner 0 | **20/0/0** at 43.771218 s, bitmap `0xcfbbff`; runner 0 |

Security controls executed before and after exact endpoint death across two
configured deployments per architecture. ARM live/absent metadata and retired
shape/bearer controls passed at 33.726156/33.744320 and 55.959076/55.965772 s;
x86 passed at 38.869251/38.879284 and 61.129785/61.146644 s. All four public lookup
forms denied the protected service without returning a connection or memory.
The complete scoped suite also exercised independent roots, grant attenuation,
raw-frame rejection and cancellation traffic. No historical failure recurred;
these observations do not retrospectively establish historical causality.

The linked coordinator used registered stdout readiness and checked TCP 19162's
actual QEMU listener (PID 41853). Both guests passed full-name DNS calls, NS-owned
third-party death retirement, old-endpoint replacement survival and three signed
architecture-pinned rollout generations. The leader's read-only NS absence
control passed at 32.202417 s. All **256** original exact IPC close/exit episodes
passed across these four guests: ARM security 8.939534 s, x86 security 13.875120 s,
ARM linked 10.664761 s, x86 linked 16.038978 s. Native runners checked assembly
permissions (1 ARM/249 x86 entries).

## Artifact identity

| Payload | SHA-256 |
| --- | --- |
| ARM NS (172,528 bytes) | `ea8a8673c76a599a6b3803494498b9f547ea33e74208bb7051967cdf9d3eceaf` |
| ARM DNS (920,464 bytes) | `3ff621b894f4887989da881f48d77f2cad74f66fe95fbc64f554b4fda8f08661` |
| ARM Echo (96,152 bytes) | `1ebc47315bf107997ab124b7361311817f0c020c45fe46e9155639b19ea89614` |
| x86 NS (120,448 bytes) | `1f5c60222efc456b3c6a04ed0d6d6c5f7d4998d19d8a934b380f325ae81e977f` |
| x86 DNS (942,512 bytes) | `105c92616df3f0488341a353e30b582d06d8521d018be6b2525e31c6e1d0db34` |
| x86 Echo (32,264 bytes) | `f59b140ec362c2570033dcbf70b9085bb7e7416d4f98d36e3a5e47d1c336b816` |
| ARM security kernel | `d788d314246ea13a70cb8f1ff2f1ef74a2b29cb6d903fd88a5060dd3ec5c8947` |
| x86 security kernel | `57767475639c0651976e0f71394e92744de57665294d12c30fe31fca281f32d9` |
| ARM linked kernel | `1c9e3d002bccc2125027fe133d9359b28f4ee980a5b4934f4b9085ed85468291` |
| x86 linked kernel | `255b66da560c455cb363486a6cf7816546a87f77d17843b4316ef201c057076a` |

Independent test artifact/deployment **public-key file** SHA-256 identities:
ARM `54c86df27304ce933a5b96e2335419a0c81e23eae7c18d81e20cde0e270a4aef` /
`059ccdc3ac537ceb60a751589b33b0ac907f0c93c7cb37abff3238c4a56afdc6`;
x86 `8c91dfbefb786250a5e92843e4e469421c61c104cc138f24d279bfeb2d2fd7e1` /
`5feead848c32d5cbf8ec666e477bf44d0a25fea75ba8b7610b5175194d33c86d`.

| Independent fixture | Probe ELF | Admitted descriptor | Alternate descriptor |
| --- | --- | --- | --- |
| ARM | `738830d323e66ff8b9d20123979d81f6c4f140cb1aeb615dbbce2333c82e8c29` | `85398edbdba243463e0a6d438de1754fad76b698343a0059f02c077d224f1e96` | `34ddf6efc94983bcd09d351706157644fbe36e79790dff87c31113bcd196e47c` |
| x86 | `adcafea880d23696c9737bd7eaad967aa9ba342337e3af29b3ae18cc5ab53d06` | `46d1af731fe9fad26c7626ce531e62f11ec1d8db0ccfd90d4fa067319b8c1476` | `5e8c03ddd495a1dd498be58dd1371af9e8e4c67231e93343ea3fac9b9e648dad` |

Published evidence is inline or version-controlled, not temporary log paths or
ignored artifacts. Historical reports are preserved. Markdown architecture/
guide/ledger/C18 and the LaTeX programming chapter were updated together. The
214-page manual rebuilt through two successful passes; pages 174/175 were
rendered and visually inspected. PDF SHA-256:
`12845f9a91bd2e2cc47253c9fe17a3263b2a8da1f39a8249c7666d6497a60d16`.

## Remaining work

NS OP_STATUS still clamps a 256-byte name to 255 and emits a page-limited prefix
while its header retains the registry count. This is an observed format gap,
recorded in the simplification ledger for a coherent tagged/bounded replacement
with exact counts, complete names and owning request/reply handling. Remaining
packed/public formats, raw adapters, metadata/outer IRQ/Drop qualification and
physical cleanup remain separate; historical IPC/persistence attribution stays
open. No new HTTP/S3/Kafka, hardware-platform or production trust qualification
is claimed by this batch.
