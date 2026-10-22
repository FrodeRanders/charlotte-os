# Signed deployment notification

CharlotteOS has a bounded off-cluster notification path for software already
placed in a centrally managed S3-compatible object store. The management
request carries a signed deployment descriptor, not the ELF and not object
store credentials.

## Operational flow

1. Build a self-contained CharlotteOS ELF and sign its CLS2 note with the
   offline artifact signing key. Pass its restricted file path to the signer,
   never its bytes; see [Signing and development trust](../guides/signing-and-trust.md).
2. Retain each final signed ELF for descriptor signing; tooling derives its architecture and SHA-256.
3. Upload those immutable bytes from CI or an operator workstation to the
   central RustFS, Dell EMC ECS, or other compatible store.
4. Create a `CDEPLOY6` descriptor with `cluster-sign deployment-sign`. It binds
   the logical artifact name, signed architecture/ABI/feature-specific binary variants, target node key,
   monotonic sequence, per-thread stack allocation in 4 KiB pages, maximum
   active-thread count, cooperative shutdown grace, placement policy, and
   named capability grants.
5. Submit it with `cluster-sign deployment-notify <descriptor> [IP:port]`.
6. Inspect the current named deployment with
   `cluster-sign deployment-status <artifact-name> [IP:port] [wait-seconds]`.
   Use `deployment-apply` below to bind a readiness wait to the generation
   returned by descriptor admission.

For a release containing several independently signed components, the host
tool can prevalidate every descriptor, submit them, and wait for the complete
set in one invocation:

```text
cluster-sign deployment-apply 127.0.0.1:8081 120 \
  ingest.cdep validate.cdep publish.cdep
```

Artifacts must already have been signed and uploaded. The command rejects a
malformed descriptor or duplicate artifact name before submitting anything,
then retains the positive generation returned for each accepted descriptor.
A complete readiness record must match that generation. Older observations
remain pending; a newer generation stops the command as superseded. Malformed
admission/status records fail without replaying the accepted descriptor. Status
records require the current scalar fields and valid counters; ready records
also require a nonempty fully ready replica set. Field order and JSON whitespace
are accepted; duplicate, unknown, missing or extended fields reject. See the
[client-generation audit](../reports/audits/2026-10-21-security-deployment-generation.md).

The normal network-and-storage service set starts `clusterctl`, `agent`, and
`deployd`. `deployd` listens on guest TCP port 7444. The QEMU runners forward
`${CATTEN_DEPLOY_HOST_PORT:-8081}` to that port under the default user network,
so the host tool defaults to `127.0.0.1:8081`.

All seven deployment HTTP controls use owning `deployment_control::Controls`
operations. Each submits its controller request once and binds that pending call
to the default CQ. The service continues accepting and parsing other inputs and
observing other controller calls while one reply remains outstanding. Listening,
input and controller phases share four positions. Response rendering, TCP send
and socket setup/close still run synchronously; the controller server itself can
also serialize work internally.

Each controller operation has one five-second deadline captured before request
validation/preparation. The reactor waits for the earliest input or RPC deadline
and rechecks all owners after wake. Timeout cancels its exact pending owner and
returns HTTP `503` with `{"error":-22}` (`clusterctl::ERR_RPC_TIMEOUT`). Accepted
mutations are never replayed. An accepted-call observation failure reports
uncertainty (-5), rather than a routing rejection. A controller may already have committed a mutation
or still own its moved request: cancellation is not remote rollback or a refund
of that receiver's original memory charge. Observe existing state before deciding
how to reconcile an uncertain outcome. Accepted-call CQ binding or wait rejection
aborts with the containing owners live.

Spare listeners are prepared before observing accepts. Input receives and
controller replies use actual producer notifications on the same CQ, including
peer EOF. A rotating cursor advances ready inputs; outstanding control calls
have their own rotating observation cursor. Each input has a five-second header
and thirty-second total deadline from observed accept completion. Progress and
unrelated CQ wakes do not restart these budgets. Rejected listener setup retains
a 50 ms admission backoff because it has no readiness source.

Retained prefixes and active controller request reservations share
`MAX_REQUEST_BYTES` (1,049,088 bytes including header allowance). A controller
reservation includes rounded moved-request pages and any retained release
receipt-validation envelope capacity. Admission rejects before allocating above
available budget; input growth cannot consume another operation's reservation.
The brief request-copy preparation, caller receive pages, returned reply memory,
allocator overhead and TCP buffers are separate. Ending a local operation does
not discharge uncertain remote backing charges. There is no protected operator
slot or per-peer fairness policy when all four positions are occupied.

The [input-isolation evidence](../reports/audits/2026-10-22-security-deployment-input.md)
records the first input milestone; the
[controller-reactor evidence](../reports/audits/2026-10-22-security-deployment-control.md)
records the next boundary. The earlier
[RPC deadline evidence](../reports/audits/2026-10-12-security-controller-deadlines.md)
qualifies the single-call wait helper separately.

For the current demonstration service, the final signing steps resemble:

```text
cluster-sign deployment-sign greet.cdep greet releases/greet-42.elf \
  <signed-artifact.elf> 0 42 4 1 5000 <private-key-file> \
  greet=publish
cluster-sign deployment-notify greet.cdep 127.0.0.1:8081
cluster-sign deployment-status greet 127.0.0.1:8081 120
```

For three distinct replicas, bless the ELF with
`FLAG_PARALLEL_INSTANCES`, use node key zero, and add placement options before
the grants:

```text
cluster-sign deployment-sign orders.cdep orders releases/orders-42.elf \
  <signed-artifact.elf> 0 42 8 8 15000 <private-key-file> \
  --replicas=3 --spread-replicas --anti-affinity-group=42 orders=publish
cluster-sign release-sign orders.crelease orders 42 <private-key-file> orders.cdep
cluster-sign release-apply orders.crelease 127.0.0.1:8081 120
```

A successful notification returns HTTP `202 Accepted` and JSON containing the
committed Raft deployment generation. All scalar signer notification commands
require the complete positive-generation acknowledgement; release uses its full
receipt. A malformed HTTP-202 body is uncertain and never causes POST replay.
The shared body reader preserves trailing data and Unicode whitespace for the
codec. See [tooling inputs](tooling-inputs.md).
Repeating the exact same descriptor is
idempotent. A lower signed sequence, or different signed bytes at the same
sequence, returns HTTP `409 Conflict`.

The `4 1 5000` between the deployment sequence and key-file path in the example means
four 4 KiB stack pages per thread and at most one active thread, including the
bootstrap thread, with five seconds to drain before forced retirement. Valid
`CDEPLOY6` execution values are 1 through 64; shutdown grace is zero through
300,000 milliseconds. All three values are signed and enforced exactly by the kernel; values outside those
ranges are rejected rather than clamped. A thread spawn beyond the signed
quota rejects before stack construction and returns `(u64::MAX, 0)`; the owned
runtime reports `ThreadError::SpawnFailed`, leaving the caller running. The release pipeline should take all three requirements from the signed,
developer-reviewed component plan. Only `CDEPLOY6` is accepted. Its exact
152-byte header must include execution limits, shutdown grace and placement;
older formats reject even with a valid signature. Regenerate old deployment
files and their containing signed release envelopes using the current tooling.

Ingress endpoints for all `cluster-sign` commands use numeric `IPv4:port` or
`[IPv6]:port` with a nonzero port; hostnames and URLs reject before connecting.
Apply captures one deadline before file preparation. Partial writes, interrupted
calls and response fragments consume the same budget, as do later descriptor
notifications and readiness queries. A timeout leaves submitted work unconfirmed
without replay. The status polling interval uses at most the remaining budget.
Blocking host filesystem calls and stdout are not preempted. See
[tooling inputs](tooling-inputs.md) for standalone command budgets and limits.

`deployment-apply` is currently release orchestration, not atomic bundle
admission. Each descriptor is a separate Raft entry, so a transport or policy
failure can leave an accepted prefix of the release committed. The client does
not replay accepted or uncertain notifications. Backend admission remains
idempotent for identical descriptors; reconcile the accepted prefix before
choosing an explicit retry. A future
signed process-bundle controller must still supply coordinated rollback and
the complete process-level policy described below.

## Atomic component releases

For all-or-nothing admission, wrap the independently signed descriptors in a
signed `CRELEASE` envelope and apply that envelope instead:

```text
cluster-sign release-sign orders.crelease orders 42 <private-key-file> \
  ingest.cdep validate.cdep publish.cdep
cluster-sign release-verify orders.crelease <public-key-hex>
cluster-sign release-apply orders.crelease 127.0.0.1:8081 120
```

The outer signature binds the release name, monotonic release sequence, and
exact ordered descriptor bytes. Every nested deployment signature is also
verified against the cluster key and must use the current `CDEPLOY6` grammar.
An envelope is limited
to 16 distinct artifact names and 3,584 bytes so it, its IPC envelope, and the
leader-resolved node assignments remain bounded.

`deployd` accepts the envelope at `POST /v1/releases`. The request can enter
through any member; a follower source-validates and correlates its relay to the
leader. The leader resolves every zero node key and signed placement policy,
then proposes all concrete replica sets as one Raft command. The replicated state machine preflights the release and
all component sequences while holding the deployment map, then changes either
all desired deployments or none. The commit result captures the release generation
and each component deployment generation in signed-envelope order. Exact retries
return a receipt only while every component still has those signed descriptor
bytes; a superseded component rejects without changing live state. Catalog
snapshots retain the signed release and replica records.

After atomic admission, node agents reconcile the desired records independently.
`release-apply` validates the complete `CRLADM01` receipt, bound to the exact
signed envelope digest and descriptor count. It retains each component's own
deployment generation. Older observations stay pending, matching ready generations
complete, and a newer generation fails as superseded. Malformed receipts or
HTTP-200 status records stop with the operation unconfirmed; the client does not
replay accepted notification. See [release admission results](release-admission.md)
for the IPC, relay and HTTP grammar and ownership boundary.

A fetch or launch failure can leave a release committed but not ready; automatic
rollback and rollout policy remain controller work. Apply carries one checked
monotonic deadline from preparation through admission and readiness; each network
step uses the remaining budget. Host polling, blocking filesystem calls and
ingress peer authentication remain separate boundaries.
`CRELEASE` binds executable deployment decisions, not
yet the BPMN digest, schemas, provenance graph, or other
semantic content of a complete Durga process bundle.

![Signed atomic release admission and rollout](../manual-v2/figures/release-admission-rollout.svg)

The request may enter through any cluster member. A follower's DNS validates
the reliable-message source as a current peer, relays the bounded request to
the current Raft leader, and correlates the committed result back to the HTTP
request. The release client therefore does not need to discover the leader.

The standalone `deployment-status` command validates the complete current
rollout record. With zero or omitted wait, it prints a valid pending or ready
observation; a positive wait completes on validated readiness. Malformed HTTP-200
records reject immediately. Invalid names reject before waiting or connecting.
The shared reader preserves every declared body byte after the first HTTP
separator and trims only JSON whitespace; extra records and Unicode whitespace
cannot supply a valid prefix to either status or apply. See
[tooling inputs](tooling-inputs.md). This command observes the current name,
without binding its result to an earlier admission.

`GET /v1/deployments/{percent-encoded-artifact-name}` reports `committed`,
`replacing`, or `ready`. Readiness is generation-safe: each ready count
represents a distinct desired node whose active distributed registration
carries the exact deployment generation. The JSON response includes
`desired_replicas` and `ready_replicas`; `ready` means the full desired set is
ready. A stale endpoint with the same logical name therefore cannot satisfy a
newer rollout.

A node key of zero asks the cluster to apply the signed placement policy. A
singleton prefers the current Raft leader and moves to a deterministic
fallback when it is draining; fixed replica counts and
`every eligible node` select from compatible, capacity-qualified non-draining voters. The leader
reconciles automatic sets after membership changes. A nonzero key remains an
explicit singleton pin. Replica policies must be submitted in a `CRELEASE`, so
the planner can honor cross-component affinity and anti-affinity consistently.
The descriptor name may use the complete
CLS2 limit of 48 bytes; it is no longer restricted by the old scalar-name ABI.

## Cluster ingress policy

Production VIPs are operations data rather than application-descriptor data.
The independent operations authority signs a complete `CINGPOL1` assignment
table, scoped to the cluster ID and a bounded UTC interval:

```text
cluster-sign ingress-policy-sign ingress.cing 7 NOT_BEFORE_UNIX EXPIRES_UNIX \
  CLUSTER_ID operations-private.hex \
  orders=10.0.2.42:443 payments=10.0.2.43:443
cluster-sign ingress-policy-verify ingress.cing CLUSTER_ID operations-public.hex
cluster-sign ingress-policy-notify ingress.cing 127.0.0.1:8081
cluster-sign ingress-policy-status 127.0.0.1:8081
```

`deployd` accepts the envelope at `POST /v1/ingress-policy`. A follower
source-validates and correlates the request while the leader verifies the
operations key, cluster ID, and validity interval against trusted UTC. The
complete replacement then enters Raft. Exact retries are idempotent; lower
sequences and conflicting reuse of a sequence fail. Use `--clear` in place of
the assignment list to sign an explicit empty-table withdrawal. The UTC range
bounds admission and replay of the envelope; a committed policy remains desired
state until a newer record replaces it.

The applied policy supersedes the runner's launch-time bootstrap table. DNS
materializes placement/readiness independently for every named service, the
frame router reconciles its leased epochs and flow tables by stable service
identity, and TCP/IP reconciles the node's `/32` VIP addresses. Catalog-v14
snapshots retain the envelope and replay generation. Applications do not
receive the operations key or production configuration; an application with
an attenuated DNS grant can poll `dns::ingress_assignments` and rebind an
exact-address listener after a change.

`GET /v1/ingress-policy`, wrapped by `ingress-policy-status`, reports the
catalog generation, signed sequence, assignment count, and admission validity
interval without exposing signing material.

## Trust and secret boundary

The HTTP listener is plaintext by design: the Ed25519-signed descriptor is the
authorization and integrity envelope and contains no secret. `clusterctl`
forwards only a structurally valid envelope; the DNS leader verifies its
signature and admission context before proposing state to Raft. Plaintext still
reveals deployment metadata and does not prevent connection-level denial of
service, so production networks should restrict the listener or place it behind
an authenticated TLS gateway when those properties matter.

The descriptor contains only an opaque object key. Endpoint addresses, bucket,
prefix, access credentials, Dell EMC ECS namespace, CA certificate, and TLS
client identity remain in the node's separately provisioned S3 profile. The
application receives neither that profile nor ambient name-service authority.

The encrypted operational-binding foundation now comprises `COPSENC1`, the
separate operator-signed `COPSBND2` admission proof, role-aware public launch
trust, host tooling, leader verification, follower relay and compact
replay-fenced catalog state. It binds an encrypted S3 or Kafka profile to a
cluster, exact release, target connector and central-object-store key. Submit a
bundle with `operations-bundle-notify` to `POST /v1/operations`. Admission fails
closed without trusted UTC and places neither ciphertext nor plaintext in
Raft. The assigned node agent now fetches the digest-pinned envelope through
its separately provisioned bootstrap S3 capability and moves a bounded pickup
package to a deployment-agent-only kernel gate. The kernel re-verifies the
signed release and exact descriptor membership, artifact identity, envelope
context and expiry, opens HPKE into zeroizing memory, validates the selected
S3/Kafka profile codec, and moves the profile read-only into the connector.
Plaintext never returns through agent or application IPC. The bootstrap S3
connector, production recipient-key custody, rotation and readiness-driven
replacement remain separate operational concerns. The trust model, format and
staged integration plan are in
[Deployment secrets and the development/operations boundary](../architecture/deployment-secrets-and-operations.md).

## Node-side pickup

Raft stores the signed descriptor with the assignment. The selected node agent
opens its local `s3` capability, streams the object, checks length and the
descriptor's SHA-256, verifies that CLS2 signs the ELF for the same artifact
name, and transfers both owning memory objects to the kernel. The kernel repeats
the checks and launches the ELF with only `grantctl` and the immutable
descriptor. An exact `publish` grant lets a service publish its endpoint
without obtaining name-service or mint authority.

## Reconciliation and current limits

Each node agent enumerates the replicated desired-deployment set. It can own
up to 64 independently named application domains, launches assignments for
its node, and fences each retirement by the stable artifact principal and
deployment generation. A `DeployedArtifact` owner in `catten_rt::owned`
retains the retirement obligation and requests best-effort retirement on
drop. The limit is a kernel admission bound, not a protocol-name limit.

- The bootstrap S3 connector and credentials must be provisioned before
  notification. It is also the privileged retrieval path for encrypted
  operational envelopes and cannot configure itself without a lower-level
  provisioning source.
- Automatic placement handles leader-preferred singletons, fixed replica sets,
  every-eligible-node policy, affinity/anti-affinity, and membership- or
  drain-driven rescheduling. Capacity-aware selection, named failure-domain
  spreading, multiple instances of one artifact per node, and rolling
  replacement remain future controller work.
- `scripts/run-aarch64.sh --deployment-ingress-test --timeout 240` automates a
  TLS RustFS upload → signed atomic release → S3 pull → scoped launch →
  readiness path.
  It is a development fixture, not a production release controller.

The fixture deploys the bundled `greet` artifact by default and accepts any
externally signed ELF through environment overrides:

```text
CATTEN_DEPLOY_NAME=<artifact> CATTEN_DEPLOY_ELF=<path> \
CATTEN_DEPLOY_OBJECT_KEY=deployments/<file>.elf \
CATTEN_DEPLOY_STACK_PAGES=<n> CATTEN_DEPLOY_MAX_THREADS=<n> \
CATTEN_DEPLOY_GRACE_MS=<ms> \
CATTEN_DEPLOY_GRANTS="<service>=client <artifact>=publish" \
CATTEN_APP_HOST_PORT=<host-port> CATTEN_APP_GUEST_PORT=<guest-port> \
CATTEN_APP_HOLD_SECONDS=<seconds> \
scripts/run-aarch64.sh debug --deployment-ingress-test --timeout 300
```

The ELF must already carry a CLS2 note from the cluster artifact key, since the
fixture signs the descriptor, not the binary. `CATTEN_APP_HOST_PORT` and
`CATTEN_APP_GUEST_PORT` add a SLIRP forward so a host client can reach the
deployed service; `CATTEN_APP_HOLD_SECONDS` keeps the guest alive after
readiness so that client can run before the runner exits. Out-of-tree projects
drive this through `charlotte-kafka-broker/tools/qemu-smoke.sh`, which packages
the artifact, deploys it, and runs the host smoke client end to end.

Mixed clusters select a committed variant per replica. Unknown profiles and
incompatible pins reject. See [execution targets](execution-targets.md) for
CPU requirements, multi-variant signing and format details.
