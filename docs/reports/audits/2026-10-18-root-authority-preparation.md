# Signed-authority slot preparation

Date: 2026-10-18. Baseline `33060e29`, which commits the preceding
[stack](2026-10-17-kernel-stack-ownership.md),
[sponsor](2026-10-18-memory-sponsor-preparation.md) and
[policy/usage](2026-10-18-root-policy-preparation.md) changes.
This continues SEC-07 R7-1/R7-2 and SEC-18 C02/C03/C18/G1/G4.
Finding dispositions remain 29 corrected within scope, six partial and three open
out of 38. No ABI, wire/storage format, cleanup family or registry is added.

## Source boundary

The signed ELF loader previously installed its authenticated principal/roles with
an infallible `DOMAIN_AUTHORITIES` `BTreeMap::insert` after publishing the root.
Final cleanup destroyed that registry node beneath lifecycle serialization.
This follows the concrete metadata inventory; it does not diagnose a historical
intermittent boot, stack, IPC, storage or NIC failure.

The same registry now uses shared admitted nodes. `PreparingMetadata` fallibly
prepares a third empty node before local lifecycle/table publication guards.
Its containing `PreparingRegistration` retains this preparation with the actual
unpublished root, capability namespace and sponsor account. Ordinary rejection
explicitly disposes unused metadata, then the same never-published root, outside
local publication guards. Partial/complete abandonment retains every field without
locks, allocator activity, callbacks, refund or implicit field destruction.

Root publication relinks an empty `DomainAuthoritySlot` tagged with the exact
assigned generation. Empty slots grant no identity. The existing signed loader
still verifies the image and signed metadata before creating the root; it fills
this already-admitted slot without allocation. Stale generation, zero principal
and duplicate installation reject before mutation. The trusted setter continues
to assert its loader invariants; there is no public API or lazy fallback.
Descriptor installation retains its exact generation and once-only digest binding.
Duplicate rejection now leaves the previous authority unchanged.

## Retirement boundary

Final cleanup copies the principal and drops its local authority borrow before
sampling usage. Principal high-water maps still allocate beneath lifecycle and
remain separate work. Sealed cleanup detaches the entire identity slot, including
any principal/roles/descriptor digest, with the existing limit/usage metadata.
It follows the same `ClosingAddressSpace` and `RetiredAddressSpace` receipt as
the exact root, slot and capability-authority namespace; those two authority
representations remain distinct.

Failed final invalidation retains the populated slot in that complete receipt.
Existing bounded root invalidation retry remains unchanged. Confirmed invalidation
releases the node outside local lifecycle/table/mailbox/queue/physical/heap/
capability/policy/usage/signed-authority guards, before root backing and slot
completion, preserving entry IRQ state. Abandonment and started physical failure
retain their existing terminal policies. No separate identity ticket, custody
registry or physical/replay authority is introduced. Local boundary probes do
not qualify every outer caller, mask or implicit destructor.

## Controls and execution

Each native boot injects 64 rejections at each of four preparation boundaries:
before limits, after limits, after usage and after the authority slot. None
reaches publication. Original root frames, capability admission, table occupancy
and all three registry sizes return to their baselines. Allocation/root disposal
probes observe local guards available. Existing table-publication rejection
also cancels the complete three-node preparation through its containing owner.

Another 64 publications use the production insertion helper on private registries
with the heap held, confirming all authority slots start empty. Actual root tests
hold the allocator through the trusted identity/descriptor setters and lookup;
they check principal, roles and exact descriptor digest. Duplicate installation
preserves the original identity. Close/reuse inherits no authority, and stale
generation/zero-principal rejection leaves the successor empty. These setters
are trusted kernel fixture calls, not image-signature verification inside a heap
hold. Existing signed partial/successful loader controls and security profiles
exercise their real callers separately.

The existing failed-invalidation root fixture now carries a populated authority
and descriptor: rejection returns the same exact values, while external lookup
sees no detached identity and a later reused ASID starts empty. Existing guarded
root abandonment also holds the signed-authority registry. Metadata release probes
now include that registry alongside the prior guard/IRQ checks.

Standalone partial abandonment retains three empty nodes across two phases,
two more than the preceding batch. The existing complete unpublished-root fixture
retains one additional authority node with its original payload. Each existing
retained detached-root fixture now retains its authority node as well. No new
retained root is added. These are real per-guest retained allocations without an
independent metadata-byte charge, not a complete heap inventory or pressure proof.

The version-controlled [evidence](evidence/2026-10-18-root-authority-preparation.json)
records source/artifact hashes, commands, outcomes and concise markers.

Host validation passed 509 Rust tests, zero failures and four ignored tests
across 57 harness summaries, plus the host runner's signing/tooling controls.
Strict Clippy passed for both custom kernel targets; `cargo fmt --all -- --check`
and `git diff --check` passed. The six changed Rust files are frozen in the
evidence against the committed baseline. Staged service bundles are reused
without userspace changes and their inventories are checked against the prior batch.

All five fresh-storage QEMU profiles used four LPs and a 180-second fixture
deadline. Every runner exited zero and archived diagnostic result `complete`:

| Profile | Deferred pass/fail/pending | Completion, guest seconds |
| --- | --- | --- |
| AArch64 / SMMUv3 | 17 / 0 / 0 | 13.327255 |
| x86 / Intel VT-d | 15 / 0 / 0 | 19.548225 |
| x86 / AMD-Vi | 15 / 0 / 0 | 14.980490 |
| AArch64 / SMMUv3 security | 19 / 0 / 0 | 56.327378 |
| x86 / Intel VT-d security | 17 / 0 / 0 | 61.864354 |

The matrix contains 1,280 metadata preparation rejections and 320 heap-held
three-registry publications, plus authority installation/descriptor/generation
and populated failed-invalidation controls in every boot. Each run also completed
the existing 64 scheduled IPC claim/close episodes. No failed native run occurred
in this batch. Prior intermittent failures remain separate historical observations;
these passes provide no causal diagnosis. Full artifact/source hashes and concise
execution markers are inline in the version-controlled evidence; ignored artifacts
are not the sole published support for these results.

## Remaining limits

Principal high-water maps, allocating telemetry snapshots, completion teardown
and other metadata/destructor contexts remain open. General heap bytes, aggregate
logical-principal admission and essential-service progress under ordinary pressure
are unqualified. `AdmittedMap` lookup is linear, including the identity lookup used
by IPC; hostile-pressure latency/hold time needs separate evidence. Empty-slot
preparation adds real per-root heap cost, not a new logical bound or reserve.
Platform authentication, rollback-resistant enrollment/state and key custody are
unchanged and production trust remains disabled. No SEC-07/18 closure or diagnosis
of historical intermittent failures is claimed.
