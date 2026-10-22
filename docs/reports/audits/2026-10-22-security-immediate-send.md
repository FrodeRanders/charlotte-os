# Immediate socket send: full identities and owned frames

Date: 22 October 2026. Reviewed baseline: `75dbe83b`.

SEC-06 remains partial. Immediate TCP/UDP send now carries the complete socket
ID and an exact caller-owned frame. Its former packed argument discarded the
high 32 bits, although socket allocation now supports nonreused positive 64-bit
IDs. A high-ID request could therefore select a different live socket with the
same low half. Exact sender-generation/principal checks still constrained that
selection to the caller's own sockets; this was not a demonstrated cross-owner
grant. The old immediate path also reconstructed memory cleanup from scalar
handles. Both paths are removed. Finding counts remain 29 corrected, six partial
and three open; production trust remains disabled.

## One send grammar

Immediate `OP_TRY_SEND` (6) and deferred TCP `OP_SEND` (16) both take the full
socket ID in `arg0` and one exact 4096-byte moved `CSOCKTX1` frame. The existing
frame has an eight-byte tag, little-endian u32 payload length, four zero reserved
bytes, 1–4080 payload bytes and a zero tail. Missing, oversized, untagged or
malformed pages reject. There is no packed decoder, length default, selector
mask or old-client branch. The frame tag and opcodes remain unchanged; the
existing grammar is now mandatory for both operations.

`OwnedSocket::try_send_call` prepares the immediate caller-owned page. It and
deferred `send_call` share frame preparation. `send_all` chunks against the same
4080-byte bound. NTP preserves its 48-byte datagram inside that frame and keeps
its existing preparation-phase diagnostics. The two-node smoke client uses
`try_send_call`. All current producers migrate with the service.

Immediate TCP can acknowledge a positive enqueued prefix; UDP acknowledges its
whole datagram. A zero/negative result is not a general rollback receipt for
earlier work. Accepted timeout or lost observation does not authorize replay.
The existing synchronous caller budgets/backoff and NTP observation policy are
unchanged. No accepted-send retry or new readiness source is added.

## Complete incoming ownership

The authenticated receive boundary adopts the complete immediate call exactly
once into `IncomingMessage`, then `PendingSend`. This existing owner retains
request/reply authority, caller-sponsored backing, read-only mapping and enqueue
state. It rejects unexpected connection attachments and invalid page shape
before backend enqueue. Mapping failure returns that same owning transaction
for rejection; no bare memory handle or manual cleanup ladder remains here.

After canonical decoding, lookup uses the exact full ID and captured sender
address-space generation/principal. Immediate send calls the existing once-only
enqueue adapter once, then releases mapping/backing before consuming reply
authority. It never installs a deferred waiter. Deferred send still retains its
owner while handshaking or buffer-full. Both use the original backing sponsor;
neither readiness nor logical enqueue grants physical cleanup or charge-refund
authority. Returned/submission failure owners retain existing runtime cleanup.

This extends C18's existing send owner. It adds no root, pin, registry, quota
family or recovery custody. The service-loop path introduces no local guards or
IRQ manipulation. General containing destructor, reply failure, cancellation and
pressure qualifications remain separate.

## Verification

The [version-controlled evidence](evidence/2026-10-22-security-immediate-send.json)
records source/artifact hashes, commands and results. All 204 host library tests
pass. The new identity control publishes two live IDs with equal low halves and
different socket kinds, checks exact selection, rejects an unissued high ID and
rejects another sender generation. Existing codec controls cover exact maximum
payload, malformed/truncated/extended/retired frames, reserved/tail bytes and
invalid lengths. Existing enqueue controls fence a positive prefix or ambiguous
error against a second write.

The native scoped fixture submits a canonical UDP request, then rejects missing
memory, a retired untagged page with its old packed argument, dirty reserved/tail
bytes, an invalid length and an oversized memory object. A canonical page with
an unissued high-half ID must return `ERR_BAD_SOCKET`, even though its low half
names the caller's live socket. A final canonical send succeeds through that
original socket. Both scoped roots run the fixture. HTTP exercises immediate
TCP through ordinary `send_all`; signed TLS/S3 release exercises those current
client helpers as well.

The initial x86 security run failed after both new send markers, both application
probe-success markers and both application publication-retirement checks. It
panicked at `el0_security.rs:55` with
`security probe reclamation: AddressSpace(OperationDrainTimedOut)`.
The captured LLDB caller is `verify` at line 543, the final `flood.stop()` for the
separate mode-1 cancellation-stress domain. The five-second reclamation policy
is unchanged. At capture the other three LPs were in their idle loop; that
snapshot proves no root quiescence or cleanup completion. The error and stack do
not distinguish an outstanding exact-root lease from pending IPC/memory cleanup.
No causal link to this change, pressure, scheduling or a particular operation is
established. The failed run and its selected diagnostics/hashes remain separate
from the same-source/budget repeat. No timeout is widened or failed close retried
through a reconstructed owner.

| Run | Result | Observation |
|---|---|---|
| AArch64 security | 19 passed / 0 failed / 0 pending | Both scoped send fixtures; NTP sample observed |
| Initial x86-64 security | Failed: reclamation panic | Both scoped send fixtures passed; no terminal self-test result |
| x86-64 security repeat | 17 / 0 / 0 | Both scoped fixtures; cancellation traffic retired after 17,048 requests |
| AArch64 HTTP | 19 / 0 / 0 | All 14 host controls; healthy concurrent replies in 0.254–0.357 s; trickled-body rejection in 30.14 s |
| x86-64 HTTP | 17 / 0 / 0 | All 14 host controls; healthy concurrent replies in 0.227–0.380 s; trickled-body rejection in 30.25 s |
| AArch64 signed RustFS release | 20 / 0 / 0 | Accepted release, generations 1/1, desired/ready replicas 1/1 |

The repeat passes final reclamation, without explaining the initial timeout.
It uses the same source, timeout and signed service bundle, with a separately
recorded kernel hash; the two kernel artifacts differ.
The signed fixture first recorded one refused connection and two HTTP 409
responses with `{"error":-23}`, then obtained admission and readiness. Those
intermediate diagnostics are retained without attributing a cause. Strict host
and AArch64 Clippy, formatting and diff checks pass. Both signed bundles build;
the AArch64 current-source reproduction/signature check passes, and all native
manifests match the current signed services. The manual PDF is unchanged.

Legitimate high-ID allocation is exercised by the host control, not forced in a native guest.
Native map/submission/reply failure, unexpected connection attachment and
cancellation races are not injected. Success here does not establish saturated
buffer progress, packet delivery, physical reclamation or whole-parent shutdown.

The existing finite deployment-output model assumes admitted deferred sends and
abstract prefix receipts. Wire framing and socket selector width are outside its
projection. That model is unchanged; this follow-up relies on direct source,
codec, owner-state and native checks for these boundaries rather than claiming a
new formal refinement result.

## Remaining work

Socket open/listen, controller/render work and ordinary teardown can still block.
Saturated admission/send-buffer progress, all-position flood pressure, protected
operator capacity, NIC failure, syscall-return and whole-service shutdown latency
remain SEC-06/07 gates. The earlier synchronous-close policy's terminal
unstarted retention may consume remote quota without recovery custody. This
selector/ownership correction does not change those dispositions.
