# Network endpoint publication under admission pressure

This continues `c030063f` and the [deployment ingress report](2026-10-11-security-deployment-ingress.md). SEC-06/07 remain partial. The historical AMD startup failure is still unattributed; the correction below addresses a separate source-confirmed admission defect and adds diagnostics for a future failure.

## Retained failure and diagnostic repeats

The failed AMD HTTP/DHCP run used kernel SHA-256 `960a7fa802771b3478d1fa8b89eb2ec117746ce15fb965a1672fa28b07e6771e` and runner-output hash `0ea150d99d8c701ab852436550aa7c69a66e06e6134d70eb77057846fbead404`. The NIC driver ASID 106/thread 14 exited at guest 11.581 s. At 12.051 s the frame-router verifier observed ASID 107 at stage 1, waiting for its mandatory NIC lookup. At 42.121 s it timed out: 15 passed, 1 failed, 2 pending; passed bitmap `0x1bbff`, failed `0x20000`, pending `0x1200000`. HTTP never validated and the runner exited 1. The NIC's exact startup stage/rejection was not captured. Frame-router timeout is a downstream symptom; queue-full registration, DMA preparation and feature negotiation are not established historical causes.

NIC startup now records its first rejection in `net_status::STARTUP_ERROR` (offset 64), publishes its u64 detail (offset 72) first, and logs stage/error/detail before terminal exit. Categories distinguish missing launch inputs, MMIO mapping, feature negotiation, queue size, memory/CPU/DMA preparation, endpoint creation, registration admission/observation and CQ binding. Diagnostics do not restart hardware or change physical cleanup policy.

Three diagnostic-only AMD repeats, with unchanged 180 s runner/probe limits and fresh storage, pass 18/0/0 (`0x123bbff`) and the complete deployment/node/cluster host suite. They use identical kernel `0d0bf72c39e0d4091065a3f1d93faf5b983cd87d911b9c13be2cc09f23ecb8bf` and signed NIC ELF `2145f65bdfeaf2e6d5101d3b5c68dd6235e08993d596e673bd1688ff9d704082`. Completion occurs at guest 18.421, 25.652 and 25.726 s. None executes a startup-rejection branch. Passing diagnostic repeats do not resolve the earlier failure.

## Source correction and ownership

NIC and frame-router endpoint registration previously terminated on any delegated-call submission rejection. The delegated-call ABI returned only a zero pending handle, hiding whether admission hit a full endpoint queue. This is a source-confirmed availability defect, independent of attribution of the historical AMD exit.

Plain delegated call syscall 39 now returns `(pending capability, IPC status)` in the two standard return registers: ARM x0/x1 and x86 rax/rdi. Raw callers discard status through the same checked emitter, declaring the secondary register output and memory effects (IPC may publish mapped CQ state). Both owned connection APIs expose exact `IpcError::Status`. Combined delegated/copied-memory calls retain their separate existing ABI; this correction does not qualify all attachment variants.

`publication::register` borrows the same endpoint and naming connection throughout submission and observation. It retries only exact `QUEUE_FULL`, with 10 ms pauses and a selected 1000 ms admission budget. Other rejection or exhaustion returns an error. Accepted calls are observed once, without replay; their observation remains a separate, unbounded progress boundary. Rejected publication leaves the endpoint owned. Returned connection/memory on an invalid registration reply is rejected and released through its Rust owners. A single first-retry trace identifies the retained source without logging every polling attempt.

The NIC now retains a typed `Endpoint`, borrows launch naming authority through `Context`, and uses the shared helper; the frame router uses that helper with its existing owner. Hardware initialization, DMA queues/backing and their terminal policy are not retried or reconstructed. The existing frame/DMA protocol adapter only borrows the endpoint for receive.

`Endpoint::close(self)` consumes only confirmed close; rejection returns the exact owner. During NIC shutdown, any rejected/uncertain endpoint is retained in the existing terminal park, alongside the existing controller domain/backing. It is not retried or dropped there. Confirmed endpoint close precedes transmit drain/reset and IRQ close. This extends C09/C18's existing owners without a registry, physical retry adapter, new pool or expanded destructor/context qualification. Other driver ring/frame owners and broader IRQ/allocator contexts remain separate work.

## Regression and execution evidence

Host ownership checks cover exact queue-full/closed/resource-limit rejection through both connection APIs without closing either source, accepted retry and pending-call/source cleanup exactly once. Endpoint tests cover confirmed close without a second Drop close and an explicitly unstarted mock rejection retaining its owner. The production NIC parks on all close errors; the mock retry is not proof of safe physical retry. Pure submission-policy checks cover two rejected attempts before one acceptance, deadline exhaustion and permanent rejection without retry. No new mapping is needed for port/name-only publication; guest physical allocation rejection and uncertain hardware close are not injected here.

The real kernel syscall fixture fills an existing four-slot endpoint, verifies delegated rejection returns zero pending capability plus exact `QUEUE_FULL`, drains only the original messages with no extra grant, retries through the same original sources, receives one delegated grant, replies once, observes the result and closes both transient capabilities. This checks the dispatcher/transaction path; real userspace NIC/router boot also exercises the architecture emitter.

Before final emitter memory-annotation tightening, ARM security/HTTP/DHCP passes 21/0/0 (`0x2123ffff`), AMD and Intel HTTP/DHCP each pass 18/0/0 (`0x123bbff`), and signed ARM shutdown passes 19/0/0 (`0x103ffff`) and reaches PSCI power-off. All runners exit 0. Existing HTTP/deployment recovery and node/cluster JSON controls pass within their selected limits; trickled deployment rejection occurs at 30.18, 30.14 and 30.23 host seconds respectively. The older HTTP EOF/reset/idle script runs only on ARM, not the x86 runner.

The signed shutdown capture observes FrameRouter acknowledgement (one acknowledged, zero unacknowledged/forced, lifecycle status 2) at guest 14.441 s, ObjectStore acknowledgement at 14.448 s, NIC driver exit at 14.455 s and power-off at 14.461 s. This is ordinary idle cooperative shutdown, not an injected endpoint/physical cleanup rejection or outstanding-I/O/physical-NIC proof.

The memory annotation was then made conservative because a delegated call can publish mapped CQ state. Rebuilt x86 bundle SHA-256 remains byte-identical (`2b72cf208aafca829c7bdee87bb68e26b3e2740cfa29c4288f57ccea3be24ece`); the recorded AMD/Intel executions therefore validate the same final x86 service artifacts. ARM's complete bundle changes from `3ba2cd36bae8f25c8d18ecdb3c03fbc1d38aa20406f6bd82b751ab85e65dcd32` to `4749835b1a0991d5ef41c6164ed35f091701a04ee43c13a8c5a772c7b7b9cd8c` and is revalidated separately below.

Final artifact validation:

| Run | Passed / failed / pending | Bitmap | Host/terminal result | Runner |
| --- | --- | --- | --- | --- |
| Final ARM security/HTTP/DHCP | 21 / 0 / 0 | `0x2123ffff` | Full existing HTTP/deployment recovery, node/cluster JSON; trickled body 30.12 s | exit 0 |
| AMD HTTP/DHCP, unchanged final x86 bundle | 18 / 0 / 0 | `0x123bbff` | Deployment/node/cluster controls; trickled body 30.14 s | exit 0 |
| Same x86 kernel, Intel HTTP/DHCP | 18 / 0 / 0 | `0x123bbff` | Deployment/node/cluster controls; trickled body 30.23 s | exit 0 |
| Final ARM signed shutdown ingress | 19 / 0 / 0 | `0x103ffff` | Cooperative phases and PSCI power-off | exit 0 |

The final ARM combined kernel is `1dea70e2d3d66c50050f4b7159bf2e97c1a079ecf8cdb7f8a8c40ad92bc55529` (completion at guest 23.387 s); x86 AMD/Intel share `5bbdce247b4f1c3566370240e6b555746ed38c7ffeaa85a3ce3d3f045d9e8bfa` (19.779/18.267 s). Final ARM shutdown has its separate profile/kernel `966edb8e782fa39f69114bf625912f7f3017e205e31c2dc8f6fc1bf3ac388d82` (kernel completion 10.166 s). FrameRouter/ObjectStore acknowledge at 13.911/13.922 s, NIC exits at 13.934 s and power-off occurs at 13.935 s. No hardware-quiescence rejection appears in that idle shutdown. The rejected/uncertain endpoint terminal-retention branch remains source/host-qualified, not injected in this guest.

The forced delegated queue-full fixture reports success at guest 5.586 s on final ARM and 12.884 s on AMD. Ordinary boots do not emit a publication retry or startup rejection in these captures; the actual helper pause is tested through the controlled host submission schedule, while guest rejection uses the real dispatcher/transaction fixture. These are distinct evidence boundaries.

Final signed service ELF SHA-256:

| Artifact | ARM | x86 |
| --- | --- | --- |
| net | `d06c5081b06dd10a9bc79d72c3be5b25febcae437238c54a1a4d03679e9c5f7c` | `69d6cad9c0c55adaa3d78e26dfa243d975870731993ca178a8701f8f5f4cdc2b` |
| frouter | `c36b0acd3e31c7af37d4ead7c8bcf6c2bffad2e703d659eb1e12888895f7572f` | `5ac73a78ff80bb145eefa2016d2a7a4296e330901317ff59e33ec14627f6b97d` |

Final host suite: **372 passed, zero failed, three ignored**. Strict service and ARM/x86 kernel Clippy pass. Workspace formatting, diff whitespace and emitter coverage (87 call sites, eight emitter arities) pass. Development checks separately rejected missing unsafe status-read blocks, a stale reactor status import and unused router rights import; these were corrected. The emitter checker also caught an initially wrong delegated syscall number before guest validation; the declared syscall 39 is used in the final checked emitter. No failed development attempt is classified as a passing boot or evidence of historical cause.

## Reproduction and remaining acceptance

Build complete signed bundles with `scripts/build-catten-services.sh --embed` and `scripts/build-catten-services-x86_64.sh` before boot. Run ARM with `--security-test --http-test --dhcp-test --fresh-storage --timeout 180`; run x86 with `--http-test --dhcp-test --iommu amd` or `intel`, separate instance/host ports, fresh storage and the same 180 s window. Reuse a kernel only for an identical profile and bundle. ARM `--shutdown-ingress-test --fresh-storage --timeout 180` independently exercises signed shutdown and ordinary idle device quiescence.

The historical AMD startup cause remains open until a failing diagnostic capture identifies its actual branch. Three passing repeats and the admission correction cannot establish that cause. Accepted registration/dependency RPC progress, sustained packet/connection floods, concurrent healthy-client fairness, failure/abandonment during real device shutdown, physical NICs, aggregate resource pressure and wider cleanup/context gates remain unresolved SEC-06/07/18 work.
