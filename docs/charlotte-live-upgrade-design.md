# Live Service Upgrade — Design

## Can CharlotteOS replace a running service without losing state?

**Yes.** The primitives needed already exist. What's missing is the
supervisor-orchestrated protocol.

### The four primitives (all implemented)

1. **Memory-object ownership transfer** — `memory::object::move_to` moves a
   page-backed memory object from one address space to another. The sender
   loses access; the receiver gains it. This is the vehicle for service state.

2. **Generation tracking** — the userspace name service records an instance
   generation that increments on restart. A client calling through a stale
   connection gets `EndpointClosed`; re-looking up the name returns the new
   generation and a fresh connection.

3. **EndpointClose / Cancelled** — when a service domain exits, the kernel
   closes its endpoint. Queued requests complete as `EndpointClosed`;
   delivered calls held by an exited server complete as `Cancelled`. Clients
   see a deterministic failure, not a hang.

4. **Bootstrap capability delivery** — the supervisor writes one initial
   capability to the config page before a domain starts. Currently this is
   either an endpoint (for the name service) or a connection to the name
   service (for everyone else). A live-upgrade handoff adds one more slot:
   the old service's state (as memory objects) plus the old service's
   endpoint capability.

### The handoff protocol

```
     Old Service         Supervisor         New Service
          |                    |                   |
     OP_HANDOFF               |                   |
          |-------------------->                   |
          |   drain in-flight, |                   |
          |   serialize state  |                   |
          |   → move_to        |                   |
          |<-------------------|                   |
          |  reply: "ready"    |                   |
          |                    |  spawn_upgrade()  |
          |  (exit)            |------------------>|
          |                    |  bootstrap:       |
          |                    |    ns_connection  |
          |                    |    state_memory   |
          |                    |    endpoint_cap   |
          |                    |                   |
          |                    |         new service reads state,
          |                    |         takes over endpoint,
          |                    |         registers (new generation)
          |                    |                   |
                    clients see EndpointClosed
                    → re-lookup → fresh connection
                    → resume from where they left off
```

### What the supervisor needs to add

```rust
// supervisor.rs — new entry point
pub struct UpgradeGrant {
    /// Memory objects the old service moved to the supervisor (its state).
    pub state_caps: Vec<MemoryObjectCap>,
    /// The old service's endpoint capability (so the new service can take it over).
    pub endpoint_cap: CapabilityId,
}

pub fn spawn_upgrade(
    image: &[u8],
    name_service: &NameServiceHandle,
    old_domain: ServiceDomain,
    grant: UpgradeGrant,
) -> ServiceDomain {
    // 1. Load the new service ELF
    // 2. Move the state memory objects to the new domain
    // 3. Delegate the old endpoint cap to the new domain
    // 4. Write bootstrap: ns_connection + state_caps + endpoint_cap
    // 5. Start the new domain
    // 6. Tear down the old domain (reclaims everything else)
}
```

### What the service must implement

The service must respond to an `OP_HANDOFF` request from the supervisor:

```rust
// In the service's message loop:
OP_HANDOFF => {
    // 1. Stop accepting new work (drain the queue first)
    // 2. Serialize all mutable state into memory objects
    // 3. Move those memory objects to the supervisor
    //    (the supervisor supplied its own address-space id in the handoff call)
    // 4. Reply "ready" → then exit
}
```

The new service receives the state via an extended bootstrap contract:

```rust
fn main(ctx: Context) -> ! {
    let manifest = ctx.manifest();
    let ns_connection = ctx.bootstrap_cap();
    let state_count = ctx.handoff_count();
    let state_base = ctx.handoff_state_cap();
    let endpoint_cap = ctx.handoff_endpoint_cap();

    // Deserialize state from the memory objects, register the old endpoint
    // under the same name (the name service bumps the generation), and
    // start serving — clients re-connect transparently.
}
```

### What clients see

A client holding a stale connection from the previous generation:

```rust
// Before upgrade:
let conn = ns.lookup("payroll")?;  // generation 3
payroll.call(OP_CALCULATE, employee_id);  // → in-flight

// During upgrade: the call completes as EndpointClosed or Cancelled.
// The client retries:
loop {
    let conn = ns.lookup("payroll")?;
    if conn.generation >= 4 { break; }  // new instance
    conn.close();
}

// After upgrade:
payroll.call(OP_CALCULATE, employee_id);  // → reaches the new instance
// The new instance has the old state, so it can resume.
```

### What is genuinely zero-downtime and what is restart-with-state

This design is **restart-with-state**, not truly zero-downtime: in-flight
requests to the old instance fail during the handoff window, and clients
must retry. True zero-downtime would require:

- Queue migration: the old service's endpoint queue would need to be
  transferred to the new service atomically, so no request is ever
  rejected. This is a future kernel feature — see "Distributed capability
  revocation" in the architecture doc's deferred decisions.

- Or, a *sidecar* model: the new service starts alongside the old one,
  both sharing the same endpoint, and the old service just stops accepting
  new work. The kernel doesn't currently support multiple owners for one
  endpoint.

### Relationship to the existing criterion-9 test

The `test_el0_uart` already exercises a crash-based restart (the
`OP_CRASH` fault-injection path). A live upgrade is the **graceful**
version: instead of crashing, the old service cooperatively hands off
its state. The restart, teardown, and re-registration are identical;
only the state transfer is new.

### Conclusion

The building blocks are in place. A live-upgrade slice would add:

1. `OP_HANDOFF` to the service protocol (supervisor calls the old service)
2. `UpgradeGrant` + `spawn_upgrade` in the supervisor
3. An extended bootstrap contract (state caps + old endpoint cap)
4. A reference handoff in `el0_service` (echo service → hand off state →
   new echo service reads it)
5. Boot-validated

This is ~150 lines of net-new code, testable in the same QEMU harness
that validates criterion 9.
