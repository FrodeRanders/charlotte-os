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
5. store only ciphertext in Raft logs, snapshots, diagnostics and crash dumps;
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
3. **Release binding — not implemented.** A future release version must name
   exact operational-envelope digests without putting plaintext profiles in
   `CDEPLOY1` or `CRELEASE1`.
4. **Cluster admission and replay fencing — not implemented.** Extend ingress,
   Raft desired state and the planner with atomic two-authority validation,
   expiry and per-binding monotonic sequence enforcement.
5. **Privileged decryption and launch — not implemented.** Add a small secrets
   service or controller-owned primitive that can use the cluster recipient key
   and move plaintext directly into an S3/Kafka connector launch.
6. **Rotation, recovery and audit — not implemented.** Define recipient-key
   rollover, loss recovery, generation replacement, redacted observability and
   KMS/measured-boot integration.

Until steps 2–5 exist, production connector profiles remain separately
provisioned as before. `COPSENC1` must not be described as accepted by
`deployd`, replicated by the controller, or automatically delivered to a
connector.

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
