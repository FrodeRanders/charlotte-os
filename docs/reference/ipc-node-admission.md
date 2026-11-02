# IPC pending-call and reply-token backing

Pending-call and reply-token records use the shared admitted node collection
with independent `ipc_nodes` requested-byte/count admission. This covers unused
candidates, published records and detached cleanup storage. It is separate from
the logical connection/call/reply dimensions in [record budgets](ipc-record-budgets.md),
unified authority backing, namespace accounts, queues and attachment buffers.

`ipc::node_budget::Charge` keeps the backing reservation and later-bound logical
record reservation in the same allocation. The common collection's `BindCharge`
contract binds that later reservation without replacing allocation admission.
The requested layout includes both fields, payload and linkage. No extra holder
allocation or cleanup registry is added. Both charges return after the actual
node has been deallocated, using the existing post-IPC disposal path.

## Preparation, publication and retirement

`PreparedCall` captures and retains the exact caller root before reserving its
logical pending/reply counts under IPC. The original reservation's trusted class
is passed unchanged to both node factories after IPC unlock. Each factory admits
the complete node layout before its allocator runs. Quota or allocator denial
returns `IpcError::ResourceLimit` and the existing complete preparation explicitly
cancels every prefix, with the root released last. A reply-node rejection leaves
no pending node, logical count or backing charge behind and precedes authority
and attachment publication. Invalid handles and rights still reject first.

Publication only relinks prepared nodes. Result observation, token claims, loans,
retirement and ASID reuse preserve each node's original backing and logical
sponsorship. Scalar reply completion, cancellation and close continue to carry
the complete detached node or list; no side ticket reconstructs its ownership.
Uncertain loan cleanup retains the live fenced record. Explicit confirmed
disposal frees payload and node before refund. This admission supplies no
physical proof, cancellation permission or replay authority.

Complete call preparation and scalar reply/list fallback remain inert under
IPC, heap and metadata-pool guards. Existing deliberate-retention fixtures now
retain their backing charges too. Wider active containing destructors, caller
IRQ masks and bounded-copy latency still need their own qualification.

## Capacity and administration

Default ordinary/total limits are 6/8 MiB and 18432/24576 nodes. Logical resource
defaults remain 6144/8192 pending calls and reply tokens separately. Supported
64-bit targets measure 200-byte pending and 176-byte reply layouts, including
the added 16-byte backing charge. At the ordinary logical ceiling, 6144 pairs
request about 2.2 MiB; 8192 total pairs request about 2.94 MiB. These are layout
planning figures, excluding nested storage, allocator overhead and other pools.

The independent pool keeps malformed or abandoned candidates bounded without
spending another resource family's reserve. Its limits allow layout/candidate
headroom and retain a quarter for trusted admission; they do not preallocate RAM
or guarantee that independent resource ceilings fit simultaneously. The new
charge adds 192 KiB at 12288 ordinary records, or 256 KiB at 16384 total records.
No existing limit is lowered or raised. Measured workloads may justify changing
the central pool limits together; broad heap and principal admission remain open.

The metadata catalog appends `ipc_nodes` as its 23rd row. Earlier identities,
record widths and the 22-word header stay unchanged. Statistics payload version
10 rejects versions 8/9; kernel, SDK, observe, HTTP and fixtures use the same
catalog. Both architectures' test/minimal services are rebuilt and signed.
Runtime register ABI and signed runtime ABI remain unchanged.

## Qualification and remaining scope

The [evidence](../reports/audits/evidence/2026-11-02-ipc-node-admission.json)
records production source identities, native artifacts, preflight rejection and
relevant diagnostics. Host allocation tracing checks that both backing and
later-bound resource admission return after actual node deallocation, for unused
and published nodes. SDK tests reject the previous payload and malformed catalog;
policy tests reject invalid IPC node profiles.

The full host suite and strict checks with/without kernel tests passed on both
architectures. Fresh four-CPU security campaigns passed ARM 19/19, Intel VT-d
17/17 and AMD-Vi 17/17. An additional ARM authenticated HTTP/TLS campaign passed
20/20 and validated all 23 inventory rows through the running observe/HTTP path.
All campaigns completed with zero failures or pending tests. The initial ARM
staging preflight rejection is preserved separately from its passing repeat.

Serialized native controls inject 256 allocator rejections across both real
layouts, exhaust ordinary/total admission with real empty nodes and prove quota
rejection before allocation with the heap guard held. Independent byte exhaustion
uses an explicit counter adapter; it is not real maximum-byte heap pressure.
Another 64 controls admit the pending prefix and reject the reply candidate,
checking exact logical counts, backing bytes/slots and unchanged authority.
Existing paired publication/detachment, batches, scalar reply, cancellation,
promotion and root-reuse controls exercise the same owners. Guarded complete
call and reply/list abandonment retain the new charges.

Connection/capability and endpoint payload-node layouts, nested attachment
storage, other weak/control storage, general heap and aggregate principal
admission remain separate work. These serialized controls do not establish
sustained hostile concurrent EL0 fairness or all caller/IRQ/destructor contexts.
SEC-07/18 remain partial.
