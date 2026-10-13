# Scoped deployment launch and retirement

`catten_rt::owned::launch_scoped_artifact(artifact, artifact_len, descriptor,
descriptor_len)` is the sole scoped application-launch API. It consumes two
`OwnedMemory` values and returns `Result<DeployedArtifact, ArtifactLaunchError>`.
The signed descriptor's complete name is authoritative; there is no packed-name
argument, named alias, independently supplied retirement name or public
ASID-only scoped owner wrapper.

## Register boundary

Syscall `SpawnArtifactScoped` (76) takes four arguments:

| Register slot | Meaning |
| --- | --- |
| 1 | ELF memory capability |
| 2 | Exact ELF byte length |
| 3 | Signed descriptor memory capability |
| 4 | Exact descriptor byte length |

On ARM these are `x1`–`x4`; on x86 they are `rdi`, `rsi`, `rdx`, `r10`.
Success returns ASID in `x0`/`rax` and the verified artifact principal in
`x1`/`rdi`. Rejection returns `(0, 0)`. The kernel derives the principal from
the descriptor name and publishes that same principal in its deployment record
only after signed ELF/descriptor verification and successful scoped launch.

Both memory capabilities are consumed on every submitted outcome. Duplicate
input handles reject and are closed once. Local runtime length rejection
precedes submission and releases both Rust owners normally. The runtime maps
neither buffer and adopts no new scalar memory owner; successful submission
constructs `DeployedArtifact` directly from the returned launch receipt.

The ARM wrapper uses the four-argument, two-result emitter with memory effects;
it does not declare `nomem`. The old five-argument scalar-result emitter has no
remaining callers and is removed on ARM, x86 and unsupported-target stubs.
The old packed-name scoped API and its named wrappers are removed together.
Scoped launch now accepts signed `CDEPLOY6` variant descriptors; all services must be rebuilt
for this register contract. No earlier arity is decoded or translated.

## Retirement owner

Keep `DeployedArtifact` until explicit `poll_retire` returns `Ok(true)`. Pending
or rejected retirement retains that owner; its existing best-effort Drop
fallback requests forced retirement of its returned principal. Node shutdown
keeps its existing enclosing-deadline and reason contract. Neither success
receipt nor a scalar error authorizes replay of uncertain physical cleanup.

Retirement syscall 65 and its shared kernel state machine require a nonzero
principal. Zero returns `u64::MAX` before acquiring or changing the deployment
registry. The unused `retire_artifact()` wrapper and singleton-selection branch
are removed. An absent nonzero principal still reports completed/absent, as
required by an owner's repeated completion observation.

This reuses the current deployment/closing-root owners and C18 policy. It adds
no registry, recovery ticket or physical-completion proof. Principal identity
does not replace exact root generation, retirement claims, admission charges
or hardware quiescence.

## Remaining removal

S04b2a removes descriptor-less syscall 64 and its runtime/SDK wrappers,
plus the packed catalog deploy/query operations. All active application rollout
fixtures use signed descriptors and scoped launch; retired opcodes reject.
Redundant primary record fields remain the next coordinated removal across
controllers, replication and persistence. The encrypted connector pickup has
a distinct authenticated target and remains a separate API.

See [administration contract](deployment-administration.md) and its
[removal evidence](../reports/audits/2026-10-13-protocol-simplification-administration.md).

See [batch evidence](../reports/audits/2026-10-13-protocol-simplification-launch.md).
