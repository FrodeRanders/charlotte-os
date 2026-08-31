A coherent CharlotteOS figure set answers eleven different questions, from broad structure down to resource
lifetime, external integration, and application delivery.

### 1. System layering

```mermaid
flowchart BT
HW["Hardware and firmware<br/>CPU · RAM · PCIe · NVMe · NIC · timers"]

      subgraph K["CharlotteOS kernel — trusted mechanism"]
          PLATFORM["Boot and platform discovery"]
          SCHED["LP scheduler and address spaces"]
          MEMORY["Memory objects and mappings"]
          CAPS["Capability tables and delegation"]
          IPC["Endpoint IPC and completion queues"]
          DEVICE["Interrupt, MMIO, DMA and IOMMU isolation"]
          LOADER["EL0 loader and service supervision"]
      end

      subgraph U["Userspace — isolated protection domains"]
          ABI["catten-syscall<br/>raw ABI"]
          RT["catten-rt::owned<br/>linear resource ownership"]
          PROTOCOLS["Typed protocol crates"]
          SERVICES["Drivers and system services"]
          SITAS["Sitas shard runtime"]
          APPS["Applications and administration tools"]
      end

      HW --> PLATFORM
      HW --> DEVICE
      PLATFORM --> SCHED
      SCHED --> LOADER
      MEMORY --> LOADER
      CAPS --> IPC
      DEVICE --> IPC

      LOADER --> ABI
      IPC --> ABI
      ABI --> RT
      RT --> PROTOCOLS
      PROTOCOLS --> SERVICES
      RT --> SITAS
      SERVICES --> APPS
      SITAS --> APPS
```

The kernel supplies isolation and bounded mechanisms; service policy, storage, networking, naming, consensus, and
applications remain outside it.

### 2. Kernel anatomy and userspace boundary

```mermaid
flowchart LR
subgraph USER["One userspace protection domain"]
CODE["Service or application"]
OWNED["Owned Rust resources<br/>Endpoint · Connection · OwnedMemory<br/>PendingCall · Completion"]
CQ["Mapped completion queue"]
end

      subgraph KERNEL["Kernel"]
          TRAP["Syscall boundary"]
          CAPTABLE["Per-address-space<br/>capability table"]
          IPC["IPC endpoints<br/>and reply tokens"]
          COMP["Completion objects"]
          VM["Address spaces<br/>and memory mappings"]
          SCHED["Threads, LP affinity<br/>and scheduling"]
          DEV["Device grants<br/>MMIO · IRQ · DMA"]
      end

      subgraph HARDWARE["Hardware"]
          CPU["Processors"]
          RAM["Memory"]
          IO["PCIe devices"]
          IOMMU["IOMMU / SMMU"]
      end

      CODE --> OWNED
      OWNED --> TRAP
      CQ <-->|completion records| COMP

      TRAP --> CAPTABLE
      CAPTABLE --> IPC
      CAPTABLE --> COMP
      CAPTABLE --> VM
      CAPTABLE --> DEV

      IPC --> SCHED
      COMP --> SCHED
      VM --> RAM
      SCHED --> CPU
      DEV --> IOMMU
      IOMMU --> IO
```

Capabilities are scoped to an address space; userspace cannot directly name another domain’s kernel objects.

### 3. Steady-state service composition

```mermaid
flowchart LR
NS["Node-local name service<br/>registration and deferred lookup"]
OBS["observe<br/>monotonic clock and system snapshots"]

      BLOCKHW["NVMe / AHCI / virtio-blk"]
      BLK["Userspace block driver<br/>blk0"]
      OBJ["Object store"]

      NICHW["Ethernet controller"]
      NIC["Userspace NIC driver<br/>net0"]
      FROUTER["Frame router<br/>single receive owner"]

      TCP["tcpip<br/>DHCP · TCP · connected UDP"]
      DISCO["disco<br/>cluster discovery"]
      REL["relmsg<br/>reliable fragmented messages"]
      DNS["dns + Graft<br/>distributed catalog and Raft"]

      TIME["time<br/>SNTP · drift model · holdover"]
      HTTP["httpd<br/>node status keyhole"]
      APP["Applications and admin services"]

      BLOCKHW --> BLK --> OBJ
      NICHW --> NIC
      NIC -->|"received frames"| FROUTER

      FROUTER -->|"IPv4 / ARP"| TCP
      FROUTER -->|"discovery EtherType"| DISCO
      FROUTER -->|"message EtherType"| REL

      TCP -->|"transmit"| NIC
      DISCO -->|"transmit"| NIC
      REL -->|"transmit"| NIC

      DISCO -->|"peer membership"| DNS
      REL -->|"Raft transport"| DNS
      OBJ -->|"durable Raft state"| DNS

      TCP -->|"UDP / NTP"| TIME
      OBS -->|"monotonic oscillator"| TIME
      OBJ -->|"calibration holdover"| TIME

      TCP -->|"TCP listener"| HTTP
      OBS -->|"thread snapshots"| HTTP
      DNS -->|"cluster status"| HTTP
      DISCO -->|"peer status"| HTTP
      REL -->|"transport status"| HTTP

      APP --> HTTP
      APP --> TIME
      APP --> DNS
      APP --> OBJ

      NS -. "lookup and registration" .-> BLK
      NS -.-> OBJ
      NS -.-> NIC
      NS -.-> FROUTER
      NS -.-> TCP
      NS -.-> DISCO
      NS -.-> REL
      NS -.-> DNS
      NS -.-> TIME
      NS -.-> HTTP
```

Launch order is not the dependency mechanism: services register with the name service and deferred lookups wait
until dependencies appear.

### 4. Ordinary boot versus optional testing

```mermaid
flowchart TD
BOOT["Firmware enters CharlotteOS"]
INIT["Initialize memory, interrupts,<br/>scheduler and secondary LPs"]
CORE["Start node name service<br/>and observe service"]
DISCOVER["Discover PCIe devices<br/>and protected DMA paths"]
LAUNCH["Launch steady-state service domains"]

      STORAGE["Block driver → object store"]
      NETWORK["NIC driver → frame router"]
      CLUSTER["disco · relmsg · dns/Raft"]
      APPLIANCE["tcpip · time · httpd"]

      READY["Publish local-ready marker"]
      DHCP["Acquire address through DHCP"]
      PEERS["Discover peers and form cluster"]
      UTC["Query NTP and calibrate UTC"]
      SERVE["Serve application and HTTP endpoints"]

      TESTS["Optional *-test verifiers<br/>observe and validate existing services"]

      BOOT --> INIT --> CORE --> DISCOVER --> LAUNCH
      LAUNCH --> STORAGE
      LAUNCH --> NETWORK
      NETWORK --> CLUSTER
      NETWORK --> APPLIANCE

      STORAGE --> READY
      CLUSTER --> READY
      APPLIANCE --> READY

      READY --> DHCP
      READY --> PEERS
      READY --> UTC
      DHCP --> SERVE
      PEERS --> SERVE
      UTC --> SERVE

      TESTS -. "adds verification only" .-> DHCP
      TESTS -.-> PEERS
      TESTS -.-> UTC
      TESTS -.-> SERVE
```

Networking, DHCP, discovery, cluster formation, UTC synchronization, and HTTP are normal boot behavior. Test
switches add validators; --no-network removes the network-dependent composition.

### 5. Capability-safe IPC call

```mermaid
sequenceDiagram
participant A as Application
participant K as Kernel
participant N as Name service
participant S as Target service
participant Q as Completion queue

      S->>K: Create service endpoint and delegable connection
      K-->>S: Endpoint owner + connection capability
      S->>K: Register name, moving the service connection
      K->>N: Deliver authenticated registration
      N->>N: Store the re-delegable service connection
      N-->>K: Registration generation / success
      K-->>S: Registration completes

      A->>K: Call bootstrap connection: lookup(name, rights)
      K->>N: Deliver authenticated IPC request
      N->>K: Delegate restricted service connection
      K-->>Q: Lookup completion
      Q-->>A: Connection capability becomes available

      A->>K: call / call_move / borrowed-memory call
      K->>S: Deliver message and reply token
      S->>K: Reply, optionally moving a capability
      K-->>Q: Post terminal completion
      Q-->>A: PendingCall resolves

      Note over A,K: Rust owners prevent duplicate adoption
      Note over A,S: Moved capabilities change owner exactly once
      Note over K,Q: Cancellation remains observable until terminal cleanup
```

This depicts why application code uses catten_rt::owned: Rust ownership mirrors capability ownership across the ABI.

### 6. Two-node cluster

```mermaid
flowchart LR
      subgraph A["Charlotte node A"]
          KA["Kernel"]
          NSA["Local name service"]
          DA["disco"]
          DNA["dns + Raft"]
          RA["relmsg"]
          OA["Local object store"]
          AA["Node deployment agent"]
          S3A["Bootstrap S3 connector"]
          CA["Operational connector domain"]
      end

      subgraph B["Charlotte node B"]
          KB["Kernel"]
          NSB["Local name service"]
          DB["disco"]
          DNB["dns + Raft"]
          RB["relmsg"]
          OB["Local object store"]
          AB["Node deployment agent"]
          S3B["Bootstrap S3 connector"]
          CB["Operational connector domain"]
      end

      CENTRAL["Central S3-compatible object store"]

      DA <-->|"L2 discovery"| DB
      RA <-->|"reliable Ethernet messages"| RB
      DNA <==>|"Raft RPCs through relmsg"| DNB
     
      NSA --> DNA
      NSB --> DNB
      OA --> DNA
      OB --> DNB

      DNA -->|"replicated names,<br/>membership and deployments"| AA
      DNB -->|"replicated names,<br/>membership and deployments"| AB

      AA -->|"bounded encrypted pickup"| KA
      AB -->|"bounded encrypted pickup"| KB
      AA -->|"fetch by digest"| S3A
      AB -->|"fetch by digest"| S3B
      S3A -->|"TLS + connector credentials"| CENTRAL
      S3B -->|"TLS + connector credentials"| CENTRAL
      KA -->|"verified launch +<br/>read-only profile"| CA
      KB -->|"verified launch +<br/>read-only profile"| CB
```

This separates three kinds of state: node-local connection registration, locally durable Raft state, and immutable
deployment artifacts and encrypted operational profiles held in a central S3-compatible store. The replicated catalog
contains desired deployment state plus compact, signed ciphertext references—not connector credentials. The selected
node fetches digest-pinned inputs through a separately provisioned bootstrap S3 connector before the bounded pickup
crosses into the kernel’s trusted verification, decryption, and loader path.

### 7. Secrets and attenuated external-service capabilities

```mermaid
flowchart LR
subgraph TRUSTED["Infrastructure-controlled services"]
S3C["Named S3 connector<br/>endpoint · bucket · TLS · credentials"]
KC["Named Kafka connector<br/>broker pool · TLS/mTLS · SASL/SCRAM"]
KS["kafka_step<br/>delivery and transaction owner"]
G["grantctl<br/>descriptor and caller checks"]
end

      subgraph APPDOMAIN["Deployed application domain"]
          BOOT["Bootstrap connection<br/>to grantctl only"]
          S3EP["Attenuated S3 capability"]
          PROC["Business procedure<br/>no_std generated adapter"]
      end

      S3["Central S3-compatible store"]
      KAFKA["Managed Kafka cluster"]

      G -->|"exact signed grant by name and rights"| S3EP
      BOOT -->|"request declared grants"| G
      S3EP -->|"object operations only"| S3C
      S3C -->|"authenticated TLS"| S3

      KC -->|"authenticated TLS + SASL"| KAFKA
      KS -->|"poll · produce · offsets · transactions"| KC
      KS -->|"typed request"| PROC
      PROC -->|"validated output or failure"| KS
```

Network locations, trust roots, user names, passwords, and Kafka transactional authority stay in infrastructure-owned
connector profiles. The application receives only the narrow capability named by its signed deployment descriptor.
For a transactional Kafka step, `kafka_step` owns the delivery and transaction resources and invokes the procedural
application; the application never receives the Kafka connector capability.

### 8. Signed atomic release admission and rollout

```mermaid
sequenceDiagram
participant C as CI / operator
participant O as Central S3 store
participant D as deployd on any node
participant X as clusterctl
participant R as dns / Raft leader
participant A as Assigned node agents
participant G as grantctl
participant P as Application components

      C->>C: Build self-contained ELFs and add CLS2 signatures
      C->>O: Upload immutable artifact objects
      C->>C: Sign one CDEPLOY2 per component, including stack pages
      C->>C: Sign ordered descriptors as CRELEASE(name, sequence)
      C->>D: POST /v1/releases
      D->>X: Move bounded release memory
      X->>X: Verify outer and every nested signature
      X->>R: Relay to leader when necessary
      R->>R: Resolve placement and preflight every revision
      R->>R: Commit all desired records in one Raft command
      R-->>A: Replicated desired deployment set

      par Each assigned component
          A->>O: Fetch opaque object key through local S3 connector
          A->>A: Verify digest and CLS2 executable signature
          A->>P: Launch with grantctl + immutable descriptor
          P->>G: Request grants and publish declared name + generation
          G->>R: Replicate published generation
      end

      loop Until deadline
          C->>D: GET /v1/deployments/{component}
          D->>R: Query rollout state
          R-->>C: committed / replacing / ready
      end
```

The `CRELEASE` envelope binds a monotonic release identity to the exact ordered `CDEPLOY2` bytes. Admission is atomic:
all component revisions enter the replicated desired state or none do. Readiness is deliberately not claimed to be
simultaneous—fetch, verification, launch, capability acquisition, and publication occur independently after commit.
Coordinated rollback, progress deadlines, and failure-domain-aware rescheduling remain controller work.

### 9. Privileged operational-profile pickup

```mermaid
sequenceDiagram
participant DEV as Development authority
participant OPS as Operations authority
participant O as Central S3 store
participant D as deployd on any node
participant R as dns / Raft leader
participant A as Authorized node agent
participant K as Kernel launch gate
participant C as Isolated connector

      DEV->>DEV: Sign ELF with artifact key
      DEV->>DEV: Sign CDEPLOY2 + CRELEASE with deployment key
      DEV->>O: Upload immutable connector ELF

      OPS->>OPS: Build bounded CHS3PF1 or Kafka profile
      OPS->>OPS: HPKE-seal COPSENC1 to cluster recipient key
      OPS->>OPS: Sign mapping and COPSBND2 with operations key
      OPS->>O: Upload encrypted profile envelope
      OPS->>D: Notify with signed COPSBND2

      D->>R: Relay bounded admission proof
      R->>R: Verify release, operations role, recipient and trusted UTC
      R->>R: Commit compact reference + detached binding signature
      Note over R: No ciphertext or plaintext enters Raft

      A->>R: Query assigned deployment + COPSLST1 bindings
      A->>O: Fetch digest-pinned ELF and encrypted envelope via bootstrap S3
      A->>R: Fetch exact signed release and descriptor
      A->>A: Query trusted UTC and build COPSPK01
      A->>K: Move owned pickup memory
      Note over A,K: Capability is consumed on every submitted outcome

      K->>K: Check authorized reuse-safe agent identity
      K->>K: Re-verify release, descriptor, ELF and artifact principal
      K->>K: Verify detached mapping + COPSENC1 signature and expiry
      K->>K: HPKE-open into Zeroizing memory and decode typed profile
      K->>C: Move profile read-only, then start connector
      K->>K: Zero transient plaintext after launch attempt
```

The application and node agent never receive plaintext infrastructure credentials. Only the kernel’s privileged
launch gate opens the envelope; the connector receives one immutable, policy-bounded profile. The bootstrap S3
connector is provisioned separately because the mechanism used to retrieve operational profiles cannot configure
itself without introducing a circular dependency.

### 10. Role-separated deployment trust and capability chain

```mermaid
flowchart TD
AK["Artifact signing key<br/>development/build authority"]
DK["Deployment signing key<br/>release authority"]
OK["Operations signing key<br/>environment authority"]
RPK["Cluster recipient public key"]
RSK["Recipient private key<br/>kernel/KMS custody"]
TRUST["CTRUST1 public policy<br/>role-specific keys + cluster ID"]
ELF["CLS2-signed ELF"]
DESC["CDEPLOY2<br/>digest · object key · selector · stack pages · grants"]
REL["CRELEASE<br/>name · sequence · ordered descriptors"]
PROFILE["CHS3PF1 / Kafka profile<br/>infrastructure details + credentials"]
ENC["COPSENC1<br/>HPKE ciphertext + operations signature"]
BUNDLE["COPSBND2<br/>release binding + detached mapping proof"]
CAT["Raft catalog<br/>desired state + compact references"]
AGENT["Uniquely authorized node agent"]
GATE["Kernel operational launch gate"]
CONNECTOR["New connector address space<br/>immutable read-only profile"]
DOMAIN["Application address space"]
GRANT["grantctl"]
ENDPOINT["Attenuated named connector endpoint"]

      AK -->|"sign code identity"| ELF
      DK -->|"sign exact deployment"| DESC
      DK -->|"sign descriptor set"| REL
      RPK -->|"HPKE seal"| ENC
      OK -->|"sign encrypted profile"| ENC
      PROFILE -->|"encrypt"| ENC
      OK -->|"sign compact mapping"| BUNDLE
      REL --> BUNDLE
      ENC --> BUNDLE
      TRUST -->|"leader admission checks"| BUNDLE
      BUNDLE -->|"compact signed reference only"| CAT
      CAT -->|"desired connector for this node"| AGENT
      AGENT -->|"move COPSPK01"| GATE
      TRUST -->|"independent re-verification"| GATE
      RSK -->|"transient HPKE open"| GATE
      ELF --> GATE
      DESC --> GATE
      REL --> GATE
      ENC --> GATE
      GATE -->|"read-only typed profile"| CONNECTOR
      CONNECTOR --> ENDPOINT
      DESC -->|"declared name + rights"| GRANT
      DOMAIN -->|"sole bootstrap authority"| GRANT
      GRANT -->|"attenuate exact capability"| ENDPOINT
```

Development can choose code and logical requirements but cannot select production credentials. Operations can bind
those requirements to managed infrastructure but cannot replace application bytes. Raft establishes cluster-wide
ordering, the reuse-safe node-agent identity controls entry to the launch gate, the kernel retains the recipient private
key and re-verifies every role, and `grantctl` exposes only the descriptor-authorized endpoint to the application. No
single key or mechanism substitutes for the others.

### 11. Durga-to-Charlotte application generation

```mermaid
flowchart LR
MODEL["Durga process model<br/>activities · messages · Kafka routes"]
GEN["Charlotte target generator"]

      subgraph OUTPUT["Generated application package"]
          ADAPTERS["Compilable no_std<br/>activity adapters"]
          HANDLERS["Business handler stubs<br/>fail closed until implemented"]
          BUILD["Cargo manifest and build script"]
          CAPS["Execution-resource and<br/>capability plan"]
          KPROFILES["Named Kafka connector<br/>and kafka_step profiles"]
          PLAN["Multi-component deployment plan"]
          COMMANDS["Exact descriptor-sign,<br/>release-sign and release-apply commands"]
      end

      ARTIFACTS["Signed component ELFs<br/>in central object storage"]
      RELEASE["Signed CRELEASE"]
      CLUSTER["Charlotte cluster"]

      MODEL --> GEN
      GEN --> ADAPTERS
      GEN --> HANDLERS
      GEN --> BUILD
      GEN --> CAPS
      GEN --> KPROFILES
      GEN --> PLAN
      GEN --> COMMANDS

      BUILD --> ARTIFACTS
      PLAN --> RELEASE
      CAPS --> RELEASE
      KPROFILES -->|"provision connector instances separately"| CLUSTER
      COMMANDS --> RELEASE
      ARTIFACTS --> CLUSTER
      RELEASE --> CLUSTER
```

Durga now generates the Charlotte-specific scaffolding and a deployable component release plan, but it does not invent
business behavior: developers still implement the generated fail-closed handlers. The present `CRELEASE` binds concrete
executable deployment decisions. A richer semantic bundle could additionally bind the original process model, schemas,
provenance, communication graph, replica policy, affinity rules, and update strategy. Connector profiles are
infrastructure inputs, not application-visible release secrets; descriptors bind only the names and rights an
application may request. Per-thread stack pages originate in the developer-reviewed component plan, are signed into
`CDEPLOY2`, and are enforced exactly for the protected domain rather than guessed or silently clamped by the cluster.
