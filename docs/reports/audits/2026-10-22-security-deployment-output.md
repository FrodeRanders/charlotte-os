# Deployment response-send ownership

Date: 22 October 2026. Reviewed baseline: `e70439e4`.

SEC-06 remains partial. Deployment response sends now use the shared CQ instead
of waiting inside `send_all`. A pending send keeps its response, socket and
original deadline while the ingress loop can observe other inputs, controller
calls and outputs. The ledger remains 29 corrected, 6 partial and 3 open.
Production trust remains disabled.

## Output and producer contract

Listening, input, control and output phases share four positions. Each output
owns complete response backing, offset, submitted prefix length, one five-second
deadline and its pending moved page before the socket field. Owned string
capacity is limited to 64 KiB per position; caller send pages, remote retained
memory, TCP buffers and allocator overhead remain separate charges. Rendering
already occurs before this admission check and is a separate allocation boundary.
The loop visits input, control and output phases before its shared wait; each
pool uses its own rotating observation cursor. It drains CQ hints before
rechecking exact owners and waits for the earliest remaining original deadline.

Deferred TCP send is opcode 16. It takes the full socket ID and an exact
4096-byte `CSOCKTX1` moved frame: eight magic bytes, little-endian u32 length,
four zero reserved bytes, 1..=4080 payload bytes and a zero tail. There is no
packed fallback on that operation. Existing immediate TCP/UDP callers use the
distinct `OP_TRY_SEND` (6); all current callers migrate together. The NIC send
protocol is separate.

TCP/IP adopts the complete authenticated request once. It maps the moved page
read-only and retains request, mapping and enqueue state together through
handshake or send-buffer pressure. Established and CloseWait can enqueue;
terminal states, explicit close and shutdown settle it with an error. Zero
means nothing entered the TCP buffer and leaves this same request pending. A
positive prefix or backend error fences another enqueue attempt with that
request. Local mapping access ends before consuming reply authority. Producer
replies notify the pending call's bound CQ; TCP/IP still has its existing 50 ms
timed transport cadence. This is no claim that the whole stack is event-driven.

Only a checked positive scalar prefix receipt advances output and authorizes
submission of its remainder under the original deadline. Returned attachments,
zero or oversized receipts reject. Service error, observation error and timeout
end output without resending uncertain bytes. Enqueue does not prove peer
receipt. Cancellation cannot undo a prefix already enqueued or establish that
the service released its original moved-memory charge. A retained deferred
request can persist until producer completion, explicit close or owner reaping.
No second request is submitted to recover an uncertain prefix.

Before submission, allocation/map/unmap/admission failure disposes only the
unstarted owner. After acceptance, CQ binding or wait rejection aborts with the
complete containing pool live. Normal observation/cancellation disposes the
pending owner before returning its socket to the existing explicit close policy.
The same C18 family row records these contexts; there is no new registry,
cleanup custody, kernel root/pin or physical retry owner.

## Verification

The [version-controlled evidence](evidence/2026-10-22-security-deployment-output.json)
records commands, artifact hashes, results and failure diagnostics.

The host library suite passes 182 tests. Output controls cover a held operation
beside a healthy receipt, rotating partial outputs, original deadlines across
chunks and unrelated wakes, timeout without replay, invalid/service/observation
results, initial/remainder admission rejection, four-slot and retained-capacity
bounds, and pending-before-socket disposal. The binding rejection adapter checks
both owners are live at the abort boundary; host unwinding is no production
teardown proof. Codec controls reject retired, truncated, extended, reserved,
invalid-length and dirty-tail frames. Producer controls distinguish handshake,
full/available buffers and terminal states, and fence repeated positive/error
enqueues. Actual mapping or CQ-binding failure is source-reviewed, without
native fault injection.

The native scoped-security fixture runs the actual output pool with terminal
TCP and unsupported UDP sockets. Both deferred calls must reject within two
seconds, leaving no occupied output position. A separate untagged moved-page
call must reject before enqueue. Normal native HTTP and signed central-store
release runs exercise successful deferred sends, complete acknowledgements,
receipt binding, pull, verification, launch and named readiness. These normal
responses fit in a chunk: they do not establish native partial-write behavior
or progress beside a genuinely saturated TCP send buffer.

Final native results:

| Run | Self-tests | Additional result |
|---|---|---|
| AArch64 security, fresh repeat | 19 passed, 0 failed, 0 pending | Two output fixtures in separate scoped roots |
| x86 security | 17 passed, 0 failed, 0 pending | Two output fixtures in separate scoped roots |
| AArch64 HTTP | 19 passed, 0 failed, 0 pending | Healthy stalled-peer controls in 0.241–0.432 s; trickled body ends in 30.19 s |
| x86 HTTP | 17 passed, 0 failed, 0 pending | Healthy stalled-peer controls in 0.203–0.383 s; trickled body ends in 30.36 s |
| AArch64 signed release | 20 passed, 0 failed, 0 pending | Complete release receipt and exact-generation ready result |

Strict host/library and native service Clippy pass. Both signed architecture
bundles build; AArch64 reproduction verifies current-source signed bytes.
Rust formatting, shell syntax and diff whitespace checks pass. The existing
manual PDF remains unchanged; Markdown and LaTeX sources carry this update.

The first AArch64 security run expired its 300-second guest budget with no
Charlotte kernel output. The capture ends in UEFI diagnostics; CPU0 PC is
`0x5fdf74ac`, and the three secondary PCs are zero. This failed run is retained
separately from the fresh-storage repeat. It did not exercise the changed
services, and the passing repeat does not identify or resolve its cause. No
production or fixture deadline was widened. The signed-release fixture also
records one connection refusal and two HTTP 409/error -23 admissions before its
accepted release and ready result; those intermediate diagnostics remain in
the evidence.

`CharlotteDeploymentOutput` adds two admitted outputs with two abstract prefixes
each. Its safe configuration explores 299 states at depth 11. Replay of an
uncertain prefix violates `PrefixesOnce`; restarting a chunk's budget violates
`OriginalBudget`. Safe checks also prohibit parking with a ready receipt or
expired owner. Atomic turns, bound sources and producer delivery are assumptions;
byte framing, allocation, dynamic admission, TCP correctness, peer delivery,
wall-clock scheduling and socket close are outside the model. The full suite
checks 39 safe configurations and 115 expected counterexamples across 37 models.

## Remaining work

Socket open/listen/close, response rendering and controller-server work can
still block the ingress loop. Four occupied positions deny another client;
there is no protected operator slot or per-peer flood policy. Actual saturated
TCP-buffer and whole-service shutdown/flood/NIC-failure qualification remain
open. Request reservations, output capacity and socket quotas do not implement
general principal/heap admission or prove release of uncertain remote charges.
SEC-06/07 acceptance remains broader than this response-send milestone.
