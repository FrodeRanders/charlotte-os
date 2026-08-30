# Deployment secrets and the development/operations boundary

CharlotteOS should let development teams define and sign application behavior
without learning production infrastructure credentials. The operations team
should be able to bind that behavior to managed Kafka and object-store services
without being able to replace the application executable. The cluster is the
run-time authority that joins those two independently authorized inputs and
hands applications only attenuated capabilities.

This document records the target model, the first implemented envelope, and the
work still required before encrypted operational bindings are admitted by a
cluster. It is not a claim that the full provisioning path is production ready.

## Separation of responsibilities

| Role | Supplies | Must not receive |
| --- | --- | --- |
| Development and CI | signed ELF, logical service requirements, protocol and authority shape | production broker/store credentials or cluster decryption keys |
| Operations | environment-specific connector profile, logical-name binding, expiry and rollout policy | artifact-signing private key or application memory |
| Cluster control plane | verification, policy intersection, placement, replay protection and capability grants | either off-cluster private signing key |
| Connector service | one read-only profile and broker/store-facing network authority | authority to grant itself to applications |
| Application | named, attenuated Kafka/S3 endpoint capabilities | connector profile, credentials, CA/client identity, or ambient network authority |

The artifact signature says *this is approved executable behavior*. The
operational signature says *this logical connector may use this environment
binding*. Neither statement is sufficient by itself. Cluster policy intersects
them and mints the capability actually delivered to the application.

Endpoint addresses are often not confidential, but they remain operational
configuration and may reveal network structure. Credentials, private keys and
tokens are confidential. The format treats the complete connector profile as
confidential so operators do not have to maintain two subtly different paths.

## Keys and cryptographic domains

Do not reuse the existing Ed25519 artifact key for encryption. Signing requires
the private key to remain with the signer and distributes the public key to
verifiers; decryption requires the private key to remain in the destination
cluster. Reusing bytes across algorithms also couples rotation and compromise
domains.

The planned hierarchy therefore has distinct keys:

- an artifact-signing Ed25519 authority held by development/CI;
- an operational-binding Ed25519 authority held by operations;
- an X25519 HPKE recipient key whose private half is available only inside the
  cluster's privileged provisioning boundary.

They may be issued and audited under the same organisational PKI or KMS, but
they are different keys with different policies. Production deployments should
ultimately seal the cluster recipient private key to measured boot, an HSM/KMS,
TPM, Arm CCA realm, or equivalent facility. A file-backed key is development
tooling, not the final custody design.

## End-to-end flow

```text
development / CI                         operations
  build ELF                                select Kafka/S3 service
  sign immutable artifact                 prepare connector-only profile
  declare logical requirements            bind cluster + exact release
          |                                encrypt to cluster HPKE key
          |                                sign with operations key
          +-------------------+--------------------+
                              |
                    cluster admission controller
                    verify both authorities
                    enforce sequence/expiry/policy
                    commit desired state
                              |
                    privileged secrets boundary
                    decrypt directly into bounded,
                    transient launch memory
                              |
                    connector gets profile;
                    application gets only a
                    role-attenuated capability
```

Transport TLS is still required where peer authentication, traffic-analysis
resistance or denial-of-service controls matter. Envelope encryption protects
the profile while it crosses storage, release tooling, ingress and replicated
state; TLS protects each live connection. They solve different problems.

## Implemented foundation: `COPSENC1`

`charlotte_launch::operations` now defines a bounded, `no_std` operational
profile envelope. `cluster-sign` can generate separate keys, seal a profile,
verify its signature and open it for development testing.

The version-one suite is RFC 9180 HPKE base mode with
X25519/HKDF-SHA-256/ChaCha20-Poly1305, followed by an Ed25519 operational
signature. The authenticated and signed context includes:

- a nonzero monotonic sequence and Unix expiry time;
- connector kind (`s3` or `kafka`) and printable logical profile name;
- a nonzero 32-byte cluster identity;
- the exact nonzero release-envelope SHA-256;
- recipient and operational signing key identifiers;
- all ciphertext and HPKE material.

Profiles are nonempty and at most 64 KiB; names are at most 256 bytes; lengths
are checked before allocation or decryption. Authentication failure zeroes the
caller-provided plaintext slice. The envelope deliberately contains no generic
key/value parser: after admission, the selected connector's existing bounded
profile decoder remains the format authority.

For a local tooling exercise, create distinct operational signing and cluster
recipient keys, then seal an existing connector profile:

```sh
cluster-sign operations-signing-generate ops-signing.key ops-signing.pub
cluster-sign operations-recipient-generate cluster-recipient.key cluster-recipient.pub
cluster-sign operations-seal kafka-orders.cops kafka/orders/transactional kafka \
  <cluster-id-hex> <release-sha256> 1 <expires-unix> \
  cluster-recipient.pub ops-signing.key kafka.profile
cluster-sign operations-verify kafka-orders.cops ops-signing.pub
```

Secret key files are created with mode `0600`; public files use `0644`. The
tool refuses to replace existing key files. Real key generation and envelope
creation should move behind organisational KMS/HSM APIs rather than exporting
private keys to a CI workspace.

`operations-open` exists to test interoperability and recovery policy. It
writes a newly created `0600` file. It is not the intended cluster interface:
the cluster must decrypt into transient owned memory and consume that memory in
a connector launch without publishing plaintext to a filesystem or ordinary
service API.

## Joining development and operations: `COPSBND1`

An operational envelope binds the SHA-256 of an exact, already signed
`CRELEASE`. Putting the operational-envelope digest back into that same release
would create a cryptographic cycle. CharlotteOS therefore does not extend
`CRELEASE1` with secret-profile references.

`charlotte_launch::operations_bundle` instead defines `COPSBND1`, a separate
operator-signed admission proof. It contains:

- the exact signed release;
- a nonzero monotonic revision of the complete operational set;
- the cluster and recipient identities;
- up to eight signed encrypted envelopes;
- for each envelope, the exact connector artifact receiving it and the opaque
  central-object-store key from which a node will later fetch it.

The outer operational signature prevents an intermediary from remapping a
valid profile to another connector or object key. Verification checks both
independent signatures, the cluster, recipient, exact release digest, expiry,
unique profile/target/object names, and that every target is a component of the
release. The bundle is bounded to 1 MiB and is an admission transport proof,
not replicated state.

`cluster-sign operations-bundle-sign` and `operations-bundle-verify` implement
the host-side format. Bundle creation takes target/object/envelope triples:

```sh
cluster-sign operations-bundle-sign orders.copsbundle 1 <cluster-id-hex> \
  <release-public-key-hex> ops-signing.key cluster-recipient.pub orders.crelease \
  kafka operations/orders-kafka.cops kafka-orders.cops
cluster-sign operations-bundle-verify orders.copsbundle <cluster-id-hex> \
  <release-public-key-hex> ops-signing.pub cluster-recipient.pub <now-unix>
```

The replicated catalog now has a version-ten compact representation for a
verified bundle. It stores the release, bundle digest and sequence, connector
target, profile name, object key, encrypted-envelope digest, expiry, key IDs,
and per-profile sequence. It does not store the large admission bundle,
ciphertext, or plaintext. Updates are atomic with the release deployment,
reject stale/conflicting sequences, retain inactive tombstones, retire bindings
when their release advances, and survive snapshots. Only a future trusted DNS
admission path may construct this compact command after full bundle
verification; ordinary clients cannot submit it directly.

## Admission and storage rules

The encrypted envelope can be uploaded to the central object store and named by
a release, or submitted directly to a bounded notification endpoint. Upload and
notification are transport choices; admission rules are the security boundary.
The cluster must:

1. verify the independently trusted development and operations signatures;
2. require the envelope's cluster ID and release digest to match admission
   context exactly;
3. reject expired, duplicate-conflicting and non-monotonic sequences;
4. authorize the logical requirement/profile-name binding;
5. store only compact ciphertext references and digests in Raft logs and
   snapshots, and never plaintext in diagnostics or crash dumps;
6. decrypt only on a node selected to launch the connector;
7. transfer the profile through owned, bounded, read-only launch memory, then
   zero the transient plaintext;
8. grant the application only the connector endpoint and role allowed by the
   signed deployment policy.

Operational rotation should install a new connector generation, prove it
ready, move grants, drain the old generation, and then retire its profile.
Rollback must not make an expired or lower-sequence binding admissible again.

## Delivery plan and status

1. **Envelope and host tooling — implemented foundation.** `COPSENC1`, HPKE,
   independent operational signing, strict bounds and codec tests exist.
2. **Role-aware trust registry — not implemented.** Replace the single embedded
   `CLUSTER_PUBLIC_KEY` assumption with separately identified artifact,
   deployment and operational verification authorities plus rotation policy.
3. **Release binding — implemented foundation.** `COPSBND1` avoids a digest
   cycle while joining one exact release to operator-signed encrypted profiles,
   connector targets and central-object-store keys.
4. **Replicated replay fencing — implemented foundation; network admission not
   implemented.** Catalog V10 atomically retains compact bindings, complete-set
   and per-profile sequence fences, retirement tombstones and snapshots. The
   ingress, follower relay and leader verification path still need the separate
   operations/recipient trust configuration and trusted UTC.
5. **Privileged decryption and launch — not implemented.** Add a small secrets
   service or controller-owned primitive that can use the cluster recipient key
   and move plaintext directly into an S3/Kafka connector launch.
6. **Rotation, recovery and audit — not implemented.** Define recipient-key
   rollover, loss recovery, generation replacement, redacted observability and
   KMS/measured-boot integration.

Until steps 2, 4's network path, and 5 exist, production connector profiles
remain separately provisioned as before. `COPSENC1`/`COPSBND1` must not be
described as accepted by `deployd` or automatically delivered to a connector.

## Review checklist

- Are artifact, operational-signing and recipient-decryption keys distinct?
- Is an envelope bound to one cluster and one exact release?
- Can operations change application bytes, or can development select production
  credentials? Both answers must be no.
- Does plaintext ever enter Raft, logs, status output, object keys or application
  IPC?
- Does every failure path zero and release transient plaintext and owned launch
  resources?
- Are sequence, expiry, key rotation and connector-generation rollback rules
  explicit?
- Does the application receive a role-scoped endpoint rather than ambient
  connector or network authority?
