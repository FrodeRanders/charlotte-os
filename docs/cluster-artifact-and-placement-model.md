# Cluster artifacts, blessing, and placement

This note records the trust and placement model implemented on the
`cluster-deploy-demo` branch, and separates it from the remaining cluster
vision in manual Chapter 19.

## Artifact admission

Cluster nodes do not build software or resolve package dependencies. A host
build produces self-contained AArch64 ELFs. Every staged name must occur
exactly once in
[`artifact-policy.tsv`](../crates/catten-services/artifact-policy.tsv), then
`tools/cluster-sign` writes a CLS2 `SHT_NOTE` record and signs the resulting
ELF with the off-cluster Ed25519 private key.

CLS2 signs these fields together with all other ELF bytes:

| Field | Purpose |
|---|---|
| logical name | prevents a signed artifact from being substituted under another store/service name |
| artifact class | distinguishes ordinary services, bootstrap code, drivers, and administration code |
| release and rollback counter | provides a monotonic policy input for future update/rollback enforcement |
| policy flags | includes explicit permission for parallel instances, statelessness, and no runtime code fetch |
| signing-key id | prevents ambiguous interpretation under the wrong trust anchor |
| provenance digest | optionally links the ELF to a retained SBOM, source attestation, or build statement |
| Ed25519 signature | authenticates the metadata and the complete ELF, with only this field zeroed while hashing |

This does not certify that third-party code is correct. It does make the
admission decision explicit and reproducible. Once admitted, a component and
its bundled dependencies can be exchanged within the cluster as an immutable
object; nodes need not fetch executable dependencies from an Internet package
registry. Production policy still needs SBOM generation, vulnerability review,
reproducible builds, source attestations, and key custody.

## Enforcement points

Trust is checked more than once because the callers protect different
boundaries:

1. `clusterctl OP_UPLOAD` verifies the signature and requested logical name
   before modifying the object store.
2. The kernel service-store resolver verifies that the object found under a
   name was signed for that name before caching it.
3. A deployment record pins the full SHA-256 of the selected stored bytes, so
   replacing an object at the same name cannot mutate an existing generation.
4. The node agent verifies the digest, key, and name before invoking its
   delegated deployment syscall.
5. The kernel snapshots the memory object, repeats name/signature and ELF
   validation, and only then maps the exact ELF in a new address space.

The agent no longer impersonates the artifact with a hard-coded endpoint. The
spawned ELF registers its own endpoint; the agent publishes and supervises it.
On reassignment, the agent aborts and reclaims the deployed domain, which
closes the endpoint and drives generation-fenced distributed removal.

Only the reuse-safe address-space identity selected by the supervisor may use
the spawn/retire syscalls. Merely registering the name `agent` grants no
authority.

## Initial NVMe image

`scripts/make-nvme-image.py` writes the blessed bundle into a version-3 object
store. Only `ns`, `nvme`, `objstore`, `uart`, and `observe` remain embedded as
the bootstrap path. Other service bytes are read from the store on first use.

`scripts/run-aarch64.sh` fingerprints every blessed ELF. It recreates an
instance image when the fingerprint changes, accepts `--fresh-storage` to
force recreation, and requires `--reuse-storage` to retain a stale service
store deliberately. The producer detects an object-id collision within the
bundle. The current 48-bit derived name-id remains a prototype limitation; a
production catalog needs collision resolution or full content addressing.

## Placement contract

`charlotte_launch::placement::PlacementPolicy` represents:

- desired replicas (or every eligible node);
- maximum instances of the component on one node;
- minimum distinct nodes;
- an affinity group for co-locating tightly dependent components;
- an anti-affinity group for separation and availability.

Validation rejects concurrent instances unless CLS2 includes
`FLAG_PARALLEL_INSTANCES`. Co-location is therefore a joint decision between
the blessed component contract and cluster placement state, not an unchecked
operator override.

The distinction matters: dependencies between components may justify placing
one instance of each together, while replicas of the same component may need
separate failure domains. A policy can express both constraints rather than
treating “affinity” as one Boolean.

Agent lifecycle stages are synchronization state, not diagnostic text. The
agent publishes them to its shared status page with release stores and the
kernel verifier consumes them with acquire loads. During cross-node migration,
this guarantees that successful domain retirement becomes visible before the
agent exits, independent of which LP runs the verifier.

## Honest remaining boundary

The current Raft deployment map still contains one active node assignment per
artifact, and the distributed name catalog has one active owner per name. The
policy type and parallel-safety gate are implemented, but a placement
controller, replica-set assignments, multi-owner lookup/load balancing, and
observed-dependency migration are not.

Raft agreement also does not authenticate the caller that proposed a mutation.
Artifact validation prevents arbitrary unsigned code execution and the key
ceremony now accepts only the set-once build-time anchor, but deployment can
still be redirected as a denial of service through the raw DNS mutation
endpoint. A separately delegated cluster-administrator capability and a real
out-of-band management plane remain required.
