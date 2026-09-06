# CharlotteOS for production operations

Bullet outline for an IT, platform-engineering, SRE, operations, and security
audience. The implementation statements describe this repository. Production
claims are deliberately separated from the research direction.

## Slide 1: CharlotteOS for production operations

**Cluster-level execution, authority, and managed-service connectivity**

- A research operating system for purpose-built service clusters
- An operational model built around signed intent and explicit authority

## Slide 2: The offering

- CharlotteOS runs isolated services across a set of replaceable nodes.
- The cluster control plane manages signed software, placement, readiness, and
  lifecycle state.
- Each component receives only the capabilities granted by its signed
  deployment policy.
- Applications use cluster service identities while nodes supply execution,
  storage, and network resources.
- The repository demonstrates this model as a research prototype. It does not
  currently provide production support commitments or a general Linux
  application environment.

## Slide 3: What the model signifies

- The cluster becomes the primary administrative and computational unit.
- Deployment describes desired service state instead of a sequence of changes
  to named machines.
- Service identity remains stable when execution moves between nodes.
- Authority travels through typed capabilities rather than ambient network
  access or credentials copied into every workload.
- Operations can bind approved application behavior to a specific production
  environment without rebuilding that behavior.

*Suggested visual: [System layering](../manual-v2/figures/system-layering.svg).*

## Slide 4: The operational state model

Operations can reason from a small set of explicit records:

- **Membership:** which nodes belong to the cluster and which are draining.
- **Release:** which signed component versions form one admitted change.
- **Placement:** which eligible nodes should run each replica.
- **Readiness:** which exact component generation is ready on each selected
  node.
- **Authority:** which named service capabilities each component may receive.
- **Observation:** whether the running system is healthy and converging toward
  the committed state.

Raft replicates the authoritative cluster records. Node agents reconcile local
execution with them.

## Slide 5: Why the cluster is easier to reason about

- One committed desired state replaces independent per-machine deployment
  histories.
- Immutable artifact digests identify the exact code selected for execution.
- Generation fencing prevents an old instance from impersonating or removing
  its replacement.
- Exact-generation readiness controls publication and new network flows.
- Bounded queues and messages make overload visible as backpressure.
- Linear capability ownership gives resource allocation, transfer, and cleanup
  one auditable path through application and platform code.

The resulting incident questions are concrete: what was admitted, where was it
placed, which generation became ready, and which authority did it receive?

## Slide 6: Development and operations responsibilities

**Development and CI**

- Build and sign immutable application artifacts.
- Define business behavior, schemas, and logical service requirements.
- Propose resource needs such as stack size, thread limits, and shutdown grace.
- Remain outside production broker, object-store, and cluster decryption keys.

**Operations**

- Select production Kafka, S3, TLS, and identity bindings.
- Approve placement, replica policy, grants, expiry, and rollout policy.
- Sign the environment-specific operational binding with a separate authority.
- Remain outside application signing keys and application memory.

**CharlotteOS**

- Verify both authorities and intersect their policy.
- Place the release and launch the selected connector and application
  generations.
- Mint only the capabilities admitted by the combined policy.

*Suggested visual: [Role-separated deployment trust](../manual-v2/figures/role-separated-deployment-trust.svg).*

## Slide 7: Four kinds of configuration

**Application definition**

- Business parameters, data schemas, and logical dependency names
- Owned by the application team and testable away from production

**Runtime contract**

- Resource bounds, capability requirements, shutdown grace, and component
  relationships
- Proposed with the application and admitted through signed deployment policy

**External-service binding**

- Kafka brokers, topics, consumer groups, TLS/SASL/mTLS identity
- S3 endpoint, bucket, prefix, credentials, and trust anchors
- Owned by operations and delivered only to connector services

**Cluster policy**

- Membership, replica placement, affinity, ingress VIPs, drain, and release
  admission
- Owned by the cluster operator and enforced by the control plane

Application code sees its business configuration and granted endpoints. It
does not receive infrastructure addresses, credentials, or an unrestricted
name-service connection.

## Slide 8: Managed services through connectors

- Kafka and S3 connectivity runs in separately deployed connector services.
- A connector receives a bounded, read-only profile plus the network authority
  needed to reach the managed service.
- An application receives a narrower endpoint capability, such as S3 access to
  one prefix or Kafka production through an approved route set.
- Independently named connector instances let one application use several
  topics, roles, stores, or environments.
- The transactional Kafka step owns poll, procedure invocation, production,
  offset inclusion, commit, retry, and dead-letter handling.
- Business procedures remain ordinary request/reply components and never hold
  Kafka transaction credentials.

This division makes the connector a governed platform product. Application
teams consume its capability contract rather than reimplementing authentication
and broker lifecycle logic.

*Suggested visual: [External service capabilities](../manual-v2/figures/external-service-capabilities.svg).*

## Slide 9: Secrets and environment bindings

- Operations encrypts each connector profile to the cluster's X25519 HPKE
  recipient key and signs it with an independent Ed25519 operations key.
- The envelope binds the profile to one cluster, exact release, connector,
  sequence, and expiry.
- A central object store may carry the encrypted envelope and immutable
  application artifacts.
- Raft stores compact references, digests, and replay fences rather than secret
  plaintext or full ciphertext.
- The kernel re-verifies the release and operational proofs, decrypts into
  transient zeroizing memory, and transfers a read-only profile to the
  connector.
- Applications, logs, status pages, and ordinary cluster IPC never receive the
  plaintext profile.

Production use still requires organisational KMS or HSM custody, measured boot
or equivalent key protection, rotation, recovery, and audited connector
generation cutover.

*Suggested visual: [Operational profile pickup](../manual-v2/figures/operational-profile-pickup.svg).*

## Slide 10: Release and deployment flow

1. CI builds and signs self-contained architecture-specific artifacts.
2. CI uploads immutable artifacts to the centrally managed object store.
3. Development supplies the signed release and logical requirements.
4. Operations encrypts and signs the environment-specific connector profiles.
5. Operations submits the bounded release proof to any cluster member.
6. The Raft leader verifies both authorities, trusted UTC, expiry, sequence,
   and digests.
7. Raft commits desired placement and compact operational references.
8. Selected node agents fetch, verify, and launch the exact generations.
9. The grant controller gives each application only its admitted endpoints.
10. Exact-generation readiness makes the service available.

*Suggested visual: [Release admission and rollout](../manual-v2/figures/release-admission-rollout.svg).*

## Slide 11: Placement and cluster network identity

- Signed policy supports singleton, fixed-replica, and every-eligible-node
  placement, with basic affinity and anti-affinity.
- The leader resolves policy against admitted, non-draining members and
  commits the concrete replica set.
- New flows become eligible only after a selected node reports readiness for
  the exact deployment generation.
- A service VIP gives clients one stable cluster address without exposing the
  node that admits a packet or runs the service.
- Direct Server Return selects a ready backend while leaving TCP state at that
  backend. Replies travel directly to the client.
- Node addresses remain available for diagnostics and deliberately
  node-specific protocols.

Capacity, trust labels, named failure domains, health-driven replacement, and
rolling surge or unavailability policy remain future scheduler work.

*Suggested visual: [Cluster management and DSR](../manual-v2/figures/cluster-management-dsr.svg).*

## Slide 12: Lifecycle, drain, and shutdown

- Readiness gates capability publication and new VIP flows.
- A committed drain removes the node from new-flow selection before teardown.
- Retained DSR epochs preserve observed connections when their backend remains
  available.
- Every deployment carries a signed cooperative-shutdown grace period.
- Services receive a shutdown request, stop admission, finish or abort bounded
  work, flush durable state, and acknowledge completion.
- The kernel forces retirement when the signed deadline expires.
- Generation fencing ensures that late cleanup from an old instance cannot
  affect its successor.

The shutdown path is implemented across the main AArch64 lifecycle and service
set. Complete x86-64 runtime validation and some device-specific teardown paths
remain open work.

*Suggested visual: [Cooperative deployment shutdown](../manual-v2/figures/cooperative-deployment-shutdown.svg).*

## Slide 13: Operational questions have explicit answers

- **Who approved this executable?** The artifact and release signatures.
- **Who approved this production binding?** The independent operations
  signature.
- **Why is the component running here?** The committed replica assignment.
- **Why can it receive traffic?** Exact-generation readiness and current
  ingress eligibility.
- **What can it access?** The capabilities in its admitted grant set.
- **Where are its credentials?** Inside the connector's protected profile.
- **What happens to an old instance?** Generation fencing closes its authority
  and prevents stale publication.

These answers come from verifiable cluster state rather than a reconstruction
of machine histories, copied secrets, and administrator actions.

## Slide 14: Relationship to Kubernetes and OpenShift

- Kubernetes and OpenShift operate a broad Linux application ecosystem and
  provide mature controllers, networking, storage, policy, and operational
  integrations.
- CharlotteOS starts with a smaller, purpose-built userspace and makes
  capabilities, deployment identity, and cluster placement native operating
  system concepts.
- The narrower contract can shorten the chain between policy and enforcement.
- The cost is application adaptation, new tooling, and a much smaller hardware
  and software ecosystem.
- Durga explores code generation from process models so business logic can
  remain conventional while generated adapters handle Charlotte mechanics.

The research question is whether a narrower cluster operating system can reduce
configuration drift and ambiguous authority enough to justify that tradeoff.

## Slide 15: AI changes the compatibility calculation

- POSIX compatibility historically avoided a large amount of manual porting
  and preserved access to mature libraries, tools, and operational knowledge.
- Coding agents can now help identify platform assumptions, translate APIs,
  generate adapters, and build tests for a different execution contract.
- This lowers the cost of evaluating a clean-break platform for applications
  whose source and behavior the organisation controls.
- Durga provides an early concrete example: a process model and business
  procedure can generate Charlotte adapters, capability requirements, build
  inputs, resource declarations, and deployment metadata.
- Generated output still requires reproducible builds, review, provenance,
  security analysis, and runtime validation.
- Closed-source products, large native dependency graphs, and software with
  deep Linux assumptions retain a strong compatibility requirement.

For operations, the question can move from “does the existing binary run
unchanged?” toward “can the required behavior be transformed into a governed,
verifiable service contract at acceptable cost?”

*Suggested visual: [Durga to Charlotte generation](../manual-v2/figures/durga-charlotte-generation.svg).*

## Slide 16: Current implementation and production gap

**Implemented and exercised**

- Capability-isolated services, linear resource ownership, and userspace
  drivers
- Raft membership, signed releases, deterministic replica placement, and
  exact-generation readiness
- Cluster VIP ingress tied to committed placement and readiness
- S3 and Kafka connectors with TLS, Kafka authentication, role attenuation,
  consumer groups, and transactions
- Role-separated encrypted operational profiles and privileged connector
  launch
- Cooperative deployment and node shutdown foundations

**Required for production operation**

- Hardware-rooted key custody, trust rotation, recovery, and admission audit
- Production administration authentication and authorization
- Automatic failed-member removal and health-driven replica replacement
- Capacity and failure-domain-aware scheduling with rolling update policy
- Unified monitoring, logging, tracing, backup, and disaster recovery
- Sustained fault injection, performance characterization, and physical
  hardware qualification

## Slide 17: A useful IT evaluation

- Select one non-critical stateless service whose data plane already uses
  managed Kafka or S3.
- Keep the business procedure small and place connectivity in Charlotte
  connectors.
- Establish separate development and operations identities in a non-production
  KMS.
- Exercise deployment, node drain, application replacement, credential expiry,
  broker failover, object-store failure, and forced shutdown.
- Compare operator steps, secret distribution, recovery ambiguity, and
  authority breadth with the current platform.
- Define production gates from observed gaps rather than assuming research
  mechanisms already provide a supported service.

The evaluation tests a concrete proposition: a cluster becomes more
understandable when desired state, executable identity, infrastructure binding,
and runtime authority share one explicit control model.
