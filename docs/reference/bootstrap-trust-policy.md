# Signed bootstrap trust policy

`charlotte_launch::trust::signed_policy` authenticates a fixed public admission
policy relative to an explicitly supplied bootstrap key, cluster and acceptance
state. Host signing and verification use the same allocation-free parser and
verifier intended for a future protected provisioning adapter.

This is policy authentication tooling, not a protected boot implementation.
There is currently no firmware/kernel installer for this record. Production
builds remain disabled; a verified record does not enable production or change
the compiled development roots or recipient key.

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

## Protected integration still required

SEC-04 remains partial until an adapter provides all of these boundaries:

1. Authenticate the executable boot chain/verifier and pin its bootstrap key
   and cluster through protected platform enrollment.
2. Read rollback-resistant acceptance state, verify this record, and commit its
   exact revision/digest atomically before publishing its role authority. Recover
   power loss without accepting either an older state or a half-installed policy.
3. Carry authenticated authority through kernel service/agent/cluster launch;
   remove development roots from the production path. A public-key validation
   result or host-file verification alone cannot create that authority.
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
