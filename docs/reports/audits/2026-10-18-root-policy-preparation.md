# Root policy and telemetry metadata preparation

Date: 2026-10-18. Baseline `3a0454f8` plus the uncommitted
[stack ownership](2026-10-17-kernel-stack-ownership.md) and
[sponsor preparation](2026-10-18-memory-sponsor-preparation.md) changes.
This continues SEC-07 R7-1/R7-2 and SEC-18 C02/C03/C18/G1/G4.
The 38 finding dispositions remain 29 corrected within scope, six partial and
three open. No ABI, wire/storage format, registry or cleanup family is added.

## Source boundary

Root registration previously inserted launch limits and zeroed domain usage into
two `BTreeMap`s after publishing the address space, beneath lifecycle/table
serialization. Neither allocation could reject before publication. Final cleanup
also removed and destroyed these nodes beneath lifecycle serialization. These
were the next concrete metadata gaps after sponsor preparation, not an explanation
of the historical intermittent stack/IPC/persistence/NIC failures.

Both existing registries now use shared `AdmittedMap` nodes. `PreparingMetadata`
fallibly prepares two empty nodes before local publication guards. The existing
`PreparingRegistration` retains that preparation together with the actual
unpublished root, capability namespace and memory sponsor account. Partial child
and complete parent fallback retain every field without locks, allocation,
callbacks, refund or implicit field destruction. Ordinary preparation rejection
explicitly cancels unused metadata, then disposes the same root, outside the
local publication guards. Failed table publication first returns the root into
its containing owner and uses that same disposal path.

Publication captures the assigned generation under the existing table hold and
only relinks the prepared limit/usage nodes. Defaults are initialized once; later
policy and telemetry updates mutate existing entries. The old allocating
registration functions are removed; there is no lazy compatibility path.

## Retirement boundary

After exact-root admission seals, cleanup detaches both nodes with their captured
generation and returns them to `ClosingAddressSpace`. Root extraction transfers
them into the same `RetiredAddressSpace` as its slot, mailbox/queue metadata and
unified authority namespace. A rejected final invalidation retains the complete
receipt. The existing bounded root recovery policy can retry that invalidation;
this change introduces no separate custody ticket, registry or replay authority.

Confirmed final invalidation releases both nodes after local lifecycle, table,
mailbox/queue, physical, heap, capability and policy/usage guards leave, before
root backing and slot completion. The entry IRQ state is preserved. Those are
local boundary observations, not qualification of every outer caller/mask or
implicit destructor. Abandonment retains the nodes and root; physical teardown
failure remains terminal under the existing policy.

## Controls and execution

Each native boot exercises 64 rejections at each of three preparation boundaries:
before either node, after limits and after both nodes. None reaches publication;
original root frames, capability admission, table occupancy and registry sizes
return to their baselines. Probes observe local guards available before allocation
and root disposal. Existing table-publication rejection additionally disposes
both fully prepared nodes through the parent owner.

Another 64 publications use the production insertion helper with private registries
while the heap allocator is held. Actual root close/reuse checks default limits,
zero usage and rejection of the old generation's policy mutation. These are
serialized boot controls, not concurrent hostile-pressure evidence.

The existing parent-abandonment fixture now retains two additional prepared nodes
with its unpublished root, namespace and complete sponsor metadata, while all
relevant guards and the allocator are held. A standalone partial fallback retains
one additional empty node, without a root or logical resource charge. Existing
failed-invalidation and abandoned-root fixtures inspect the exact retained nodes;
the abandoned retired receipt is dropped with lifecycle/table/policy/usage/heap
guards held. These expand existing retained fixtures without adding a retired
root. Their retained allocations remain live for that guest's lifetime.

The machine-readable [evidence](evidence/2026-10-18-root-policy-preparation.json)
records frozen Rust hashes, commands, artifact hashes and execution markers.

Host validation passed 509 Rust tests, with zero failures and four ignored tests
across 57 harness summaries, plus the host runner's signing/tooling controls.
Strict Clippy passed for both custom kernel targets; `cargo fmt --all -- --check`
and `git diff --check` passed. Nine Rust files change in this batch; all 21
modified/new Rust files, including the two prior patches, are frozen in the
evidence. Staged service bundles remain byte-identical to the sponsor batch.

All five fresh-storage QEMU profiles used four LPs and a 180-second fixture
deadline. Every runner exited zero and archived diagnostic result `complete`:

| Profile | Deferred pass/fail/pending | Completion, guest seconds |
| --- | --- | --- |
| AArch64 / SMMUv3 | 17 / 0 / 0 | 11.441324 |
| x86 / Intel VT-d | 15 / 0 / 0 | 19.254460 |
| x86 / AMD-Vi | 15 / 0 / 0 | 16.364978 |
| AArch64 / SMMUv3 security | 19 / 0 / 0 | 56.104159 |
| x86 / Intel VT-d security | 17 / 0 / 0 | 61.657357 |

The matrix contains 960 preparation rejections and 320 heap-held metadata
publications. Each run also completed the existing 64 scheduled IPC claim/close
episodes. No failed native run occurred in this batch. Prior intermittent failures
remain separate historical observations; these passes provide no causal diagnosis.
Full kernel/image/UART hashes, frozen source and service-bundle inventories are
inline in the version-controlled evidence; ignored build artifacts are not the
sole published support for these results.

## Remaining limits

Fallible allocation is not aggregate admission. These nodes have no independent
byte/principal budget. General kernel heap, logical-principal consumption and
essential-service progress under sustained ordinary pressure remain open.
`AdmittedMap` insertion/lookup is linear; hostile-pressure hold time is unqualified.
Signed authority insertion, principal stack/heap high-water maps and allocating
telemetry snapshots remain separate metadata gaps. Completion teardown, wider
implicit destructors, outer caller/IRQ contexts and previously reported intermittent
failures remain unresolved. No SEC-07 or SEC-18 closure is claimed.
