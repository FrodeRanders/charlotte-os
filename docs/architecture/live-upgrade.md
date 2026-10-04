# Live service upgrade

CharlotteOS has a tested reference implementation of restart-with-state.
The old service hands off one memory object; the replacement restores it,
creates a fresh endpoint and registers a new name-service generation. Clients
holding old connections re-look up and retry. This does not promise uninterrupted
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
6. Clients retry failed requests against the replacement.

The existing service-manager/echo self-test exercises this path. State transfer
uses the same memory-object ownership primitives as ordinary IPC, with the
original sponsor retaining the physical-backing charge until final reclamation.
See [resource ownership](../guides/resource-ownership.md) and
[memory-object budgets](../reference/memory-object-budgets.md).

The executable snapshot is verified as a CLS2 Ed25519-signed artifact before
mapping. The object checksum detects corruption; it is not executable authority.

## Multi-state kernel prototype

The supervisor also contains a separate, currently uncalled helper:

```rust
pub fn try_spawn_upgrade(
    image: &[u8],
    name_service: &NameServiceHandle,
    rights: ConnectionRights,
    old_asid: AddressSpaceId,
    grant: UpgradeGrant,
) -> Result<ServiceDomain, UpgradeSpawnError>;
```

`UpgradeGrant` describes state capabilities already held by the kernel
supervisor and an optional old endpoint from which to delegate a connection.
This helper is not the implementation invoked by the userspace upgrade syscall.

Its preparation owner holds the loaded, not-yet-running replacement and every
`PreparedMove`. Destination identities are reserved before source authority is
hidden; the state payload remains source-owned until commit. Connection
delegation completes before atomic move-batch publication. Failure drops the
preparation owner, restores original source handles without fresh quota, and
closes the staged replacement domain. Late cancellation is fenced by exact
namespace identity, including ASID/handle reuse. The legacy scalar reverse-move
cleanup API has been removed.

Kernel memory and IPC fixtures test the shared prepared-move mechanism's quota,
cancellation, retirement and batch-publication behavior. They do not establish
end-to-end coverage of this unused multi-state helper. See
[capability admission](../reference/capability-admission.md).

## Service responsibilities

Each service needs its own handoff protocol: stop accepting work, drain or
terminate in-flight operations, serialize state, and acknowledge handoff.
The replacement must validate and restore that state before advertising
readiness. Queued requests complete as `EndpointClosed` when their endpoint
exits; delivered calls whose server exits without replying complete as
`Cancelled`. Applications still need retry policy, including a decision about
operations whose side effects may already have happened.

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
