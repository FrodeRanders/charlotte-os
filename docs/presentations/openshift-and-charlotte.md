# OpenShift and CharlotteOS clusters

Bullet outline for an IT organisation that already operates or is adopting
OpenShift. The comparison treats OpenShift as a mature production platform and
CharlotteOS as a research alternative at the operating-system boundary.

## Slide 1: OpenShift and CharlotteOS clusters

**Two operating models for production services**

- OpenShift industrialises Linux application operation at cluster scale.
- CharlotteOS explores a cluster-native operating system with explicit
  capabilities and signed execution objects.

## Slide 2: Scope of the comparison

- OpenShift combines Kubernetes with an integrated Linux, security, networking,
  identity, build, registry, and operations environment.
- CharlotteOS is a research operating system for purpose-built services across
  replaceable nodes.
- Both approaches depend on desired state, reconciliation, scheduling,
  replicated control state, and readiness.
- They differ most in the execution substrate and in how authority reaches an
  application.
- The comparison concerns architecture and operational reasoning. CharlotteOS
  does not claim OpenShift's feature breadth, ecosystem, qualification, or
  support model.

## Slide 3: Different starting points

**OpenShift starts with general-purpose Linux applications**

- Applications commonly expect POSIX processes, filesystems, shared libraries,
  package contents, users, and freely addressable network interfaces.
- Containers assemble a reproducible unit while namespaces, cgroups, seccomp,
  Linux capabilities, and SELinux constrain the process environment.
- Kubernetes and OpenShift coordinate those units across machines.

**CharlotteOS starts with purpose-built execution objects**

- A component enters a separate address space as a signed, self-contained
  artifact.
- The component begins with no ambient filesystem, network, name-service, or
  device authority.
- The cluster grants typed capabilities for the exact services the component
  may use.

## Slide 4: The execution paths

**A typical OpenShift path**

- Workload declaration
- Pod and image specification
- Kubernetes scheduler and kubelet
- CRI-O and OCI runtime
- Namespace, cgroup, seccomp, SELinux, and network setup
- Linux processes on a host operating system

**The CharlotteOS path under investigation**

- Signed release and deployment descriptor
- Cluster placement decision
- Verified execution object
- Capability-constrained address space
- Charlotte kernel and delegated userspace services

OpenShift's path preserves compatibility with a vast software ecosystem.
CharlotteOS gives up that compatibility to shorten the path from admitted
policy to kernel-enforced execution.

*Suggested visual: [System layering](../manual-v2/figures/system-layering.svg).*

## Slide 5: Complexity that both systems must handle

- Distributed consensus and loss of quorum
- Placement, replicas, health, and recovery after partial failure
- Service discovery and network routing
- Durable data ownership, repair, backup, and disaster recovery
- Rolling change, compatibility, rollback, and state migration
- Secret creation, custody, delegation, expiry, and rotation
- Monitoring, logging, tracing, accounting, and audit

CharlotteOS changes where these responsibilities live and how they connect.
It cannot remove the intrinsic complexity of distributed operation.

## Slide 6: Complexity created by the Linux compatibility contract

OpenShift must turn a machine-oriented process environment into a controlled,
movable workload. This requires mature machinery for:

- OCI filesystem images and registries
- Container runtime and low-level launch integration
- Namespace and cgroup construction
- Linux capability, seccomp, and SELinux policy
- CNI network setup and CSI storage integration
- Host operating-system lifecycle and per-node platform components

CharlotteOS investigates how much of this machinery changes when isolation,
resource ownership, deployment identity, and cluster placement are native OS
concepts. Memory protection, accounting, policy enforcement, networking, and
storage remain necessary.

## Slide 7: Deployment objects and desired state

**OpenShift**

- OCI image plus Kubernetes resources such as Deployments, StatefulSets,
  Services, ConfigMaps, Secrets, and custom resources
- Controllers reconcile the declared resources with Pods and node state.
- Image digests, rollout revisions, labels, selectors, and owner references
  connect the operational objects.

**CharlotteOS**

- Signed immutable ELF artifacts grouped into one signed release
- Deployment descriptors bind artifact digests, resource limits, replica
  policy, shutdown grace, and named capability grants.
- Raft commits desired placement, exact generations, and readiness records.
- Node agents fetch the artifacts and reconcile their local protected domains.

The Charlotte form aims to make executable identity and runtime authority part
of the same admitted contract.

## Slide 8: Authority and isolation

**OpenShift**

- Service accounts, RBAC, admission policy, Secrets, NetworkPolicy, Security
  Context Constraints, seccomp, and SELinux combine to constrain a workload.
- Enforcement spans the control plane, container runtime, Linux kernel, and
  network implementation.
- The model supports broad application compatibility and mature multi-tenant
  administration.

**CharlotteOS**

- Possession of a capability authorises one operation on one kernel object or
  service endpoint.
- A signed grant list defines the connections a component may acquire.
- Applications receive no ambient name-service or network authority.
- Linear Rust owners represent allocation, transfer, cancellation, and release
  in normal program structure.

CharlotteOS tests whether fewer authority mechanisms can make the enforcement
chain easier to inspect end to end.

*Suggested visual: [Capability-safe IPC](../manual-v2/figures/capability-safe-ipc.svg).*

## Slide 9: Development and operations separation

**A common OpenShift workflow**

- Development supplies images, Helm charts, Operators, or Kubernetes
  manifests.
- Operations adds namespaces, policy, routes, storage classes, credentials,
  admission controls, and environment overlays.
- The boundary is workable and mature, although ownership often overlaps in
  YAML, templates, CI/CD systems, and application environment variables.

**The CharlotteOS model**

- Development signs executable behavior and declares logical requirements.
- Operations independently signs the production binding and encrypts connector
  profiles to the destination cluster.
- The cluster verifies both authorities and grants their policy intersection.
- Development cannot select production credentials. Operations cannot replace
  the approved executable through a connector binding.

The separation is a cryptographic property of the deployment path rather than
only a repository convention or pipeline role.

*Suggested visual: [Role-separated deployment trust](../manual-v2/figures/role-separated-deployment-trust.svg).*

## Slide 10: Application configuration and infrastructure connectivity

**OpenShift applications commonly consume**

- URLs and broker lists through ConfigMaps or environment variables
- Credentials and certificates through Secrets or mounted volumes
- SDKs, sidecars, service meshes, or Operators that implement connection policy

**CharlotteOS applications consume**

- Business configuration and logical dependency names
- An attenuated S3, Kafka, clock, or procedure endpoint capability
- Small route or operation identifiers already constrained by connector policy

The Charlotte connector receives addresses, credentials, TLS roots, client
identity, topic or prefix limits, and broker lifecycle responsibility. Changing
an infrastructure endpoint or credential can become a connector rollout while
the application artifact remains unchanged.

*Suggested visual: [External service capabilities](../manual-v2/figures/external-service-capabilities.svg).*

## Slide 11: Cluster networking and service identity

**OpenShift**

- Services, EndpointSlices, Routes or Ingress, DNS, CNI, and load balancers
  give stable identities to changing Pod sets.
- The platform supports broad ingress policy, network isolation, observability,
  and integration choices.

**CharlotteOS**

- Distributed naming maps a logical service to an active generation and owner.
- Committed placement plus exact-generation readiness determines which replicas
  may receive new connections.
- A service VIP hides individual nodes behind one cluster identity.
- Direct Server Return selects a backend without terminating TCP in an
  intermediate proxy.

The present Charlotte implementation supports a much narrower ingress model.
Its significance is the direct connection between cluster placement,
readiness, capability policy, and packet eligibility.

*Suggested visual: [Cluster management and DSR](../manual-v2/figures/cluster-management-dsr.svg).*

## Slide 12: Lifecycle and shutdown

**OpenShift**

- Deployments, StatefulSets, Operators, health probes, disruption budgets, and
  termination grace periods provide mature rollout and lifecycle control.
- Controllers and ecosystem tooling cover many application and infrastructure
  failure modes.

**CharlotteOS**

- Artifact digest and generation identify the exact running component.
- Readiness gates grants, publication, and new ingress flows.
- Generation fencing prevents stale cleanup or registration.
- A signed shutdown grace follows the deployment through drain and retirement.
- The kernel enforces a final deadline after cooperative teardown.

CharlotteOS has implemented the main AArch64 lifecycle path. Health-driven
replacement, rich rolling policy, full platform parity, and production
controllers remain open work.

*Suggested visual: [Cooperative deployment shutdown](../manual-v2/figures/cooperative-deployment-shutdown.svg).*

## Slide 13: The node as an operational object

**An OpenShift node**

- Runs Red Hat Enterprise Linux CoreOS, kubelet, CRI-O, networking and storage
  components, and platform DaemonSets.
- Participates in coordinated OS, cluster, driver, and workload lifecycle.
- Retains a Linux host identity even when direct administration is tightly
  controlled.

**The CharlotteOS target node**

- Boots an appliance-like image, establishes identity, and joins cluster state.
- Advertises resources and failure-domain properties.
- Receives assignments and loads only verified artifacts with delegated
  authority.
- Carries no per-node application installation or general-purpose package
  environment.

Application variation belongs in signed releases and replicated policy. Nodes
become replaceable resource providers for the cluster.

## Slide 14: How incident reasoning differs

- **Why is this workload running?** OpenShift answers through workload objects,
  controllers, scheduler decisions, and events. Charlotte answers through a
  signed release and committed assignment.
- **Which code is active?** OpenShift uses image identity and rollout state.
  Charlotte uses the admitted artifact digest and exact generation.
- **What may it access?** OpenShift combines identity, RBAC, Secrets, network
  policy, and host controls. Charlotte inspects the granted capability set and
  connector attenuation.
- **Why did it receive traffic?** OpenShift inspects Service selection,
  endpoints, readiness, ingress, and network state. Charlotte inspects
  placement, exact-generation readiness, and the DSR eligibility epoch.
- **Where are external credentials?** OpenShift commonly projects a Secret to
  the Pod or delegates through another platform component. Charlotte confines
  them to a separately launched connector.

CharlotteOS seeks a shorter causal chain. Operational evidence must determine
whether that chain remains understandable as the platform grows.

## Slide 15: AI changes the compatibility economics

- POSIX and Linux compatibility have long reduced migration cost by letting an
  organisation reuse existing software and operational knowledge.
- Coding agents can reduce the human effort needed to discover assumptions,
  translate APIs, generate platform adapters, and create conformance tests.
- This makes clean-break execution environments more credible for new software
  and for applications whose source and business behavior the organisation
  controls.
- Durga demonstrates the direction: BPMN process intent and business procedures
  can produce Charlotte adapters, capability requests, resource declarations,
  build inputs, and deployment metadata.
- Reproducible generation, review, provenance, security analysis, and runtime
  verification remain mandatory. AI-generated code does not become trusted by
  origin.
- Opaque vendor products and applications with deep Linux or native-library
  dependencies continue to favour OpenShift's compatibility model.

AI therefore changes the cost boundary rather than removing it. An IT
department can assess the cost of verified adaptation alongside the cost of
carrying the general-purpose compatibility stack.

*Suggested visual: [Durga to Charlotte generation](../manual-v2/figures/durga-charlotte-generation.svg).*

## Slide 16: Maturity and organisational risk

**OpenShift today**

- Production platform with vendor support, release engineering, security
  response, lifecycle policy, and a large integration ecosystem
- Broad Linux workload compatibility and established operational practice
- Rich administration, identity, observability, storage, networking, backup,
  and disaster-recovery options

**CharlotteOS today**

- Research implementation with working capability, consensus, placement,
  connector, ingress, and lifecycle foundations
- Purpose-built Rust applications and a small set of tested virtual or
  experimental hardware targets
- Outstanding work in KMS and hardware-rooted trust, administrative
  authorization, automated failure convergence, rolling policy, platform
  observability, backup, and physical qualification

Existing production estates remain natural OpenShift workloads. CharlotteOS is
appropriate for controlled evaluation and architectural research at this
stage.

## Slide 17: The research proposition

OpenShift addresses this practical question:

- How can an organisation operate today's Linux applications safely and
  consistently across a cluster?

CharlotteOS investigates a different question:

- What should an operating system look like when the cluster, signed workload,
  placement decision, and explicit authority are native from the beginning?

The likely outcome is selective inheritance. Desired state, reconciliation,
scheduling, readiness, and health-driven recovery remain valuable. Pods, OCI,
CRI, and part of the boundary-construction machinery may become unnecessary in
a clean-break execution environment.

The worthwhile comparison measures operational consequences: number of policy
layers, secret distribution, causal clarity during failure, recovery steps,
and the effort required to adapt applications.
