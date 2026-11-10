# Q4 completion principal funding

Baseline `b25b018b`. Capability-backed completion, detached-operation and CQ
registry nodes now charge actual layouts to the original principal. The same
principal funds the bounded kernel CQ ring and backlog. The existing
[completion ownership contract](../../reference/completion-node-admission.md)
governs disposal and publication.

## Capture and release

The existing target captures now obtain the principal, exact root generation
and explicit platform resource class together. Each target matches its captured
generation to the namespace before preparation. Account identity, capacity and
queue source checks still govern publication. A root with trusted backing
continues to need the explicit platform grant to use completion reserves. The
shared helper uses the same grant lookup as the earlier class-only capture.

The three node layouts use the shared funded node factory and the existing
8 MiB principal metadata allowance. The CQ charge also reserves requested
backlog and heap ring bytes before allocation. The original CQ budget still
bounds queue count and bytes per domain and node. Mapping owners retain the
physical ring frame charge. The complete CQ owner disposes detached backlog
nodes and frees the heap buffers before returning the CQ storage charge.

| Native requested layout | Bytes |
| --- | ---: |
| Capability-backed completion node | 80 |
| Detached operation node | 152 |
| CQ node | 240 |
| Funded node charge, included above | 32 |

At 15360 ordinary and 20480 total nodes, even populations of the largest layout
request at most 3686400 and 4915200 bytes, within the 6/8 MiB family ceilings.
CQ storage keeps its own 3/4 MiB ordinary/total ceilings. The layout growth fits
the existing policy. Allocator excess and simultaneous occupancy across families
remain part of the joint heap plan.

## Controls

Two roots share a principal and an independent principal retains access to its
own headroom. The controls reconcile 192 funded allocator failures and quota
denial through each production adapter. With room for the CQ buffers but one
byte short of the node charge, replacement rolls back the full private prefix
under the heap guard. The existing queue source and all original charges stay
live. The independent principal replaces a queue and completes a detached
operation.

A retained capability/authority pair uses 176 bytes. Its retired CQ keeps the
240-byte queue node, 4288 bytes of heap ring and backlog, and a 152-byte detached
node after root close and exact ASID reuse by a different principal. Each refund
reconciles independently of the successor and a surviving completion reference.
The physical queue fixture charges 384 bytes of backlog alongside the node
and verifies that both alias rejections preserve the borrowed frame contents.
The existing abandoned private namespace retains 4960 bytes after root close,
including the namespace, queue node and buffers. The drop control holds the
principal history, node pools, registry and heap guards.

## Qualification

[Versioned evidence](evidence/2026-11-11-q4-completion-principals.json)
records the checks, native artifacts and failed initial runs. The first ARM and
Intel runs reach an older namespace fixture whose principal total accounts for
namespace, authority and mailbox nodes. The observed 5224 bytes include the
additional 4528-byte CQ node and buffers, while the fixture expected 696 bytes.
The updated fixture reconciles these components against the separate pools and
actual CQ charge.

The full host suite, four strict test/minimal profiles, formatting and capacity
census pass. Four-CPU fresh-storage security campaigns pass ARM SMMUv3 19/19,
Intel VT-d 17/17 and AMD-Vi 17/17. ARM and Intel minimal boots pass. Intel and AMD
security use the same kernel, embedded security fixtures, signed bundle and
initial disk. All five signed bundles match the prior qualified device batch.
The security and minimal deadlines were 240 and 90 seconds respectively.

Remaining Q4 scope includes completion record Arcs and holders, observer
backing, other pending families, allocator excess and the joint heap plan. Serialized fixture pressure establishes
rejection and retention boundaries. The wider caller and IRQ review, sustained
progress and physical completion gates remain open.
