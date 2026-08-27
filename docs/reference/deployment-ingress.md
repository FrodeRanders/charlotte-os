# Signed deployment notification

CharlotteOS has a bounded off-cluster notification path for software already
placed in a centrally managed S3-compatible object store. The management
request carries a signed deployment descriptor, not the ELF and not object
store credentials.

## Operational flow

1. Build a self-contained CharlotteOS ELF and sign its CLS2 note with the
   offline cluster private key.
2. Calculate the SHA-256 of the final signed ELF.
3. Upload those immutable bytes from CI or an operator workstation to the
   central RustFS, Dell EMC ECS, or other compatible store.
4. Create a `CDEPLOY1` descriptor with `cluster-sign deployment-sign`. It binds
   the logical artifact name, exact digest, opaque object key, target node key,
   monotonic sequence, and named capability grants.
5. Submit it with `cluster-sign deployment-notify <descriptor> [host:port]`.

The normal network-and-storage service set starts `clusterctl`, `agent`, and
`deployd`. `deployd` listens on guest TCP port 7444. The QEMU runners forward
`${CATTEN_DEPLOY_HOST_PORT:-8081}` to that port under the default user network,
so the host tool defaults to `127.0.0.1:8081`.

For the current demonstration service, the final signing steps resemble:

```text
cluster-sign deployment-sign greet.cdep greet releases/greet-42.elf \
  <sha256-of-signed-elf> <target-node-key> 42 <private-key-hex> \
  greet=publish
cluster-sign deployment-notify greet.cdep 127.0.0.1:8081
```

A successful notification returns HTTP `202 Accepted` and JSON containing the
committed Raft deployment generation. Repeating the exact same descriptor is
idempotent. A lower signed sequence, or different signed bytes at the same
sequence, returns HTTP `409 Conflict`.

## Trust and secret boundary

The HTTP listener is plaintext by design: the Ed25519-signed descriptor is the
authorization and integrity envelope and contains no secret. `clusterctl`
verifies it before proposing state to Raft. Plaintext still reveals deployment
metadata and does not prevent connection-level denial of service, so production
networks should restrict the listener or place it behind an authenticated TLS
gateway when those properties matter.

The descriptor contains only an opaque object key. Endpoint addresses, bucket,
prefix, access credentials, Dell EMC ECS namespace, CA certificate, and TLS
client identity remain in the node's separately provisioned S3 profile. The
application receives neither that profile nor ambient name-service authority.

## Node-side pickup

Raft stores the signed descriptor with the assignment. The selected node agent
opens its local `s3` capability, streams the object, checks length and the
descriptor's SHA-256, verifies that CLS2 signs the ELF for the same artifact
name, and transfers both owning memory objects to the kernel. The kernel repeats
the checks and launches the ELF with only `grantctl` and the immutable
descriptor. An exact `publish` grant lets a service publish its endpoint
without obtaining name-service or mint authority.

## Current limits

- The S3 connector and credentials must be provisioned before notification;
  external S3 is not a bootstrap dependency.
- The replicated placement ABI currently accepts artifact names of at most
  eight bytes, and the first agent is specialized to `greet`.
- There is one active assignment and one active owner per artifact name.
- The notification response confirms Raft commitment, not application
  readiness. Rollout status, generic multi-artifact agents, replica placement,
  long deployment names, and a full central-store integration fixture remain.

