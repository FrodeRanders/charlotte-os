# Live service upgrade

CharlotteOS has a tested reference implementation of restart-with-state.
The old service hands off one memory object; the replacement restores it,
creates a fresh endpoint and registers a new name-service generation. Clients
holding closed old connections reacquire authority and apply protocol-specific
recovery. Name rebinding does not revoke a still-live old endpoint. This does not promise uninterrupted
in-flight requests or arbitrary-service upgrade compatibility.

## Running reference path

1. The service manager asks the old instance to hand off its state.
2. The old service drains its work and returns an unmapped state memory object.
3. The manager obtains a signed replacement ELF from the local object store, or
   selects the embedded recovery image.
4. The privileged `SpawnUpgrade` syscall validates the manager's connection
   authority, snapshots and verifies the executable, loads the replacement,
   transfers the single state object and supplies its bootstrap capabilities.
5. The replacement reads the state, creates an endpoint and registers the same
   logical name. The name service publishes the new connection/generation.
6. Clients reacquire a connection to the replacement. They replay only when
   the protocol establishes that it is safe; accepted unanswered work may
   already have changed state.

The existing service-manager/echo self-test exercises this path. State transfer
uses the same memory-object ownership primitives as ordinary IPC, with the
original sponsor retaining the physical-backing charge until final reclamation.
See [resource ownership](../guides/resource-ownership.md) and
[memory-object budgets](../reference/memory-object-budgets.md).

The executable snapshot is verified as a CLS2 Ed25519-signed artifact before
mapping. Executable authority comes from the signed artifact; the object checksum
detects corruption.

## Shared kernel preparation

The live syscall uses the supervisor's complete preparation path:

```rust
pub(crate) fn try_spawn_upgrade_from(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    old: AddressSpaceHandle,
    source: AddressSpaceHandle,
    grant: UpgradeGrant<'_>,
) -> Result<ServiceDomain, UpgradeSpawnError>;
```

`UpgradeGrant` borrows IDs from the captured source namespace and optionally
names an endpoint in the captured old service. The syscall passes its authorized
manager as source and one or zero state objects. An IPC identity lookup supplies
the old root generation; replacement classification is captured before loading.
The old, source, naming and replacement roots remain retained through preparation.

`PreparingUpgrade` owns the launch transaction, every `PreparedTransfer` and both
byte-admitted transfer/target-ID vectors. Both arrays reserve before escrow or
delegation. Failure explicitly cancels the known prefix outside guards, frees
array backing, then cancels the unstarted domain once. Abandonment retains every
field and original charge; no implicit cleanup or reverse-move reconstruction
runs. The first cleanup error stops without a second attempt. Commit publishes
the complete move batch before writing destination IDs and starting the initial
thread. Source/destination lifetime survives until its phase is complete.

Native controls cover both allocator classes, quota rejection before allocation
and after a vector prefix, failed transfer prefixes, guarded whole-parent
abandonment, maximum 31-entry array layouts, and real single-state/EL0 persistent
upgrade. They do not claim 31 simultaneous physical handoffs or a multi-state
endpoint upgrade campaign. See [launch/cache ownership](../reference/service-launch-ownership.md).

## Service responsibilities

Each service needs its own handoff protocol: stop accepting work, drain or
terminate in-flight operations, serialize state, and acknowledge handoff.
The replacement must validate and restore that state before advertising
readiness. Queued requests complete as `EndpointClosed` when their endpoint
exits; delivered calls whose server exits without replying complete as
`Cancelled`. Applications still need retry policy, including a decision about
operations whose side effects may already have happened.

The [reconnecting client skeleton](../guides/reconnecting-clients.md) keeps
business progress outside the session, preserves its operation deadline and
stops on uncertain accepted work. Use it as the application starting point;
the reference manager still contains raw ABI paths awaiting ownership conversion.

## Remaining work

- Connect the multi-state preparation helper to an authorized userspace
  orchestration API, with end-to-end failure tests.
- Define manager-owned old-domain teardown, handoff deadlines and recovery if
  the replacement cannot become ready.
- Define state schemas and compatibility policy for each real service.
- Provide explicit rollback/recovery policy; signed release metadata alone
  does not implement that policy.
- Extend crash-based recovery tests to interrupted handoff and replacement
  failure.

True zero-downtime would additionally require queue migration or a supported
overlap protocol. The current endpoint model has one owner; this reference
upgrade does not transfer a live endpoint queue.
