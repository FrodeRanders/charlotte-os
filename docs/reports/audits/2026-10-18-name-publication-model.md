# Local NS publication and death watches — 18 October 2026

## Scope and source findings

This continues the [DMA model synchronization](2026-10-18-dma-operation-model.md)
against baseline `1c6963a2887d7d69472af9179374f81ecd0750d4`, preserving earlier
uncommitted work. The new
[`CharlotteNamePublication`](../../tla/CharlotteNamePublication.tla) projects the
local name-service registry's publication classes, persistent death watches,
replacement, deferred lookup and logical retirement. Two safe instances and
thirteen required counterexamples supplement the authorization, endpoint-observer
and service-lifecycle models.

Source comparison also found a concrete exhaustion edge. `PolicyStore` checked
unsigned generation overflow, while NS returns generations as positive `i64`
values and accepts that same range in retirement records. Publishing after
`i64::MAX` could mutate the policy binding to generation `9223372036854775808`
before NS's signed conversion rejected it, leaving policy and registry out of
step. The store now checks `MAX_SERVICE_GENERATION = i64::MAX as u64` before
inserting a binding. Rejection preserves its generation, ceiling and active
state, including tombstones. No wire format or new cleanup owner is introduced.

A source comment also overstated class separation: both public and
policy-published registrations have policy-store bindings. Public lookup cannot
resolve a policy-published entry, but an authorized lookup can resolve a public
entry when its policy checks permit it. The comment and model now describe
that direction accurately; lookup behavior is unchanged.

## Implementation correspondence

- [`ns.rs`](../../../crates/catten-services/src/bin/ns.rs): `register`, immediate
  and deferred public lookup, authorization helpers, `retire_closed`, exact
  generation retirement, serialized CQ/request reactor and fail-closed aborts.
- [`ns_publication.rs`](../../../crates/catten-services/src/ns_publication.rs):
  complete connection/watch preparation, live quota predicate, current owned
  completion observation and watch-before-source field disposal.
- [`charlotte-authorization`](../../../crates/charlotte-authorization/src/lib.rs):
  checked service publication and retained generation tombstones. Role checks
  happen in policy publication, after candidate watch preparation.
- [`publication_generation.rs`](../../../crates/catten-services/src/publication_generation.rs)
  and [`charlotte-protocol-ns`](../../../crates/charlotte-protocol-ns/src/lib.rs):
  retirement authorization and the positive signed generation record range.
- [`loader.rs`](../../../crates/catten/src/service/loader.rs): trusted NS
  construction prepares the 64+1 completion/watch budget. Its actual admission
  and kernel watch provenance remain assumptions of this model.

The [action map](../../tla/CONFORMANCE.md#local-ns-publication-and-death-watches)
records direct and abstract boundaries. Historical native evidence remains in
the [endpoint-death report](2026-10-14-ns-endpoint-death.md) and
[bearer-removal report](2026-10-15-ns-bearer-removal.md); those executions are not
reclassified as new model or generation-exhaustion tests.

## Projected contract and bounds

A candidate owns its connection before fallible watch admission. Rejection
leaves the old publication authoritative. Public replacement requires an active
public entry with the same nonzero registrar. The policy manager/role and
generation checks precede policy mutation. Registry installation follows as a
separate private step; the serialized reactor admits no request between them.
Deferred replies use the installed entry before old-owner disposal.

The general instance has two atomic names, two principals, two candidate
publications and live capacity one, with one spare watch for replacement.
One lookup result and one queued lookup explore immediate/deferred public and
authorized paths. Authorization success is an input from the separate policy
model, not a second rights proof. Issuance snapshots the then-current
generation; later replacement does not retroactively revoke delegated authority.

Every real publication hint is deliberately allowed to alias the current watch;
an extra unrelated hint is also present. Even a matching old hint cannot
substitute its result for the current owner's own confirmed close. This stresses
hint handling without claiming concrete operation IDs are reused. Endpoint-close
results are authentic per-publication input outcomes. Unexpected observations,
rejected waits and lifecycle interruption retain registry owners through terminal
abort. Explicit retirement separately checks exact generation and nonzero
registrar/manager authority. Logical removal keeps the generation tombstone;
watch disposal precedes source close.

The exhaustion instance uses three candidates and generation limit two. It
restricts publication to one selected public name/principal and omits lookup,
CQ, endpoint-close and abort interleavings already checked in the general
instance. It reaches the last generation and attempts publication again both while
the binding is active and after retirement. This keeps exhaustion reachable without multiplying every lookup/watch
history by a third candidate. Neither instance establishes liveness or a Rust
refinement proof.

| Weakened action | Required violated invariant |
|---|---|
| `UnsafePublicLookup` | `PublicIssuance` |
| `UnsafeDeferredLookup` | `PublicIssuance` |
| `UnsafePublicReplacement` | `ReplacementProtected` |
| `UnsafeMutationBeforeWatch` | `BeforeWatchUnchanged` |
| `UnsafeHintRetires` | `DeathNeedsExactResult` |
| `UnsafeStaleHint` | `DeathNeedsExactResult` |
| `UnsafeCloseSourceFirst` | `WatchRetainsSource` |
| `UnsafeNewAtCapacity` | `LiveBound` |
| `UnsafeAbortDisposes` | `AbortRetainsOwners` |
| `UnsafeUnauthorizedRetire` | `RetirementAuthorized` |
| `UnsafeGenerationWrap` | `GenerationNotReused` |
| `UnsafePublishExhausted` | `PolicyGenerationRepresentable` |
| `UnsafeLookupDuringPublish` | `GrantGenerationCoherent` |

The beyond-range publication control corresponds to the reproduced source
finding. Other controls weaken reviewed obligations; they are not claims of
newly reproduced Rust defects.

## Generation regression and implementation checks

Two production-store unit tests seed its private generation state at the
boundary, avoiding an impractical sequence of `2^63` registrations. Both failed
before the range check. The last-legal-generation test first publishes
`i64::MAX`, then expects exhaustion; the second covers active bindings and
tombstones at that limit, beyond it, and at unsigned exhaustion. The controlled
failure returned:

```text
left: Ok(ServiceBinding { generation: 9223372036854775808, ceiling: AuthorizationRights(3), active: true })
right: Err(ServiceGenerationExhausted)
```

After correction, both tests pass and compare the complete unchanged binding
on rejection. A separate repeat removes only the new range-check line from
the final source, compiles its unit tests directly, and reproduces both failures.
The evidence records that exact source construction and hash, so this repeat
is distinct from the first Cargo failure and from passing results.

The authorization crate passes 16 tests, including both new boundary controls;
the complete host suite passes **519 tests, zero failed**. Strict Clippy passes
for that crate's host targets and the NS service on ARM/x86, and workspace
formatting passes. AArch64 services were rebuilt and signed through
`scripts/build-catten-services.sh --embed`.

A fresh-store, no-network ARM/SMMUv3 QEMU boot of that bundle, four LPs and a
180-second budget, exited 0 with **17 deferred tests passed, zero failed or
pending**. It reported bitmap `0x1ffff` at 19.940796 seconds. Kernel SHA-256 is
`bb8f0e4de588ec198675af9823616d0baf908e80f9ea5bf460c859d0f8521678`;
signed NS SHA-256 is
`77d0cb593aa1743e7eb76cb628d81e8555aa6c90ed9aaf2372bcd0c2e14f83be`.
This is ordinary service/boot integration, not native exhaustion or live-quota
saturation. QEMU arguments, serial hash and boot/bundle artifacts are in the
manifest. No new x86 native execution or multi-node qualification is claimed.

The manual passed three LaTeX passes in a temporary output directory; pages
198–210 were rendered and inspected. Chapter 21 has no overfull boxes; warnings
elsewhere remain outside this change. Its inventory matches the harness and
all 620 local Markdown destinations in changed documents resolve. The user's
existing chapter 6 edits and tracked PDF were preserved.

Complete suite coverage passed across the recorded partitions: **30 safe
configurations / 77 expected counterexamples**, all required actions exercised,
with **130,369,086 generated / 24,767,037 distinct
states** summed across safe runs and no states left queued. The general NS
instance explored **17,941,477 / 7,048,680**, depth 25;
the exhaustion instance explored **155 / 127**, depth 21.
The manifest hashes all **137** checked model/configuration/harness inputs and
records every case. The unchanged four-safe/thirteen-negative prefix plus the
corrected remaining 90 cases cover all 107 final harness entries. This is split
verification, not a claimed exit-zero single invocation.

The exact beyond-range trace reaches `UnsafePublishExhausted` after two
successful publications and a third watched candidate. Policy advances beyond
the representable bound before registry installation, matching the source
finding. The safe projection instead rejects that candidate without mutation.

## Evidence and limits

The [version-controlled evidence](evidence/2026-10-18-name-publication-model.json)
retains checked-input/source hashes, finite bounds, safe results, required action
coverage, all thirteen counterexample action sequences and failed/passing
implementation observations separately. The checker is the pinned TLA+ Tools
v1.7.4 JAR, reporting TLC 2.19, revision `5a47802`, SHA-256
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.
Local Java is Azul OpenJDK 25.0.3; CI uses Java 17. Local worker coordination
requires running TLC outside the filesystem sandbox.

A full-harness attempt also exited 1 after both NS invariant checks passed:
its exhaustion coverage check requested `RetireExplicit`, but TLC reported
that quantified call under `ExhaustionNext` (12 new / 24 total successors).
The corrected `ExhaustionRetire` wrapper names the same transition explicitly.
The final verification retains the completed unchanged-model prefix from that
attempt, reruns both NS instances, and executes every remaining final harness
case. The manifest records this split and the failed coverage hook separately;
it does not present the earlier exit as a passing single full invocation.

During development, the first draft stopped on an out-of-domain expression;
that expression was corrected. Larger/intermediate exploration and one full
harness invocation were interrupted while the bounds and overflow control were
being finalized. Those are incomplete checks, not passing runs or implementation
failures. The manifest distinguishes final checked inputs; intermediate source
files were not separately archived.

Kernel identity capture, authentic watch results and trusted 65-slot launch
preparation are assumptions. The model does not prove complete-name wire parsing,
actual allocator/metadata/IRQ/guard/destructor behavior, native 64-publication
saturation, concurrent death/scan ordering across several live entries, all
pending-lookup/grant forms, crash recovery or physical cleanup.
Names are complete identity atoms, not byte encoders. Abort grants no recovery
or quiescence receipt. C18 and SEC-07/18 remain partial; finding counts do not
change. Boot-policy installation ordering and rejected application snapshot
restore remain the next formal gaps.
