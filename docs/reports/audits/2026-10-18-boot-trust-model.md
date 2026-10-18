# Boot trust and installation ordering — 18 October 2026

## Scope

Baseline `adae8b2f` commits the preceding security/model batches. This follow-up
adds [`CharlotteBootTrust`](../../tla/CharlotteBootTrust.tla), two safe
configurations and ten required counterexamples. No Rust or production behavior
changes are introduced. The existing signed-policy verifier, x86 fixture and
one-shot kernel handoff already enforce their reviewed checks.

The two configurations have different meanings:

- **Fixture/development projection:** implemented behavior, including relative
  signature/lineage verification, the fixture's exact installed-state comparison,
  recipient binding, explicit development configuration and immutable publication.
- **Conditional installation contract:** assumes the protected context, atomic
  rollback-resistant state and publication serialization that production still
  lacks. It explores concurrent stale expectations, failed/interrupted commit
  and reboot. This is an obligation for a future adapter, not an implemented
  installer or production trust claim.

Production remains disabled. SEC-04 and the finding counts remain unchanged.
The [trust reference](../../reference/bootstrap-trust-policy.md) records the
missing platform, persistent-state and custody boundaries.

## Source comparison

- [`signed_policy.rs`](../../../crates/charlotte-launch/src/trust/signed_policy.rs):
  explicit enrollment/installed expectations, exact current digest, immediate
  successor/predecessor checks and immutable `VerifiedPolicy` receipts.
- [`trust_policy.rs`](../../../tools/cluster-sign/src/trust_policy.rs): explicit
  CLI modes and verification against caller-supplied state. Signing and
  verification perform no protected state installation.
- [`boot_fixture.rs`](../../../crates/catten/src/service/admission/boot_fixture.rs):
  x86 public test material, borrowed module copy, exact compiled installed
  sequence/digest check before preparation. A valid direct successor is still
  uninstalled and rejects before boot publication.
- [`admission.rs`](../../../crates/catten/src/service/admission.rs): the
  zeroizing recipient owner, launch-cluster/public-binding checks, one-shot
  `publish_into`, returned preparation on rejection and common boot view.
- [`launch.rs`](../../../crates/catten/src/service/launch.rs): explicit
  development versus fixture selection; publication before manifest composition.
  The operational gate borrows the same installed kernel slot.
- [`build.rs`](../../../crates/catten/build.rs) and
  [shell admission](../../../scripts/lib/signing-policy.sh): production refusal
  remains active. No model parameter enables a production build.

The [conformance map](../../tla/CONFORMANCE.md#boot-trust-and-conditional-protected-installation)
labels unimplemented transitions explicitly. A leftover authorization-table
sentence was also corrected: authorized lookup may resolve a public NS entry
when policy permits it, as the preceding NS review established. That is a
documentation correction with no lookup behavior change.

## Finite contract

Eight record classes represent the current record, two separately signed
successors, an initial higher revision, a gap/wrong predecessor, foreign context,
attacker signature and malformed input. Record identity denotes the complete
signed-fields digest; cryptography and key/role qualification collapse to an
authenticity classification relative to supplied inputs. Enrollment has a fixed
floor of one. Both instances have two boot epochs; the current fixture's
revision seven is scaled to one.

Verification accepts a current record or valid direct successor. It does not
advance durable state or grant boot authority. Fixture preparation requires the
current installed identity. The conditional contract additionally serializes
commit against the captured exact state: two siblings can verify against the
same old expectation, but a stale second commit cannot install a fork.

Definite commit rejection leaves state intact. An interrupted commit may have
atomically installed either the whole new record or nothing. The caller enters
a blocked state and cannot publish its cached receipt; a fresh protected read
must precede later verification/preparation. That read is qualified only after
protected recovery has settled or fenced the original writer. Observing old
bytes alone proves no quiescence. Atomicity and this recovery fence are assumed,
not proved. No arbitrary I/O error authorizes replay.

Boot publication is one-shot even for the same record. A rejected duplicate or
stale preparation retains its candidate/key state until explicit local disposal.
Consumers copy the policy from that same slot. Reboot clears volatile receipts,
the slot and consumer view while preserving committed identity; it represents
machine restart, not a runtime reset API or proof of memory erasure/custody.

| Weakened action | Required violated invariant |
|---|---|
| `UnsafeUninstalledPublish` | `InstalledBeforePublication` |
| `UnsafeStaleCommit` | `NoForkedInstallation` |
| `UnsafeEnrollmentFallback` | `NoEnrollmentFallback` |
| `UnsafeOverwrite` | `OnePublicationPerBoot` |
| `UnsafeIndependentManifest` | `SharedPolicy` |
| `UnsafeWrongRecipient` | `RecipientBound` |
| `UnsafeAuthenticate` | `VerifiedAuthenticated` |
| `UnsafeDevelopmentFallback` | `ExplicitDevelopment` |
| `UnsafeFailedCommitPublishes` | `InstalledBeforePublication` |
| `UnsafeUnsupportedProduction` | `ConditionalBoundary` |

These are deliberately weakened obligations, not newly reproduced Rust defects.
The conditional controls do not imply that a protected installer currently exists.

## Validation

Both new safe configurations pass with all required actions exercised:

| Instance | Generated | Distinct | Depth |
|---|---:|---:|---:|
| Fixture/development | 689 | 234 | 15 |
| Conditional protected contract | 2,580 | 796 | 17 |

All ten negative configurations fail on their named invariant and exercise their
weakened action. The two safe graphs have no queued states remaining. Independent
final repeats retain complete action sequences for the ten counterexamples.
The existing signed-policy host tests pass **9/0**; shell signing-policy controls
also pass, including early production rejection by runner/build wrappers.

The configured suite now has **30 modules, 32 safe instances and 87 negative
configurations**. Its preceding 107 cases reuse the committed baseline: all
136 existing model/configuration hashes match, and removing only the twelve new
case declarations restores the previous harness byte-for-byte. New cases use
its exact unchanged runner functions and required-action checks. The manifest
hashes all **150** current model/configuration/harness inputs and records this
incremental provenance; it claims no fresh single full-suite invocation.

The manual passed two final LaTeX passes after a layout correction, with pages
198–211 rendered and inspected. Chapter 21 has no overfull boxes; existing
warnings elsewhere remain outside this change. Its inventory matches the harness
and all 316 local Markdown destinations in changed documents resolve. The
user's concurrent chapter 6/PDF commit `95c66e4f` remains intact; the tracked
PDF was not rebuilt by this follow-up.

The [evidence manifest](evidence/2026-10-18-boot-trust-model.json) retains bounds,
source and checked-input hashes, required-action coverage, counterexample action
sequences and verification provenance. The new cases run through the current
repository harness's unchanged functions and exact new case declarations.
The preceding [NS evidence](evidence/2026-10-18-name-publication-model.json)
supplies the unchanged-model baseline; hashes are checked before any prior
result is reused. This is incremental verification, not a claimed fresh single
full-suite invocation.

TLC uses the pinned TLA+ Tools v1.7.4 JAR, reporting TLC 2.19/revision `5a47802`,
SHA-256 `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.
Local Java is Azul OpenJDK 25.0.3; CI uses Java 17. A first draft failed semantic
analysis because two existential branches shared a variable scope. Explicit
parentheses corrected that draft; both final safe instances and all controls
are checked afterwards. That draft failure is not an implementation defect or
a passing model run; its source was not separately archived.

The existing [native fixture report](2026-10-08-security-qemu-secure-boot.md)
retains its signed boot-chain and substituted-policy evidence. This batch adds
no native boot, multi-node, firmware, hardware, protected storage or custody
qualification. Earlier reports and their evidence remain unchanged.

## Limits and remaining work

These are finite safety checks. Protected enrollment, authentic platform inputs,
atomic rollback-resistant storage, serialization, custody and reset erasure are
assumptions of the conditional instance. They have no production implementation.
The model does not represent actual key bytes, zeroization, firmware variables,
disk writes, partial records, hardware memory lifetime, lock/IRQ context,
bootstrap-key rotation, higher enrollment floors or the full u64 sequence space.
Initial platform service loading still uses development roots. It neither
qualifies physical cleanup nor adds a recovery/custody owner.

The next formal gap is rejection of application snapshot restore before Raft
progress, membership or join completion can change. Production integration
and the broader cleanup/security audit remain open independently of this model.
