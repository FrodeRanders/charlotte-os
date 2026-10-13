# Current tooling inputs

Signing commands accept private-key **file paths only**, including public
development fixtures. They never accept the fixture's hexadecimal bytes in
argv. Files retain bounded no-follow reads, real-key ownership/permission
checks and zeroizing buffers. Known public fixture contents retain their
existing permission exemption; filenames do not confer that exemption.
`CLUSTER_SIGN_KEY_FILE` supplies a path to build/signing scripts; the retired
secret-valued variable still rejects without expanding its contents.
See [signing and trust](../guides/signing-and-trust.md).

The AArch64 runner accepts repeated `--cluster-service [NAME=]IPv4:PORT`
assignments. Named entries use deployment placement and exact-generation
readiness. An entry without a name selects the platform-service policy over
admitted, non-draining members. These are distinct current policies, not old
and new formats. An explicitly empty name rejects.

Builds consume one semicolon-separated `CATTEN_CLUSTER_SERVICES` table, for
example `orders=10.0.2.42:8080;10.0.2.43:80`. An absent/empty table configures
no bootstrap service assignments. The first committed signed ingress policy
replaces this bootstrap authority. `--cluster-service-name` is removed, and
`CATTEN_CLUSTER_VIP`, `CATTEN_CLUSTER_TCP_PORT` and
`CATTEN_CLUSTER_SERVICE_NAME` reject at kernel build time even when empty;
their values are not printed or used to synthesize a table. Runner-local
VIP/port values still select the first assignment's SLIRP host-forwarding
target and are not a second kernel policy source.

The unused `catten_services::socket::MAX_SOCKETS` export is removed. Socket
capacity comes from the privileged TCP/IP launch policy, with separate total
slots and per-principal socket/buffer quotas. The deprecated constant's 16
was never the current stack capacity. No wire opcode, runtime ABI or admission
budget changes in this batch.

The exported application SDK carries the same file-only signer. Sibling broker
and Durga callers previously recorded as passing key bytes must migrate their
call sites to paths; this repository does not maintain their argv exception.
See [S07 evidence](../reports/audits/2026-10-13-protocol-simplification-tooling.md).
