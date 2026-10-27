# Signed bootstrap trust policy

`charlotte_launch::trust::signed_policy` authenticates a fixed public admission
policy relative to an explicitly supplied bootstrap key, cluster and acceptance
state. Host signing and verification use the same allocation-free parser and
verifier intended for a future protected provisioning adapter.

This provides policy authentication and a kernel handoff, but no protected boot
implementation or firmware/persistent-state installer. Production builds remain
disabled. Normal boot explicitly selects development trust; a verified host
record does not change the compiled development roots or recipient key.

## Wire format

`CBTRUST1` version 1 is exactly **328 bytes**. Integers are little endian. All
lengths/version values must match exactly; trailing or truncated data rejects.

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 8 | Magic `CBTRUST1` |
| 8 | 2 | Version 1 |
| 10 | 2 | Header length 80 |
| 12 | 4 | Total length 328 |
| 16 | 32 | SHA-256 of the externally pinned bootstrap Ed25519 public key |
| 48 | 32 | Previous accepted signed-fields digest; zero only for initial enrollment |
| 80 | 184 | Canonical `CTRUST1` public admission policy |
| 264 | 64 | Ed25519 signature |

The signature is ordinary Ed25519 over:

```text
SHA256("CharlotteOS bootstrap trust policy v1\0" || bytes[0..264])
```

That digest is also the **accepted policy digest**. It binds the header, key
selector, predecessor, cluster, sequence and all role keys. It excludes only
the signature; signing the same payload with fresh signing noise does not
change acceptance identity. It is different from SHA-256 of the candidate file
or of the complete signed record. There is no unsigned extension area or
uncovered field.

`BootstrapKey::new` validates canonical, non-small-order, prime-order Ed25519
material and rejects development fixtures and their known sign/conversion
aliases. Every signed policy also passes `ProductionTrustCandidate` validation.
The bootstrap key must be separate from all four admission roles, including
sign-negated signing identities and conversion into the recipient key. The
record contains a selector, never an authority to choose a verifier's key.

## Acceptance state

There is no default or record-derived floor. `PolicyExpectation` has two explicit
constructors:

- `enrollment(minimum_sequence)`: the floor must be nonzero; the record must
  meet it and have a zero predecessor. This is initial provisioning only.
- `installed(sequence, digest)`: both fields must be initialized. Older
  revisions reject. A record at the current revision must match the exact
  accepted digest. A new revision must be the immediate successor and name
  that accepted digest as predecessor. Gaps, forks against an already installed
  revision and revision wraparound reject.

The public cluster and expectation must come from the same protected enrollment
context as the bootstrap key. After a successful check, `VerifiedPolicy` exposes
the public candidate and digest; it does not install authority, write persistent
state, decrypt credentials or authorize a boot.

Two operator-signed successors can pass against the same old expectation. A
future installer must serialize **verification, protected state commit and
authority publication**. Once one successor is committed, the other conflicts
with its exact digest. Persisting a plain file, merely recording a sequence, or
reusing a stale expectation concurrently does not provide that guarantee.
Failure to read or update installed state must fail closed; it must never
silently retry as enrollment. Bootstrap-key rotation/revocation needs a separate
protected-anchor transition and is not implemented by this wire format.

## Exact boot selection

`verify_for_boot` uses the same signature, key, cluster and lineage checks, then
requires the exact installed sequence and digest. An enrollment expectation
rejects with `InvalidExpectation`. A valid direct successor rejects with
`UninstalledRevision`; rollback, conflicting digests and malformed records retain
their existing errors. No failure retries verification under a different mode.

Success returns `VerifiedBootPolicy`. Its fields are private, and there is no
conversion from the broader `VerifiedPolicy`. The kernel handoff accepts this
boot receipt, so enrollment/rotation verification cannot be passed to it directly.
For an adapter with independently authenticated and serialized inputs:

```rust
let installed = PolicyExpectation::installed(sequence, accepted_digest)?;
let boot = signed_policy::verify_for_boot(record, &bootstrap, &cluster, &installed)?;
```

This function reads or writes no storage. Caller-supplied state remains a trust
input; constructing an expectation from candidate bytes does not prove
installation. A future protected adapter must retain the installation/publication
fence through recipient acquisition and one-shot kernel publication. The receipt
does not establish that fence, key custody or rollback resistance.

## Kernel handoff

`service::admission::PreparedBootTrust::verified` consumes a `VerifiedBootPolicy`
and a `Zeroizing<[u8; 32]>` recipient-key owner. It checks the launch cluster
and derived recipient public key before publication. Failure wipes the
unpublished private owner; it never selects development trust. The separate
`development` constructor is used explicitly by today's development boot.

After memory initialization, the BSP selects the preparation into the existing
boot slot before APs or platform services start. The slot retains its zeroizing
recipient owner and immutable public policy/digest. Platform ELF verification,
store validation and persistent-image checks use that selected artifact key;
missing selection rejects without a development-key fallback.

Service composition takes the preparation once, leaving its public authority
intact. Publication checks that same policy/digest and consumes the owner into
the installed slot, returning a private-field `BootTrust` view. A second
publication rejects, including the same policy, a signed replacement or development trust, and returns its complete
preparation owner after unlocking. There is no runtime replacement/reset API.
Policy rotation therefore requires a future protected restart/installation
boundary; it cannot overwrite an already running node's authority.

DNS, clusterctl and agent manifests use that published view. The final kernel
deployment/operations gate reads the same slot, which also captures the
supervisor's exact name-service domain handle and bootstrap endpoint. Callers
cannot configure a manifest policy independently of the gate. The private key
stays in kernel-owned zeroizing storage; each operational callback gets a
zeroizing temporary copy after the policy guard is released.

`launch_steady_state_with_trust(cluster, prepared)` is the consuming service
composition entry point for a future trusted boot adapter. It rejects a foreign
cluster before publication/service composition and returns the preparation owner
when publication is already complete. Today's scheduler boot wrapper takes the
early-selected preparation and passes it to that same composition function.
Selection alone exposes neither operational policy nor recipient callbacks.

This handoff authenticates no platform input by itself. Its future trusted
caller must authenticate the executable chain and enrollment context, commit
rollback-resistant state and obtain the private owner from custody before
calling it. A receipt checked against attacker-selected root/state cannot
establish those conditions. Initial platform service loading now follows the
selected policy artifact root, including the name service before final
publication. There is still no production launch selector. The captured
name-service handle also does not replace shutdown/root leases.

The [disposable x86 QEMU Secure Boot fixture](../guides/qemu-secure-boot.md)
now exercises signed EFI/config/kernel/module loading and the kernel handoff.
Its boot selector accepts only the exact compiled installed test revision/digest;
an otherwise valid successor cannot publish before its state is installed.
That check now uses the shared `verify_for_boot` boundary instead of a separate
fixture-local assertion.
For restart/rotation qualification, the x86 development fixture can compile a
different complete signed snapshot selected through
`CATTEN_BOOT_TEST_INSTALLED_POLICY`. Boot authenticates that snapshot against the
pinned test anchor; a separately supplied candidate still must match its exact
revision/digest. The [rotation campaign](../reports/audits/2026-10-27-security-management-rotation.md)
advances public fixture state before rebuilding and observes older signed-kernel
replay restoring old authority. This selectable input adds no protected state or
production installation path.
The fixture uses public test enrollment/state/recipient material and an isolated
platform bundle signed by its selected public test artifact root. Images signed
by the development key reject at the name-service/NVMe loader gate. It is not a
production boot selector,
persistent-state adapter or custody provider.

## Protected integration still required

SEC-04 remains partial until an adapter provides all of these boundaries:

1. Authenticate the executable boot chain/verifier and pin its bootstrap key
   and cluster through protected platform enrollment.
2. Read rollback-resistant acceptance state, verify this record, and commit its
   exact revision/digest atomically before publishing its role authority. Recover
   power loss without accepting either an older state or a half-installed policy.
3. Connect protected installation to early authority selection and the one-shot
   kernel handoff. The common platform-loading path now uses the selected key;
   production still needs authenticated, non-fixture inputs and a fence through
   loading/publication. Remove development roots from that production path.
   A public-key validation result or host-file verification alone cannot create
   that authority.
4. Bind recipient-key custody/release to the authenticated platform and policy;
   keep development recipient material excluded. This record contains no private
   material and implements no sealing/unsealing or custody provider.
5. Reject substituted executable images, root/policy replacement, stale or
   missing state, concurrent/forked installation, custody failure and interrupted
   commit in the selected QEMU platform before physical qualification.

This format authenticates public policy only; it does not sign the kernel image,
firmware or bootloader. It has no UTC expiry field, and does not resolve SEC-09's
unauthenticated time. The [remediation criteria](security-remediation.md) retain
the broader production and quiescence gates.

Usage: [signing and trust guide](../guides/signing-and-trust.md#signed-trust-policy).

## Formal ordering projection

[`CharlotteBootTrust`](../tla/CharlotteBootTrust.tla) checks current verifier,
fixture and one-shot handoff behavior separately from a **conditional** protected
installation contract. The latter assumes atomic rollback-resistant state and
trusted platform/custody inputs; those boundaries remain unimplemented. It
covers stale competing expectations, failed/interrupted commit, qualified reads
after recovery has settled/fenced original writers, and reboot before/after commit, without turning verification into
installation authority. The [model review](../reports/audits/2026-10-18-boot-trust-model.md)
and [early authority follow-up](../reports/audits/2026-10-19-security-platform-artifact-authority.md)
record twelve required counterexamples, source correspondence and limits.
Production remains disabled; a passing contract instance is no enrollment,
protected-storage, custody or physical-lifetime qualification.
