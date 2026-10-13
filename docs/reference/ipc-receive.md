# Canonical IPC receive envelope

The canonical envelope introduced in runtime ABI 2 remains in current ABI 3.
`IpcRecv` (71), `IpcRecvBlock` (72)
and `IpcRecvVec` (73) return the same twelve register values. The SDK methods
are `ipc_recv`, `ipc_recv_block` and unsafe `ipc_recv_vec`; the owning runtime
methods are `Endpoint::try_receive` and `Endpoint::receive`. The former
`*_authenticated` methods and the nine-register syscalls 22/27/53 are removed.
Those numbers are unassigned and follow the ordinary invalid-syscall policy.

| Field | AArch64 | x86_64 |
| --- | --- | --- |
| Status | x0 | rax |
| Opcode | x1 | rdi |
| Scalar argument | x2 | rsi |
| Reply token | x3 | rdx |
| Sender ASID | x4 | r10 |
| Interface | x5 | r8 |
| Interface version | x6 | r9 |
| First memory capability | x7 | r11 |
| Connection capability | x8 | rcx |
| Sender generation | x9 | r12 |
| Signed-artifact principal | x10 | r13 |
| Supervisor roles | x11 | r14 |

All error returns clear the eleven payload/identity registers. Sender identity
is captured by the existing enqueue owner, not resolved from the ASID at receive
time. Trusted kernel/serialized boot-fixture senders without a user root have
zero generation; unsigned fixture roots may have a generation but no principal
or roles. Missing principal/roles confer no authorization.

Vector receive takes endpoint in x1/rdi and result-memory capability in x3/rdx.
The writable owned or BorrowWrite result object need not be mapped. It receives
a little-endian u16 count immediately followed by that many little-endian u64
capability IDs. Invalid, read-only or insufficient storage rejects without
consuming the queued message. SDK assembly is a compiler memory barrier and
declares every envelope output. Raw assembly callers must preserve unrelated
state outside those outputs; the fixtures keep ARM result pointers in x27 and
save x86 r12--r14 on their mapped stack.

`IncomingMessage` retains the existing ownership of its reply, memory and
connection attachments. Readiness alone neither dequeues nor adopts them.
Blocking receive still registers against endpoint readiness; pending/claimed
fronts, loan cleanup, cancellation and exact delivery publication retain their
existing contracts. This simplification adds no polling or cleanup owner.

A signed variant declaring runtime ABI 1 or 2 cannot be placed or launched on an
ABI 3 node. Rebuild and re-sign all variants and bundled services. The CDEPLOY6
layout and current persistent record tags are unchanged; exact target matching
rejects old executable requirements rather than guessing or rewriting them.
