# HTTP listener progress under security queue pressure

This follows `a73e7106` and the [caller-buffer report](2026-10-10-security-tcpip-caller-buffers.md). It investigates the combined security/HTTP failure retained there. SEC-06 remains partial: this is one concrete listener-admission defect, not general management flood fairness or aggregate admission qualification.

## Failure evidence and diagnosis

Two unchanged-deadline reproductions separately failed the host keyhole check even though all 21 kernel checks passed (`passed_bitmap=0x2123ffff`, zero failed/pending). Each host probe exhausted its three 12-second request attempts. Later HTTP recovery does not turn these runs into passes.

| Capture | Kernel SHA-256 | Packet capture SHA-256 | Result |
| --- | --- | --- | --- |
| Unmodified `a73e7106`, packet trace | `562e273095cbe92d3497e03f427344d1a5dd723a785212094866c07014338770` | `280ce4076440dd85566238139522e34f19226fcf9da10dce2c592d11d35ee4c0` | Kernel 21/0/0; host failed; runner exit 1 |
| Opt-in HTTP phase trace, before fix | `0b7c16007099145f8d7456339d8984dd89a5ee267e9dc1619ba59994b83eccac` | `4575bdbcd4b8e21dc9d849ad1ff981e417cc452862e1b46e5148eea80fdf1a34` | Kernel 21/0/0; host failed; runner exit 1 |

The retained inline observations below are the evidence; capture hashes identify the original local artifacts, which are not version-controlled dependencies of this report.

The unmodified packet trace recorded these Unix timestamps:

| Event | Timestamp | Observation |
| --- | --- | --- |
| First host SYN | 1791661523.450894 | Initially unanswered; frame-router diagnostics separately observed one dropped frame |
| Retried SYN | 1791661529.428290 | Guest SYN/ACK at 1529.522517 |
| First GET `/metrics` | 1791661529.522583 | 61 request bytes; guest ACK at 1529.683892 |
| First host FIN | 1791661535.452266 | Request deadline expired with no HTTP payload |
| Second GET | 1791661538.521870 | Guest ACK at 1538.629518; host FIN at 1550.456655 |
| Third GET | 1791661553.470811 | Guest ACK at 1553.628374; host FIN at 1565.458748 |

The guest therefore received and acknowledged application bytes during the stall. The initial dropped SYN cannot explain the subsequent absence of HTTP responses.

The pre-fix phase trace repeatedly entered listener construction from guest time 9.971 to 11.125 seconds. After the final `accept` marker at 11.125, its first `receive` marker appeared only at 72.537. A later metrics request completed its status sections quickly: observe 74.823, TCP/IP 74.829, name service 74.832, frame router 74.835, DNS/discovery 74.854, reliable messages 74.857, history 74.874, send 74.878. Blocking metrics RPCs and the coincident time-service retry are not established causes.

Source inspection found that failed `OP_ACCEPT` submission abandoned the active `OwnedSocket`. Its best-effort close could encounter the same full endpoint queue and lose the remote ID. The replacement listener then waited while an earlier live listener accepted and buffered host requests. TCP/IP retains activated sockets belonging to a live domain; its terminal TCP grace period is 60 seconds. That policy and the phase trace explain the observed late recovery, but the failed captures alone do not identify the exact submission error. Post-fix execution below establishes actual `QUEUE_FULL` rejection while retaining the listener.

## Correction and ownership

Scalar call submission now returns its admission status alongside the pending-call capability. `ConnectionRef::call` and `Connection::call` expose rejection through `IpcError::Status`, preserving the connection and creating no pending owner. ARM x1 and x86 rdi are declared outputs; raw adapters discard the status through the same checked emitter rather than assuming the input register survives the syscall. The emitter checker now covers secondary-return emitters and assembly arms with setup instructions.

HTTP retries rejected `QUEUE_FULL` acceptance after its existing polling interval while retaining the same `OwnedSocket`. It checks lifecycle shutdown on every iteration. Permanent admission errors still leave the connection loop. This long-lived listener wait is distinct from request receive/send deadlines, which are unchanged.

`OwnedSocket` close retries only queue-full admission rejection, with one total 1000 ms deadline and 5 ms pauses. No accepted close is resubmitted by that admission loop. The existing consuming-close Drop fallback has its own bounded attempt, so rejected explicit close may spend another 1000 ms in fallback. The exact remote owner remains in the containing socket until submission. Existing explicit close error reporting and best-effort Drop fallback remain; this is not a guarantee of remote release under indefinitely saturated admission. No new cleanup family or registry is added. C18 evidence is extended for the existing userspace protocol owner, without qualifying outer kernel guards or physical teardown.

The launch manifest of the signed HTTP service enables phase/error tracing only for `http_net_test`; ordinary launches disable it. Tracing exposes admission status and the listener ID so the pressure regression is observable without changing queue capacity or host deadlines.

## Execution evidence

A development host regression checks queue-full, closed-endpoint and resource-limit rejection through both owned connection APIs. Rejection closes no capability; an accepted retry through that same connection produces exactly one pending owner, and dropping the pending call, connection and endpoint closes each exactly once.

Before the final raw-adapter correction, two combined runs passed all 21 kernel checks and the complete host EOF, abortive-close, idle-timeout, node-JSON and cluster-JSON checks. Both used kernel SHA-256 `ee18ae2eb15d3f0c227850f3c5d3d429c845dfe7ac0d79b4ceaf5bcc3b9cbedf` and signed HTTP ELF `91099c2576eec719c7a6e2b3fd8c92a43f6e6a0e129eb8e8862cad2e23d4f5ed`.

- First run: five `accept admission=Status(1) socket=2` markers between 20.316 and 20.872 seconds, preserving socket 2; kernel completion at 33.950. Initial metrics required one existing timeout retry. Packet-capture SHA-256 `7355f3cf60bf9dbb4c7bf4f224d5595cd13d75d59b86b74c1056beb420733990`.
- Repeat: eight queue-full markers retained socket 2 between 13.505 and 14.493 seconds; kernel completion at 27.787; initial metrics passed without a timeout retry. Packet-capture SHA-256 `a598abfe93ace4d01e2e814b573259823d5fa8b1efe53681aab9d1f9ea9e99e4`.
- Both retained the existing connection-reset retries in later peer-close/idle checks. Passing means the documented runner limits were met, not that all individual transport attempts succeeded.

An earlier invalid build attempt is separate: HTTP tracing initially failed compilation because its log macro lacked a namespace. A guest runner was mistakenly started before the service bundle finished rebuilding/signing. It rejected the signed identity of `rng.elf` at launch and panicked at `service/launch.rs:354`, then lacked an authoritative result at 180 seconds. Runner-output SHA-256 `30312449da5f1470ecae44f668963c8161d9e293d466b36d42ded29037e8af35`. This mixed-build attempt is not evidence for or against the HTTP fix; all qualifying runs start after complete bundle preparation. The first expanded emitter-check attempt also falsely rejected four valid reply emitters because it did not recognize their setup instruction; the parser was corrected before final validation.

Final source validation used completely rebuilt/signed ARM and x86 bundles, with explicit submission/observation separation and the corrected raw adapter:

| Run | Passed / failed / pending | Bitmap | Host result | Runner |
| --- | --- | --- | --- | --- |
| ARM combined security/HTTP/DHCP | 21 / 0 / 0 | `0x2123ffff` | Metrics, EOF, abortive close, idle timeout, node/cluster JSON all pass | exit 0 |
| Same ARM kernel, fresh-storage repeat | 21 / 0 / 0 | `0x2123ffff` | Same complete host suite passes | exit 0 |
| x86 Intel, no network | 15 / 0 / 0 | `0x1bbff` | Not an HTTP test | exit 0 |
| Same x86 kernel, AMD, fresh storage | 15 / 0 / 0 | `0x1bbff` | Not an HTTP test | exit 0 |

Both final ARM runs use kernel SHA-256 `bb04a3fd91ceec35e09b4beb775a0c59d7f12d34326036f60524f2a6ae4230f3`; both final x86 runs use `6985d609a45eae8636b67a5315e646d1e883d96dbca65879e0285e88ab8c068f`. ARM kernel checks complete at 25.076 and 23.637 seconds; Intel at 15.704 and AMD at 15.399. The first ARM run records ten queue-full accept rejections on socket 1 between 11.031 and 12.086 seconds; the repeat records eight on socket 1 between 10.133 and 11.014. Both initial metrics checks pass without a timeout retry. Later peer-close/idle checks retain their existing connection-reset retries.

Final packet-capture SHA-256 values are `3ca1c8c77824f05697217246bbe6183062b39849aab9eabdc2638400c3d8f38b` (first ARM run) and `ebd379665d3cab76974fa6463048690c948635315900e224a016896679819701` (repeat). In the first, host GET `/metrics` at Unix time 1791663778.747490 receives the first HTTP `200 OK` payload at 1791663778.915571, about 168 ms later. This is direct execution evidence that acknowledged request bytes now reach HTTP response generation.

The earlier run after raw-adapter correction, before explicit observation separation, also passes all 21 ARM checks and the complete host suite with kernel `bfe1c46a13d58f13ae66ebb703615b201ce90037c2ac9e04ffd6002e6c03d187` and packet-capture hash `71eef9a403efbb51c8ecfef8ef7bc0e50fbd3da6b0a9e7e40b5cfc46b0e34a7f`. Its first metrics check needs no timeout retry. The corresponding earlier AMD run passes 15/0/0 with kernel `f56c72eb2beab1c2b158676da36f82e3030e26b3cb87d0aecc98093d56d2fe3c`. These intermediate passes are separate from the final-artifact matrix above.

Final signed service ELF SHA-256 values:

| Bundle / service | SHA-256 |
| --- | --- |
| ARM HTTP | `45f3eccde2156bb537081fe1a4688f9bde736f61e6091a23e7517ca33843194e` |
| ARM TCP/IP | `703b6db89b7cc530e083fe4eeaeee6186e189922959233b3cb8678e0fd9bf28e` |
| x86 HTTP | `1663c25b883eefd3e8134766bd2b94669ed7b64cd941d1e5005d7387f9b63df9` |
| x86 TCP/IP | `903ff5007bdeecd3e5368321654bb7ede433773c409dc7656854edcc17aa5875` |

Host suite: 354 passed, zero failed, three ignored, including the owned scalar-admission regression and signing/boot-result checks. The emitter checker covers 87 call sites across seven emitter forms. Strict ARM service Clippy, combined-feature ARM kernel Clippy, x86 kernel Clippy, workspace formatting and whitespace checks pass. Both signed bundle builds complete before final guest launch. No host request deadline, socket capacity, pressure-client workload or kernel verifier deadline is relaxed.

## Reproduction and limits

Build the ARM bundle with `scripts/build-catten-services.sh --embed` and use dedicated host ports:

```sh
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_QEMU_NET_DUMP=1 \
CATTEN_HTTP_HOST_PORT=19085 CATTEN_DEPLOY_HOST_PORT=18385 \
scripts/run-aarch64.sh --security-test --http-test --dhcp-test \
  --instance audit-http-progress --fresh-storage --timeout 180
```

Reuse a kernel only for an identical feature profile and completely prepared bundle. The two ordinary signed pressure clients replenish the shared TCP/IP request queue; the fixture checks reactor/protocol-clock progress before retiring them. This is a scoped CALL-pressure scenario, not a physical packet flood, indefinite adversarial saturation, or NIC fault-recovery test. The original failed results remain historical evidence. Broader SEC-06 fairness and SEC-07 R7-4/R7-5 pressure/aggregate acceptance remain open.
