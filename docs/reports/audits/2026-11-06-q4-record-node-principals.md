# Q4 authority and mailbox node principal funding

Baseline `9fe49d90`. Unified authority records and mailbox endpoint nodes now
fund their requested layouts against the original verified principal through
actual node deallocation. This covers two more existing catalog families. Q4
remains in progress; no finding disposition, aggregate gate, production trust or
physical-platform acceptance changes.

[Version-controlled evidence](evidence/2026-11-06-q4-record-node-principals.json)
contains frozen source/input identities, artifact hashes, selected policy, inline
milestones, commands and failed preflights. Historical reports remain unchanged.
The current [24-pool checklist](../../reference/metadata-admission.md#principal-coverage-and-remaining-reconciliation)
identifies remaining family, allocator-excess and joint-heap work.

## Capture, ownership and shared mechanics

The authority namespace now retains its complete immutable `CapturedStorage`
policy from the unpublished prebound root. `PreparedReservation` checks the exact
namespace generation under `CAPABILITIES`, copies that policy, leaves the guard
and funds its candidate. Delegation retains recipient storage sponsorship,
independently of logical grantor charges. Later promotion cannot reclassify that
namespace's original backing policy or select another refund recipient.

Mailbox preparation uses the principal already captured together with its exact
root and trusted class, before acquiring its owning root operation and allocating
the endpoint candidate. Earlier captures/nodes retain their original class;
a fresh capture after authorized promotion can use the existing trusted reserve
policy. Neither permits changing the original principal. Publication retains its
existing generation/retirement checks and grants no new resource privileges.

`NodeCharge<Family, Sponsor>` extends the existing shared typed charge. It funds
the actual prepared-node layout before family byte/count admission and allocation.
Family denial rolls back the unused principal reservation. Allocation rejection
returns the complete unstarted charge. The existing retirement collection keeps
that charge outside node deallocation, then refunds family and original-principal
bytes. No new node owner, identity registry, cleanup family or physical completion
rule is introduced. Unmigrated families retain the default unsponsored 16-byte
charge and do not gain principal coverage from sharing the implementation.

The existing inert preparation, mailbox close and final-root owners still retain
complete nodes through abandonment or uncertain cleanup. Actual physical
completion remains their prerequisite for release; a funding token is no root
lease, release authority or retry owner. Permanent-kernel/trusted raw fixtures
retain their explicit unidentified exemption. Signed application construction
uses the prebound verified principal.

## Capacity and limit choice

A shared **8 MiB** `principal_metadata_bytes` allowance in the existing history
record funds migrated general metadata. Authority and mailbox endpoint nodes
currently use it; it does not automatically cover other metadata, allocator excess
or the general kernel heap. It remains independent of the 16 MiB buffer and two
1 MiB account allowances, so retained nodes do not consume their protected funding.

One vector adds 16 bytes to each existing principal-history node. Native layouts
are 232 bytes before and 248 after this change. At the unchanged 4096 total/3072
ordinary node ceilings, requested history storage is 1015808/761856 bytes,
below 1 MiB/768 KiB. **No existing pool limit needs raising.** This preserves
count capacity without a new counter registry or compacting away ownership state.
History nodes themselves still need their own Q4 sponsorship/exclusion review.

The funded node charge occupies 32 bytes. Both authority and mailbox payload
nodes now measure **96 bytes**, up from 80. Authority count ceilings imply
9/12 MiB ordinary/total layouts, within 12/16 MiB. Mailbox count ceilings imply
1152/1536 KiB, within 1536/2048 KiB. These are bounds within already counted byte
pools, not additional RAM to add to the catalog sum. Global logical limits,
byte/count pools, heap reserve, protocols, SDK and signed service bundles stay
unchanged. The catalog still has 24 pools and 19 byte-denominated independent
ceilings totaling 205.5/154.125 MiB total/ordinary, not joint observed usage.

The 8 MiB allowance intentionally makes one principal's retained metadata bind
before the authority pool's ordinary count ceiling. Raising it admits more retained
memory; lowering it can reject authority and endpoint preparation earlier.
No exercised workload needs a larger global pool or smaller logical limit.
The allowance grants no per-service fairness or cleanup capacity guarantee.

Capturing the complete immutable policy enlarges the capability namespace node
from 112 to 136 bytes. The other four namespace layouts remain 128/224/96/136,
for 720 bytes per five-node root. Twice complete-root count headroom needs
5898240/4423680 total/ordinary bytes, within 8/6 MiB. Capacity review also corrected
the current reference's pre-existing unconditional count-first assertion: an
isolated mix of 224-byte completion nodes reaches byte limits at 37449/28086
total/ordinary nodes before the shared count limits. Existing count-pressure
controls use capability nodes. The complete-root mix fits; allowing more of the
largest type would cost more heap and is not required by that contract. Historical
reports and passing results are preserved, without extending their scope.

Runtime pool samples still report family bytes/counts, not principal usage.
The policy census now includes the selected metadata allowance; wider admission
and coherent joint heap fit remain open.

## Controls and qualification

Native fixtures prebind private principals at the trusted constructor boundary;
combined security campaigns separately exercise verified signed launch. Actual
allocation deltas reconcile both 96-byte layouts and sibling sharing.
128 injected funded allocation failures restore family and principal baselines.
128 authority and 128 endpoint quota denials occur with the real heap held;
established receiver reuse still succeeds. Four real endpoint/authority quota
prefixes reject before endpoint allocation or before complete authority funding,
preserving node/resource counts and the original principal's bytes. A different
healthy principal can still prepare work, and promotion cannot redirect funding.

Real detached authority retains original bytes across root close and exact ASID
reuse by a different funded principal. Explicit release returns old bytes while
leaving the successor's independently funded node unchanged. Existing global
ordinary/total pressure and trusted reserve controls continue to pass with the
larger node layouts. A default-policy workload holds **87381 real empty authority
candidates**, using 8388576 of 8388608 principal bytes, with ordinary family slots
remaining. Disposal restores both principal and family baselines. This serialized
candidate pressure is not concurrent EL0 or essential-service progress acceptance.

Existing negative controls use the same deliberately retained objects. The
unpublished and retired authority records retain **192 original-principal bytes**
through root close/reuse under history, lifecycle/registry, heap and family guards.
Complete unpublished and retired mailbox parents each retain **192 bytes**, their
exact roots and original authority/resource owners under history/publication/
physical/heap/pool guards. Closing those roots remains Pending. No new deliberate
leak is introduced. These controls qualify those containing owners, not arbitrary
reference destructors or physical retry authority.

Host controls execute production funded-node admission with principal rejection,
family-rejection rollback and transferred final release. Existing production
retirement-collection controls independently verify actual deallocation before
charge refund. The full host suite retains allocation/ownership, buffer, crypto
and protocol controls. Native heap exhaustion is not forced.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting, including standalone byte-allocator fixtures | Passed |
| Full host suite and policy profiles | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19; 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each; 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed; 90-second budgets |

Final qualification uses 56 frozen source/input identities. Native AMD DWARF
independently confirms the 96-byte authority/endpoint nodes, 136-byte capability
namespace node and 248-byte history node in that campaign's exact kernel.
The baseline history inspection matches the preceding qualified Intel minimal
kernel identity. All five campaign artifact identities are recorded in the linked
evidence, including unchanged signed service bundles.

## Preserved preflights and limits

An early compiler check rejected unused test hooks before the corresponding
controls were connected. The first full host campaign then failed compiling the
frame-tracker harness with `-D warnings`: the added node-reservation fixture used
`Atomic::fetch_update`, deprecated by the pinned toolchain in favor of
`try_update`. Updating that fixture corrected the check; the failure and its
traceback remain separate from the final host campaign. It was not a runtime
frame-tracking failure. Final strict profiles were checked again before native
qualification. No native failure or deadline increase occurred.

The ARM display second-grant-probe, Intel retirement-pin and ARM exit-watch
incidents remain unresolved. Passing headless campaigns neither explain nor close
them. Remaining catalog principal coverage, allocator excess, coherent joint heap
fit, wider caller/IRQ and implicit-destruction review, physical recovery and
sustained service/cleanup progress remain Q4–Q8 work. No aggregate gate is closed.
