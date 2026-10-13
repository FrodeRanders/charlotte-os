# One scoped launch owner and an explicit retirement principal

S04b1 of the [simplification sweep](../../reference/api-simplification.md), based
on `69b3f910`. This removes duplicate scoped launch APIs and the unused singleton
retirement selector. Descriptor-less deployment/fixture callers remain the next
ordered removal; this batch does not claim all S04 work is complete.

## Current boundary

`launch_scoped_artifact` consumes the ELF and descriptor `OwnedMemory` values
with their exact lengths and returns `DeployedArtifact`. The kernel returns the
same signed-name principal that it installs in the deployment registry. The
runtime no longer derives retirement identity from a separate caller-supplied
name. The ASID-only packed scoped launch, its named wrapper and the separate
named owning wrapper are removed; the agent uses the one owning API.

Syscall 76 now takes `(elf_cap, elf_len, descriptor_cap, descriptor_len)` and
returns `(asid, verified_principal)`. Every rejection clears both results.
Both submitted inputs remain consumed on all outcomes; duplicate handles
reject with one close instead of duplicate release. Descriptor/ELF verification,
exact name binding, admission, launch ownership and duplicate-principal rejection
remain in the existing kernel gate. The signed descriptor bytes and version
are unchanged. See the [register contract](../../reference/scoped-deployment-launch.md).

ARM uses the four-argument/two-result emitter, including memory effects. The
five-argument scalar emitter previously used by this wrapper is now uncalled
and removed on ARM, x86 and fallback targets, including its stale opcode arm.
The generic x86 emitter returns the second register in the same tuple. All
active service callers are rebuilt; no old-arity adapter remains.

The unused `retire_artifact()` wrapper and kernel singleton branch are removed.
Zero principal returns `u64::MAX` before registry acquisition or mutation;
cooperative, force and node-shutdown requests retain their existing explicit
principal, deadline and reason behavior. Absent nonzero principals retain the
owner-completion contract. The complete cleanup/root owners, charges and
physical quiescence policy are unchanged; C18 evidence records that scope.

Contributor instructions, ownership guidance, the current reference and the
Markdown/LaTeX/PDF documentation describe the contract. The remaining legacy
path is called out explicitly rather than described as a second scoped API.

## Regression scope

The runtime tests now capture the exact four submitted arguments. They verify
local length rejection closes both memory owners without submission, successful
submission moves both inputs and retains an arbitrary kernel-returned principal,
completed retirement uses that principal without a later force fallback,
kernel rejection consumes the two inputs without local re-close, and Drop
forces only the returned principal. Existing Pending/error retirement, shutdown
deadline, memory mapping and cancellation tests remain enabled. The owning API
does not map either input, so there is no new runtime mapping-failure branch.

One added host test covers successful-launch Drop. The existing native C18
single-entry registry fixture now submits zero with force requested, requires
rejection, and verifies the exact entry/principal/claim and force state are
unchanged before continuing the named pending/error retention test. This is
the existing owner-family fixture, not a new registry category.

## Host and build validation

`run-host-tests.sh` passes **411 / 0 / 4 ignored**, including signing policy,
boot-result, syscall-emitter and signed-envelope tool checks. Both service
bundles build and strict service Clippy passes with `-D warnings`. The initial
post-removal host build identified the now-unused scalar `svc5` helper; all
three target implementations were removed before final validation. Two initial
kernel lint invocations omitted `CATTEN_S3_TEST_CA_DER` and failed at the
fixture's compile-time certificate include, not at changed Rust logic; invocations
with the runner-provided certificate and bundle environment passed.

Signed bundle digest is SHA-256 over sorted newline-delimited
`basename:ELF-SHA256` entries:

- ARM, 41 ELFs: `e3d6fa51d102ef509c0146acfa21da6afa5ea1634aae471126996ec968146a2e`.
- x86, 37 ELFs: `ae0a3fd34592f1c706469b45034f97362e6aa81f87cc8e640d34decd4234c778`.

The initial ARM signed-ingress sample passed 20/0/0 at 14.167736 s and the
host worker confirmed signed upload, atomic release, pull, launch and readiness.
The agent logged `greet` generation 1, ASID 142 and returned principal
`0xa31bf89d72682689` at 19.065239 s. Kernel SHA-256:
`cffeadd9be7da923b49fa4062545c0c3c5f37434ba8c12b847c54d8977500816`.
Descriptor/release hashes were
`9e78656a9a049aac2471708d63c5d5e68a04b5ceba4f46e5f1484313ee148699`
and `bc23e7699939b49d6f58e42ea25ca52ddd4de8ccd6143452efd9b12dbb00e365`.
This sample precedes the timer-fixture correction below.

The final x86 signed-ingress run passed 18/0/0 at 19.203675 s. The host worker
confirmed the same complete signed path; the agent logged generation 1,
ASID 132 and principal `0xa31bf89d72682689` at 21.319855 s. The corrected
pre-park rejection, sleep fallback and watchdog-accounting fixture completed.
Kernel SHA-256:
`136e4632d45f913b7b4b49a22dfcca9a0f857a9a82ead4045f863738bde9e37e`.
Descriptor/release hashes:
`767bbfaf6bc27bc893176e51dd17cdd6e678865f0581350d9ec1334e9c6b7e32`
and `c3ba5644c49c31598edb927b91f2227484ee00545a6bb7efc0b94c21451a2024`.
The logged principal matches the signed `greet` name's SHA-256-derived identity
and exercises the second syscall return register through the real runtime owner.
The C18 zero-selector assertion executes in the ordinary boot-suite probes.

The final ARM repeat with the isolated worker passed 20/0/0 at 13.784874 s
and the full host signed-ingress check. The agent logged generation 1,
ASID 140 and principal `0xa31bf89d72682689` at 17.713010 s. Kernel SHA-256:
`4073d5e379fdb9aa7772cc84d45a362926b09b858ac8791c62287ea98362685b`.
Descriptor/release hashes:
`82eda6dcc20851d2a14ec113c3176e1123b0cb3b0b89e65aa371477d4176029e`
and `1b8fccb80ef89449fe55232af1394be541c3bec19a63ec4bfd914840d955e080`.

Both final runs use fresh NVMe fixture disks, debug kernels and the normal
`--deployment-ingress-test --timeout 240` runners, with signed bundles already
rebuilt (`CATTEN_SKIP_EMBED_BUILD=1`). The kernel profile is
`acpi,virtio_net_test,dhcp_test,s3_test`. Each native runner checks executable,
read-only assembly sections and captures an immutable kernel hash. The local
RustFS TLS development fixture is regenerated separately for each run, so its
certificate and timestamp-bound envelopes differ. These are public development
trust fixtures, not production enrollment or recipient custody.

Workspace formatting and diff checks pass; 614 local Markdown links resolve.
The rebuilt manual remains 208 pages. Page 134 was rendered and visually
checked; PDF SHA-256:
`1cfc19ddcd7958aa5c43095e32bac13dd84ca3db9d46e63604c2442808ae3358`.

## Initial x86 fixture failure and correction

The initial x86 signed-ingress guest panicked at
`timers/event_tests.rs:275:5`: the completion wait returned status 1
(`PENDING_OR_TIMEOUT`) while the fixture expected 3 (`WAIT_ADMISSION_FAILED`).
The fixture used a 1 ms deadline. Its last published self-test state at
13.686362 s was 12 passed / 6 pending; no authoritative complete record followed.
The runner eventually failed signed release response/readiness with OS error 35
(`Resource temporarily unavailable`) and timeout. Kernel SHA-256:
`c9e223bfbdb4409f14014f85f974ef1f82bd32df93a767e886faeb2a5e207026`.

The checked wait explicitly returns timeout when its monotonic budget expires
before watchdog admission. The observed status is therefore a valid outcome
for that short-budget setup; the run did not capture the exact preemption or
clock-read interval. It does not establish a production admission leak or a
launch-ABI failure. The earlier ARM run passed 20/0/0 and signed launch with
that same fixture source; its result remains a separate sample below.

The admission probe now uses a 60-second budget and a short IRQ-state-preserving
hold around the full-account, pre-allocation rejection. Timeout semantics and
production waits are unchanged. It also removes the old live-thread sponsor
replacement/destructor: the event fixture runs on a dedicated raw kernel
worker whose private trusted timer sponsor is installed before publication or
any event capture. The parent registers exact-generation thread-exit observation
before dispatch and waits on that existing completion source, then explicitly
closes the observation. No polling flag or new readiness registry is added.
The exit observation proves fixture completion, not stack/root quiescence.
C16 retains the existing admission, node ceilings and cleanup qualification.

## Limits and next removal

This is a typed logical launch receipt, not proof of physical cleanup, hardware
custody or production enrollment. Existing deployment/root retirement claims,
generation retention and uncertain-cleanup rules still apply. Published
deployment registry allocation and containing Drop/guard contexts remain their
existing qualification work; no numbered security finding is closed here.

Descriptor-less syscall 64, local artifact staging/upload, packed catalog
deploy/query operations, descriptor-less command/snapshot records and diagnostic
primary fields still need coordinated signed caller/fixture replacement. The
encrypted operational pickup remains a distinct authenticated protocol, not
an old scoped-launch overload. No production-store migration is needed now;
future persisted-record evolution still requires the documented explicit,
interruption-tested migration.
