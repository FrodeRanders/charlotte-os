# Cluster artifacts, blessing, and placement

This note records the trust and placement model implemented on the
`cluster-deploy-demo` branch, and separates it from the remaining cluster
vision in manual Chapter 19.

## Artifact admission

Cluster nodes do not build software or resolve package dependencies. A host
build produces self-contained, architecture-native ELFs in separate AArch64
and x86-64 bundles. Every staged name must occur exactly once in
[`artifact-policy.tsv`](../../crates/catten-services/artifact-policy.tsv), then
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

## Signed deployment descriptors

The ELF signature and the deployment decision are different trust statements.
An ELF signature binds code to an artifact name. A `CDEPLOY1` descriptor binds
that artifact's complete SHA-256 to an opaque central-object-store key, a
monotonic deployment sequence, a selected node, and a bounded list of named
`SEND`/`CALL` capability grants. The descriptor is separately signed by the
offline cluster Ed25519 authority. Tampering with placement, an object key, or
a grant therefore fails verification even when the referenced ELF remains
validly signed.

Descriptors contain no object-store endpoint, bucket credentials, Kafka
credentials, or TLS client identity. The object key is interpreted through a
locally configured S3 connector whose capability and secret profile remain in
the platform service. This makes the intended management-plane flow:

1. CI builds and signs a self-contained ELF off-cluster.
2. CI uploads the immutable ELF to the centrally managed object store.
3. CI signs a small deployment descriptor referring to the object's key and
   digest, then notifies the cluster with that descriptor.
4. A node pulls through its preconfigured S3 capability, verifies both
   signatures and the digest, and launches the application.

`tools/cluster-sign deployment-sign` and `deployment-verify` implement the
canonical bounded wire format used by the kernel and userspace. For example:

```text
cluster-sign deployment-sign orders.cdep orders releases/orders-a5.elf \
  <artifact-sha256> 0x1234 7 <private-key-hex> \
  kafka/orders/input=call kafka/orders/output=client
```

The private key stays off-cluster. Nodes and `grantctl` hold only the public
verification key.

## Capability-scoped application launch

The scoped launch API consumes a signed ELF and signed descriptor together. It
checks that the descriptor's artifact name and digest match the ELF, copies the
descriptor into a read-only launch profile, and gives the application only a
connection to the node's `grantctl` endpoint. It does not give the application
a name-service connection.

For each acquisition, the application uses the owned
`catten_services::grant_client` helper. `grantctl` verifies the descriptor,
binds its artifact name to the kernel-authenticated caller principal, rejects
stale or conflicting descriptor revisions, and checks the exact named grant.
It then uses its private name-service connection to obtain re-delegable
authority and replies with only the requested `SEND`/`CALL` rights. The
temporary re-delegable connection is owned and closed by the controller.

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

## Initial block-device image

`scripts/make-nvme-image.py` writes the blessed bundle into a version-3 object
store. AArch64 embeds the small bootstrap set needed to reach the object store
and reads other service bytes from it on first use. The x86 parity suite
currently embeds its complete tested service set and also stages the same
signed artifacts in the persistent image. The historical script name does not
restrict that image to NVMe: the x86 runner can attach it through NVMe, AHCI,
or virtio-blk.

`scripts/run-aarch64.sh` and `scripts/run-x86_64.sh` fingerprint every blessed
ELF. They recreate an instance image when the fingerprint changes, accept
`--fresh-storage` to force recreation, and require `--reuse-storage` to retain
a stale service store deliberately. The producer detects an object-id
collision within the bundle. The current 48-bit derived name-id remains a
prototype limitation; a production catalog needs collision resolution or full
content addressing.

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

The signed descriptor, grant-controller service, owned application helper, and
scoped kernel launch primitive are implemented. The existing `clusterctl`
upload operation and demo deployment agent still use the older local-object
copy and unscoped launch operation. Wiring descriptor notification to an
authenticated network management endpoint, pulling the referenced ELF through
the central S3 connector, and replicating the descriptor to the assigned node
remain the next integration step. Until that is complete, the new path is an
internal launch API rather than a complete off-cluster deployment product.
