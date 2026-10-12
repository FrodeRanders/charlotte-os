# Deployment controller RPC deadlines — 2026-10-12

This follow-up to `f290f713` addresses another source-confirmed SEC-06 ingress progress gap. It also exposes moved-call admission status and records a separate Intel block-read/readiness failure. SEC-06/07/18 remain partial.

## Bounded client and ownership

All seven deployment HTTP controls previously waited indefinitely for the cluster controller after request receipt: deployment, release, operations, shutdown, ingress-policy replacement, rollout query and ingress-policy query. They now share `clusterctl_client`, with one selected 5000 ms deadline starting before scalar/moved-call submission. Observation polls with 5 ms pauses. No accepted request is automatically replayed.

Timeout returns a distinct `clusterctl::ERR_RPC_TIMEOUT` (`-22`); the existing HTTP response path emits 503 and that numeric JSON error. The pending owner cancels during scope exit. A receiver may already have committed a mutation, so timeout/cancellation is not rollback or authority to undo it. The typed client distinguishes admission, observation and timeout; other IPC failures retain the adapter's existing wire `ERR_NOT_LEADER` mapping. The client creates no new remote authority or compatibility allocation path.

Moved submission consumes `OwnedMemory`. Rejection disposes the returned original request through its owner; acceptance transfers it once and retains the exact pending owner through result/cancellation. A received request remains with its receiver. Returned connection/memory fields remain owned while the adapter validates or rejects the response; dropping scalar notification replies disposes unexpected attachments too. Existing mapping failures still unwind their typed request/mapping owners. No new mapping or loan abstraction is introduced by this client.

The five-second policy does not bound arbitrary kernel cancellation, earlier request preparation, parser input, controller-internal waits, TCP/IP RPCs, response transmission or socket teardown. Existing `PendingCall` fallback aborts the domain if close rejects rather than ending an unsafe borrow. C08/C18 in [cleanup/recovery](../../reference/cleanup-recovery.md) record this scope without a registry, custody category, physical retry or wider guard/IRQ qualification.

## Moved-call status correction

Plain moved-call syscall **33** previously returned only a pending capability or zero; both owned connection APIs reported `CreationFailed` for every submission rejection. The new pressure fixture expected exact queue-full and exposed this remaining opaque ABI.

The dispatcher now returns `(pending capability, IPC status)` in ARM x0/x1 and x86 rax/rdi. Checked `svc4_x1` declares the secondary argument register as output and retains memory effects; the raw wrapper uses that emitter and discards only the returned status. The old ARM single-output syscall-33 arm is removed. Both owned `call_move` APIs return the exact original page and `IpcError::Status` on rejection, consuming the page only on successful publication. Borrowed/copy/vector and combined attachment variants retain their separate existing ABIs; this does not qualify all IPC error returns.

Two host ownership tests cover queue-full, endpoint-closed and resource-limit rejection through both APIs without closing the source page/connection, subsequent acceptance with no local double-close, and dropping a rejected page exactly once. They do not establish remote physical cleanup from mock cancellation.

## Actual pressure and completion evidence

Each of the two signed ARM primary probe launches exercises the production `clusterctl_client` against the existing eight-slot, never-receiving remote endpoint. Scalar observation must time out within seven outer seconds, while the client budget remains five seconds. Then the caller retains one-page owners until further memory allocation rejects:

1. Eight original calls fill the remote queue. Moving one held page through the client must reject with exact queue-full. One replacement page must allocate, and a second must reject while all other owners stay held.
2. Dropping the original calls drains that queue. Moving a held page now accepts but never receives; the five-second client timeout must cancel it within seven outer seconds. Exactly one replacement page must again allocate, with a second rejected.
3. All eight queue slots must be refillable after cancellation. A healthy TCP/IP status call through the same client returns its caller-supplied page; dropping the reply must restore exactly one allocation under the same saturated original sponsor.

Only exact queue-full **admission rejection** may retry the read-only healthy control; its two-second outer retry budget is checked before each attempt. Accepted observation retains its own five-second client deadline. Timeout, observation error and accepted result do not replay that call. These checks strengthen existing bits 512/1024; the required mask remains `0xffff`. They check one-page allocation eligibility and complete queue recovery in the exact live caller, not all metadata/aggregate charges or uncertain cleanup after remote receipt. Existing owner tests cover request mapping failure; this fixture does not inject it.

Both final launches report `controller RPC: scalar/move deadlines, eight queue slots, rejected/cancelled/returned page charges and healthy status passed` at guest 34.337438 and 56.942990 s. Publication generations advance 1 to 2 and retirement fencing passes. This executes the ARM userspace emitter and real dispatcher/transfer/cancellation paths. x86 ordinary deployment controls exercise successful moved/scalar calls through its checked emitter; forced full-queue/memory-ceiling RPC rejection is not injected by the x86 runner.

## Retained failed runs

The first ARM attempt, before the moved-call status correction, failed new check 52 at guest 28.935710 s. Its verifier panicked at `el0_security.rs:100` with `security probe failed check 52`; completion at 29.739554 s was **20 passed, 1 failed, 0 pending**, passed bitmap `0x123ffff`, failed `0x20000000`. HTTP EOF recovery and deployment status subsequently timed out; runner exit 1. Kernel SHA-256 `a6ce3c232c4dfef60593bb0be8c4b6fe5bb2f35ebd3944fda33388e922dc2f83`; runner-output hash `a4c70e62b24caaea9d3ddbc48feece94d2bb304e055d8768f56b952f81b2b018`. The old source necessarily hid moved rejection behind `CreationFailed`; its exact kernel rejection was not logged. The corrected run explicitly observes queue-full and passes. Later HTTP timeouts do not independently establish their cause.

The first **final-artifact Intel** attempt separately panicked at `supervisor.rs:637`: `local object store failed before node readiness`, observed error **209 (`0xd1`)**. The last status at 17.736503 s was **15 passed, 3 pending**, passed bitmap `0x33bff`, pending `0x1208000` (scheduler lifecycle, HTTP, DHCP); no final completion. HTTP was not host-validated; runner exit 1. Kernel SHA-256 `ad4a38fc0841c651fa920e603dfdd8e54dfff7879147f9037fd13986b7586a75`; runner-output hash `3484ebe5efe4be7cd63fe2867757155608b3f6cdde2ad69b4b1144e1a3ecec37`.

Source assigns `0xd1` when an object-store block read fails, including submission/observation rejection or a nonzero device result. The store had logged operator-scope updates at guest 15.911757/16.330579 s. Diagnostics latch errors during ordinary I/O too; these observations do not identify the read, its caller, device result, rejection phase or whether readiness is treating a recoverable serving error as a fatal startup error. Those details were not captured. The same kernel, same signed bundle and fresh-storage Intel repeat passes below. That repeat does **not** resolve this failure or the earlier historical AMD NIC exit; neither is attributed to the new deadline/ABI by these observations.

## Final validation and artifacts

Both complete signed service bundles were prepared before boot. Passing repeats are separate from the failed Intel run:

| Configuration | Passed / failed / pending | Passed bitmap | Result |
| --- | --- | --- | --- |
| Final ARM security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | Both RPC/pressure probes; node/cluster/HTTP/deployment controls; completion 57.442022 s; exit 0 |
| Final AMD HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | Deployment/node/cluster controls; completion 20.027178 s; exit 0 |
| Unchanged Intel repeat | 18 / 0 / 0 | `0x123bbff` | Same controls; completion 17.942091 s; exit 0 |
| ARM signed release/S3 | 20 / 0 / 0 | `0x303ffff` | Upload/atomic release/pull/verification/launch/exact readiness; completion 13.835527 s; exit 0 |
| ARM signed shutdown ingress | 19 / 0 / 0 | `0x103ffff` | Accepted HTTP intent, cooperative drain and PSCI power-off; completion 10.457744 s; exit 0 |

Trickled deployment rejection/recovery takes 30.08/30.16/30.18 host seconds on ARM/AMD/Intel repeat. Bounded transport-reset retries remain visible; the controls pass. Only ARM runs the older HTTP EOF/reset/idle script. No deliberately stalled production cluster controller is placed behind HTTP in these runs: timeout/cancellation is exercised through the same shared client against the remote silent endpoint, while valid HTTP controls test its normal use.

The signed release has SHA-256 `9fb624944b3b2da19ab3b2ef1710d2517bcf54961ea1fc16bd86b43ac0a2634a`. The agent verifies 94984 bytes at guest 17.856448 s, launches generation 1 at 17.877303 s and publishes exact deployment/service generation 1 at 18.535074 s. Host readiness reports one desired/ready replica and node key 1122115291. One startup connection refusal precedes acceptance. This uses one valid signed component and public test trust, not maximum-size authenticated operations or production custody.

Shutdown accepts its signed HTTP intent, acknowledges DeploymentControl at 14.189101 s and HTTP ingress at 14.217941 s, each with one acknowledged, zero unacknowledged/forced, lifecycle status 2. ObjectStore acknowledges at 14.901854 s; PSCI power-off occurs at 14.912727 s. This is ordinary cooperative shutdown, not uncertain hardware/loan cleanup injection.

Kernel SHA-256: final ARM combined `460564f7a46b432d4729cc438f3b8dadb4ca4c1a321f0db2793db8f4b6001d8c`; final AMD/Intel/repeat `ad4a38fc0841c651fa920e603dfdd8e54dfff7879147f9037fd13986b7586a75`; separate ARM signed release profile `0033a158c197b5109ed434ced7b33e77393ae40e2448344744ba2ea6a8f16d77`; separate ARM shutdown profile `ca52530689c31b0d0324ce11d7343544b284d3046cc3978025b05d294c0abe00`.

Complete sorted `filename:ELF-digest` bundle SHA-256: ARM `5eea7ed21ea49921631824fd8c5b7fb2b377858b40e652016069e52eae32b616`; x86 `b4c28f238802089d1243e576324f16f5fd4bf4755a021966dede1324289420f5`. Selected signed ELF hashes:

| Artifact | ARM | x86 |
| --- | --- | --- |
| deployd | `47cfd4a7503a605bc3c8813f64c2e19902ccf8ea5e287d3d5aefe40060d83785` | `696902bb23f1d5fb82612178ba048febf4b481c65f98ec18ce0cbe77137b642f` |
| objstore | `23beb950535c6bf63e19dd5908c96512cc82b5328a9106a42c87ea3a01406d0a` | `2782e889e006c52a07d4a4150ddbbecfdb0fd993af1faf91bd2ef581243bf95e` |
| bundled security probe | `f44841f13727c7c01725cae1dc6eac2669560877004caad6db35c1d12f187dba` | `b828cbe42caeea3e4a9906afe72673d2d037f0446db5fd77e58e997fa860f47c` |

Fresh fixture generation re-signs the ARM probe under its independent test root; the executed final fixture ELF is `36a4dabf6979a6abf087de9c2fab7694938b91e00b2603e6cdd9f7fc2253ab70`. x86 does not execute the signed security fixture here.

Host suite: **374 passed, zero failed, three ignored**. Strict service and ARM/x86 kernel Clippy, workspace formatting, diff whitespace and syscall-emitter coverage (87 calls, nine arities) pass. A development host compile rejected assertions using nonexistent `OwnedMemory::as_raw`; tests now inspect their private original capability field instead. This failed compile is separate from passing final tests. No mapping/ownership API was added for test convenience.

## Reproduction and open gates

Build complete bundles with `scripts/build-catten-services.sh --embed`, then `scripts/build-catten-services-x86_64.sh`. Use ARM `--security-test --http-test --dhcp-test --fresh-storage --timeout 180`, x86 `--http-test --dhcp-test --iommu amd` or `intel` with the same window, separate instances/forwarding ports and fresh storage. Reuse a kernel only for an identical profile/bundle. ARM `--deployment-ingress-test --fresh-storage --timeout 240` runs disposable TLS RustFS and signed release readiness; ARM `--shutdown-ingress-test --fresh-storage --timeout 180` is a separate kernel profile.

The Intel object-store/readiness failure needs a failing capture with exact read result/admission/phase and startup-versus-serving state. The historical AMD NIC startup remains unattributed. Controller-internal RPCs, dependency lookup, concurrent client fairness, sustained transport/allocation floods, aggregate principal accounting, maximum authenticated payloads and broader cleanup/IRQ contexts remain open. Ingress-side RPC deadlines and these targeted page/queue checks do not complete SEC-06/07/18.
