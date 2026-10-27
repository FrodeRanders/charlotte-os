# Cluster observability keyhole

CharlotteOS has traditionally exposed a node keyhole: `httpd` collects
voluntarily published service status and a capability-authorized scheduler
snapshot, then serves `/metrics`. The cluster keyhole complements it with the
questions an operator asks of the cluster rather than of one machine:

- which Raft term, leader, commit index, and membership epoch produced this
  answer;
- which nodes are admitted, draining, or currently reporting usable capacity;
- where every application is desired and ready at its exact generation; and
- which VIPs exist, which member advertises each VIP, and which ready replicas
  accept new flows.

The first implementation is available at `GET /cluster` and
`GET /cluster/metrics`. The JSON schema is named `charlotte.cluster.v2`.

## Authority and data path

The Raft/DNS service constructs a bounded `CLOBSV2` binary snapshot from one
locally applied catalog view. It answers only while that view satisfies the
same bounded-read freshness requirement as DSR. `httpd` owns no Raft state and
cannot invent placement: it asks DNS for the snapshot, strictly decodes it,
and renders JSON. A stale or malformed view becomes HTTP 503.

The cluster keyhole can be assigned an operations-owned VIP, for example:

```text
scripts/run-aarch64.sh release --http-test --cluster-service 10.0.2.42:443
```

An unnamed assignment denotes a platform service, so its new-flow backend set
is the admitted, non-draining membership. DSR follows its normal semantics:
the selected advertiser answers ARP for the VIP and forwards a flow directly
to one eligible member. The leader is normally the advertiser, but it need not
terminate the HTTP connection. Any eligible member can serve a fresh applied
snapshot; the response identifies both `raft.leader` and `raft.served_by`, as
well as whether it was served by the leader. This keeps observability available
during leader movement without weakening its consistency contract.

Capacity has a narrower meaning than committed catalog state. Each node row
marks its capacity sample as present and fresh independently. Followers can
report the replicated sample but generally cannot prove when the leader last
received it, so they conservatively render it stale. Committed frame promises
remain visible even when dynamic telemetry is stale. Raw one-second telemetry
and history remain in the node keyhole; the cluster view reports the filtered
inputs and decisions that close the placement feedback loop.

`truncated: true` means a configured record bound or the 64 KiB transport
budget was reached. Consumers must never interpret a truncated snapshot as a
complete inventory.

## Node drill-down

The initial dashboard links `/` on the member that served the cluster request.
The replicated membership and discovery protocols currently carry stable node
identities and identity-to-MAC routes rather than management IPv4 addresses.
Raft commits membership, but discovery and reliable messaging do not
cryptographically authenticate peers. Expected-peer and session checks provide
correlation within the current trusted-network boundary.
For that reason `nodes[].management_endpoint` is explicitly `null`; the UI
does not guess an address from a MAC or a QEMU convention.

The intended next step is a leased, operator-authorized management endpoint
record keyed by stable node identity. After that record exists, the cluster
adapter can offer `GET /cluster/nodes/{node-key}/metrics` as an authenticated
proxy. Proxying is preferable to placing every node keyhole directly on a
routable network: one policy and one audit point protect both the cluster view
and drill-down. A direct node URL remains useful on a protected maintenance
network, but it should use the same observer identity policy.

## Access control and browser certificates

The native `httpd` requires mutual TLS on port `443` for direct node and
cluster/VIP views. Deployment ingress uses the same mandatory TLS engine on
`7444`. An exact DER leaf digest is enrolled as reader or administrator; a
CA-issued but unenrolled certificate grants nothing. Readers can observe;
mutations require an administrator before controller dispatch and still require
the signed operation bodies. DSR is a routing identity, not authorization.

One complete signed HPKE boot resource carries server key/chain, client roots,
explicit enrollments and all eleven adjustable TLS limits. The kernel checks
its exact selected admission policy before trust publication and supplies
read-only launch profiles to both services. There is no separately selected
frontend authority or exposed plaintext backend. Missing credentials disable
network admission. Server certificate SANs must cover the expected DNS/IP name;
client certificates need `clientAuth` EKU and proof of private-key possession.
See [management TLS](../reference/management-tls.md) for file-based enrollment,
verified host clients, resource installation and related limits.

Public development roots, unauthenticated SNTP and incomplete custody mean
production remains disabled. Native single-node direct/VIP qualification does
not establish protected installation, full rotation/revocation or multi-node
leader movement. Replacement requires advancing the installed policy revision,
replacing the complete resource and restarting both services; old sessions are
not hot reconfigured.

The observer certificate is not the cluster signing key and must not reuse
that key material. Signing establishes authority over committed cluster
records; an observer client identity authorizes a human or tool to read a
particular view. Keeping those credentials separate preserves the development,
operations, and cluster trust boundaries.

A browser is configured as follows:

1. Operations issues a password-protected PKCS#12 (`.p12`/`.pfx`) bundle
   containing the observer's client certificate and private key.
2. Import the bundle into the operating-system certificate store for Safari or
   Chrome, or into Firefox's certificate manager when Firefox uses its own
   store. Import/trust the organization's keyhole CA separately if it is not
   already enterprise-managed.
3. Browse to the certificate DNS name, such as
   `https://keyhole.cluster.example/cluster`; use of the DNS name avoids a
   certificate-name failure that a bare VIP would cause unless the IP address
   is also present in the certificate SAN.
4. Select the observer certificate when prompted. Never export or install a
   cluster signing private key in the browser.

Importing an identity does not enroll its leaf or assign its role. Both the
client trust store and the server enrollment must match the reviewed policy.
`--http-test` uses disposable test credentials; ordinary boot requires the
explicit resource. No insecure verification or plaintext fallback is provided.

Version 2 removes the local object ID from deployment observations and the
JSON keyhole; deployment placement and readiness are represented by actual
node sets. The fixed deployment record is 28 bytes before name/node payloads.
Version-1 records are unsupported.

## Wire and API bounds

`catten_services::cluster_observe` owns the `CLOBSV2` codec. Version 2 is
limited to 32 nodes, 256 deployments, the launch ABI's bounded ingress table,
and 64 KiB total. Node identities are rendered as 16-digit hexadecimal strings
in JSON so JavaScript cannot lose precision. Unknown flags, noncanonical
ordering, duplicate identities, invalid service identities, trailing data,
and oversized messages fail closed.

The snapshot IPC operation is `dns::OP_CLUSTER_SNAPSHOT`. It is read-only and
does not start a Raft read or consensus round; freshness is proven from the
replica's applied-state lease. That separation keeps the packet path and the
Raft leader independent of browser polling.
