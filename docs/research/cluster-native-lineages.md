# Cluster-native research lineages

## Purpose and status

This note reconnects the current CharlotteOS cluster implementation to prior
and current operating-systems, distributed-systems, cluster-management,
networking, software-supply-chain, and workload-identity research. It
complements the broader historical survey in
[Related Operating-Systems Research](related-systems.md).

The comparison follows four questions:

1. Which earlier mechanism or research claim is relevant?
2. What correspondence now exists in CharlotteOS code and tests?
3. Where does CharlotteOS deliberately take a different path?
4. Which experiment would distinguish a contribution from a relocation of
   existing complexity?

Implemented means that a bounded mechanism and direct tests exist in this
repository. It does not imply production scale, generality, refinement proof,
hardware qualification, or operational support.

## Research thesis after the cluster work

CharlotteOS now supports a more precise thesis than the earlier proposal of a
distributed capability system:

> Cluster orchestration can act directly on signed,
> capability-constrained execution objects. Declarative desired state,
> reconciliation, scheduling, and health management remain necessary, while
> some machinery used to turn a general-purpose machine process into a
> controlled cluster workload may change or disappear.

The implemented path joins:

- dynamic Raft membership and replicated deployment state;
- signed releases and deterministic replica placement;
- central S3 retrieval of digest-pinned execution objects;
- per-node exact-generation readiness;
- capability grants derived from signed policy;
- DSR ingress whose eligible backend set comes from placement and readiness;
- independently authorised, HPKE-encrypted Kafka and S3 connector profiles;
- bounded remote invocation and transactional Kafka steps; and
- generation-fenced drain and cooperative shutdown.

The individual mechanisms have established ancestors. The research interest is
their composition into one authority and lifecycle model.

## Current mechanism map

| CharlotteOS mechanism | Closest lineages | Present distinction | Remaining evidence |
|---|---|---|---|
| Cluster as administrative unit | Amoeba, Solaris MC, LegoOS, Borg/Kubernetes | Native signed execution objects replace per-node application installation | Larger deployments, resource accounting, automated recovery |
| Desired state and placement | Raft, Borg, Omega, Kubernetes controllers, CRUSH | The same committed state governs replica ownership, generation, readiness, and grants | Capacity, trust, health and failure-domain constraints; rolling policy |
| Cluster service identity | Location-independent RPC, rendezvous hashing, Maglev, DSR | Application placement and exact readiness directly form the VIP backend set | Multiple services/VIPs, hostile-L2 protection, scale and churn measurements |
| Remote service invocation | Amoeba, Birrell/Nelson RPC, Waldo et al. | Common typed service model with explicit remote failure and uncertain outcomes | General distributed delegation/revocation and durable deduplication |
| Role-separated release trust | TUF, Uptane, in-toto, Sigstore/SLSA | Development authorises behavior; operations separately authorises one environment binding | Key rotation, revocation, audit, recovery and hardware-rooted custody |
| Connector-confined managed services | Object-capability proxies, SPIFFE/SPIRE, service brokers | Applications receive attenuated operation endpoints while credentials remain in Kafka/S3 connectors | Readiness-driven connector rotation and wider managed-service integration |
| Lifecycle and shutdown | MINIX recovery, Erlang/OTP, leases/fencing, Kubernetes reconciliation | Generation, grants, ingress drain and a signed shutdown grace share one retirement path | Full platform parity, failure injection and stateful cross-node recovery |
| Generated non-POSIX applications | Model-driven engineering, interface generation, AI-assisted migration | Durga retains process intent while generating Charlotte adapters and deployment contracts | Semantic preservation, provenance, maintainability and measured migration cost |

## 1. From distributed operating systems to a cluster operating system

### Lineage

[Amoeba](https://www.cs.vu.nl/pub/amoeba/manuals/usr.pdf) treated processors as
a pool and made capability-protected objects available through
location-independent RPC. [Solaris
MC](https://www.usenix.org/conference/usenix-1996-annual-technical-conference/solaris-mc-multi-computer-os)
extended Unix into a cluster-wide single-system image while preserving the
Solaris ABI. [LegoOS](https://www.usenix.org/conference/osdi18/presentation/shan)
reopened the OS boundary for disaggregated processor, memory, and storage
components, while deliberately retaining common Linux system calls to ease
adoption.

Borg and its descendants took a different route. They made the cluster the
deployment target above commodity host operating systems. The
[Borg paper](https://research.google/pubs/large-scale-cluster-management-at-google-with-borg/)
describes admission control, placement, availability, naming, monitoring, and
declarative job specifications at production scale.

### CharlotteOS correspondence

CharlotteOS combines the processor-pool and cluster-manager views. A release is
assigned to a cluster. Nodes join replicated membership, provide resources,
receive concrete assignments, fetch immutable artifacts, and create protected
domains. Clients use logical names or a cluster VIP rather than the identity of
the machine that currently executes a component.

### Deliberate departure

CharlotteOS does not provide a failure-transparent single-system image and does
not preserve the Linux ABI. Remote calls retain deadlines, retries,
generations, and uncertain outcomes. Stateful movement requires an explicit
ownership-transfer protocol. This accepts the central warning from Waldo et
al.'s [A Note on Distributed
Computing](https://waldo.scholars.harvard.edu/publications/note-distributed-computing).

### Research question

The test is whether removing the machine-shaped application environment makes
cluster policy easier to inspect without merely moving comparable complexity
into Charlotte-specific services and tooling.

## 2. Desired state, reconciliation, placement, and readiness

### Lineage

[Borg, Omega, and
Kubernetes](https://research.google/pubs/borg-omega-and-kubernetes/) established
the modern cluster-control pattern: users declare intent, a scheduler chooses
placement, and controllers continuously reconcile observed state with desired
state. The [Kubernetes controller
model](https://kubernetes.io/docs/concepts/architecture/controller/) explicitly
describes these control loops.

CRUSH, introduced with
[Ceph](https://www.usenix.org/legacy/event/osdi06/tech/full_papers/weil/weil_html/),
is another useful influence. It turns a compact cluster map and placement rules
into deterministic object locations, with failure-domain structure and limited
movement after topology changes.

### CharlotteOS correspondence

The DNS-owned Raft catalog now commits releases, replica policy, concrete node
sets, generations, operational bindings, shutdown intent, and per-node
readiness. The leader resolves singleton, fixed-replica, and
every-eligible-node policy against admitted, non-draining voters. Affinity and
anti-affinity groups make component relationships part of placement. Node
agents reconcile all assigned deployments and publish readiness for the exact
generation they launched.

This makes placement an authority decision as well as a scheduling decision.
Only selected and ready owners can publish the service or enter the new-flow
backend set.

### Deliberate departure

CharlotteOS currently uses one authoritative controller path instead of Omega's
parallel optimistic schedulers. Deterministic placement and bounded records are
more important at the present scale than scheduling throughput.

### Research question

The next experiment must add node capacity, trust labels, named failure
domains, health, communication cost, and rolling availability constraints
without weakening deterministic reconciliation or capability policy.

## 3. Cluster service identity and DSR

### Lineage

Location-independent services date back to distributed object systems and
Amoeba's FLIP network. Modern load balancers preserve connection placement
under backend changes through deterministic hashing and retained flow state.
[Maglev](https://www.usenix.org/system/files/conference/nsdi16/nsdi16-paper-eisenbud.pdf)
is a production-scale example of consistent connection mapping across a
distributed software load-balancer fleet. CRUSH is a related precedent for
deriving placement from a stable map without storing one decision per object.

### CharlotteOS correspondence

CharlotteOS exposes a `VIP:port` as a cluster service identity. The frame
router takes an immutable projection of Raft membership, committed application
placement, exact-generation readiness, and drain intent. Rendezvous hashing
selects a ready backend for each five-tuple. A remote packet crosses one
cluster L2 hop unchanged inside a Charlotte envelope. The backend owns the TCP
state and returns traffic directly to the client.

Ingress leadership, execution placement, and the public service identity are
separate. Losing the VIP advertiser need not lose TCP state on surviving
backends.

### Deliberate departure

The first implementation has one configured IPv4/TCP service, bounded retained
epochs, no distributed connection tracker, and an administratively trusted L2
fabric. It optimises for a small, inspectable state machine rather than broad
load-balancer features.

### Research question

The novel systems question is whether a control plane can make network
eligibility a direct consequence of application authority and lifecycle. Tests
must explore churn, partitions, multiple services, policy changes, L2 attacks,
and the cost of maintaining stable flows at larger membership sizes.

## 4. Capability authority and managed-service connectors

### Lineage

KeyKOS, EROS, seL4, and Amoeba establish the object-capability rule that
possession of a protected reference conveys authority. SPIFFE addresses a
modern neighbouring problem: deliver short-lived workload identity from a
privileged local agent without embedding a bootstrap secret in each
application. The [SPIFFE Workload
API](https://spiffe.io/docs/latest/spiffe-specs/spiffe_workload_api/) identifies
the caller through its platform context and supplies the identity documents
selected for that workload.

### CharlotteOS correspondence

Applications start with no ambient network or name-service authority. Signed
deployment grants let `grantctl` mint only the named connections admitted for
the authenticated principal and generation. S3 access points constrain an
endpoint, bucket, prefix, and operation set. Kafka access points constrain a
broker pool, role, group, topic/partition routes, and transactional identity.

The connector receives network access and its secret profile. The application
receives an attenuated service capability. Several independently named
connectors can supply distinct roles and destinations to one application. The
generic Kafka-step service owns the transaction and calls a separately
deployed business procedure.

### Deliberate departure

SPIFFE normally delivers identity material to a workload. CharlotteOS often
keeps the external identity in a connector and gives the application an
operation endpoint instead. The two models can complement each other: a future
connector could use a SPIFFE identity while still hiding its private material
from business logic.

### Research question

This arrangement should be evaluated as an authority firewall. Relevant
measurements include credential exposure, policy review effort, connector
reuse, revocation latency, and the effect of a compromised application or
connector.

## 5. Supply-chain trust and the development/operations boundary

### Lineage

[TUF](https://theupdateframework.github.io/specification/v1.0.28/) separates
metadata roles, versions, expiry, and threshold policy to survive compromise
of part of an update system. [in-toto](https://www.usenix.org/conference/usenixsecurity19/presentation/torres-arias)
binds software artifacts to verifiable supply-chain steps. Sigstore, SLSA,
Notary, and Uptane extend related ideas into cloud and safety-relevant release
workflows. [RFC 9180](https://www.rfc-editor.org/info/rfc9180/) supplies the
HPKE construction used to encrypt an operational profile to its destination
cluster.

### CharlotteOS correspondence

CharlotteOS assigns distinct meanings to its trust roles:

- an artifact signature identifies approved executable bytes and policy;
- a release signature binds components and their deployment descriptors;
- an operations signature binds encrypted Kafka or S3 configuration to one
  cluster, release, connector, sequence, and expiry; and
- an HPKE recipient key confines plaintext recovery to the privileged cluster
  launch boundary.

The leader re-verifies the complete admission proof against trusted UTC and
commits only compact references and replay fences. An assigned node retrieves
the digest-pinned ciphertext. The kernel repeats the trust checks, decrypts
into zeroizing memory, validates the connector-specific profile, and moves a
read-only memory capability into the new connector. Neither the application
nor Raft receives plaintext credentials.

### Deliberate departure

TUF and in-toto primarily protect distribution and provenance. CharlotteOS
uses those ideas as inputs to runtime authority construction. Development can
approve behavior without selecting production credentials. Operations can
bind infrastructure without replacing the application artifact.

### Research question

The production test requires organisational KMS or HSM custody, measured boot
or remote attestation, independent key rotation, redacted audit, recovery, and
readiness-driven connector replacement. Until then, the implemented mechanism
is a cryptographic foundation rather than a complete operational trust system.

## 6. Lifecycle, drain, and bounded shutdown

### Lineage

MINIX 3, CuriOS, and Pebble study isolated operating-system services and
recovery. Erlang/OTP makes supervisor-owned restart and explicit process links
ordinary application structure. Cluster controllers add readiness, desired
replica counts, drain, and replacement. Lease and fencing research supplies
the rule that an old owner must be unable to act after authority moves.

### CharlotteOS correspondence

CharlotteOS uses generation fencing from local endpoints through distributed
naming and placement. Exact-generation readiness gates grants and new DSR
flows. A signed shutdown grace travels with the deployment. Retirement first
stops admission, then requests cooperative teardown, and finally forces the
domain after the deadline. A signed node-shutdown intent removes the target
from new placement and ingress before local services and drivers stop.

### Deliberate departure

CharlotteOS promises a bounded and observable failure boundary rather than
transparent restart. A delivered external effect may require an idempotency
key, durable log, transaction, compensation, or an explicit uncertain result.

### Research question

The lifecycle path needs longer fault-injection runs, platform parity,
controller restart, rolling multi-component changes, stateful recovery, and
evidence that shutdown always preserves durable invariants before reporting
success.

## 7. AI agents, Durga, and the value of POSIX compatibility

### Historical constraint

Clean-break operating systems have repeatedly paid a severe adoption cost.
LegoOS explicitly retained the Linux system-call interface to ease adoption,
while Capsicum and container platforms add stronger isolation without giving
up the Unix software base. This remains decisive for opaque products and
software with deep native dependencies.

### Changed condition

Coding agents can reduce the human cost of identifying environment assumptions,
translating APIs, generating adapters, and constructing tests. The evidence is
promising and incomplete. The 2025
[CODEMENV](https://arxiv.org/abs/2506.00894) benchmark reports substantial
failure rates even for package-version migration. The
[FreshBrew](https://arxiv.org/abs/2510.04852) project-level benchmark likewise
emphasises semantic preservation and high test coverage rather than accepting
textually plausible changes.

### CharlotteOS correspondence

Durga provides a more constrained path than free-form source translation. A
BPMN process model and developer-owned business procedure can generate a
Charlotte adapter, capability requirements, resource declarations, build
inputs, replica and affinity policy, and deployment metadata. The generated
layer owns Charlotte-specific IPC and lifecycle mechanics while the business
rule remains testable on an ordinary development host.

### Research question

AI changes the economics of compatibility; it does not establish correctness.
The relevant experiment should compare:

- manual and agent-assisted effort to port or generate a service;
- semantic conformance against the original process model;
- provenance and reproducibility of generated artifacts;
- review effort for generated capability and resource requirements;
- defect rate across failure, cancellation, retry, and shutdown paths; and
- long-term maintenance as both the model and Charlotte ABI evolve.

This makes POSIX compatibility a measured engineering cost for source-available
applications rather than an automatic architectural requirement.

## 8. The proposed contribution

CharlotteOS does not claim a new consensus algorithm, capability theory,
software-supply-chain primitive, scheduler, or load-balancing hash. Its
proposed contribution is an end-to-end composition in which:

- desired placement determines execution ownership;
- readiness for one exact generation determines publication and new-flow
  eligibility;
- signed grants determine the local capabilities an application can acquire;
- independently authorised operational profiles determine how connectors
  reach managed infrastructure;
- applications receive connector operations instead of infrastructure
  credentials;
- remote failure remains explicit; and
- drain and shutdown withdraw authority in a bounded order.

This composition is useful only if it stays easier to inspect and operate as
functionality grows. The research programme should therefore treat complexity
as an empirical result, not a premise.

## 9. Falsifiable evaluation questions

1. Can an operator explain why a particular generation ran, received a grant,
   and accepted traffic from replicated records alone?
2. Does connector confinement reduce credential distribution and the impact of
   an application compromise?
3. Do deterministic placement and readiness-derived DSR converge correctly
   through membership change, partition, restart, and rolling replacement?
4. Can the system rotate an operational profile without exposing plaintext,
   granting the wrong generation, or interrupting unrelated applications?
5. Does bounded backpressure remain visible through a multi-service call chain
   under overload?
6. Can stateful ownership move once, recover after failure at every step, and
   prevent the previous owner from producing effects?
7. Does agent-assisted or model-driven generation reduce adaptation cost while
   preserving semantics and reviewable authority?
8. Which control and compatibility mechanisms genuinely disappear, and which
   reappear in Charlotte-specific form?

These questions reconnect implementation work to research by making every
architectural claim answerable through code, models, experiments, or operational
evidence.
