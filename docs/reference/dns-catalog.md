# Complete DNS catalog observations

`dns::OP_CATALOG` returns one locally applied active catalog. It is diagnostic
metadata, separate from persistent catalog-v18, Raft pair-v3 and linearizable
lookup. The request has `arg0 = 0` and no attachments. Invalid requests consume
their unique received attachments and return `ERR_BAD_OPCODE`.

The only reply grammar is `CDNSCAT1`, version 1. Its exact meaningful length
is returned as the scalar result. All integers are little-endian.

| Header offset | Field |
| --- | --- |
| 0 | Eight-byte `CDNSCAT1` magic |
| 8 | Version `u16`, exactly 1 |
| 10 | Header length `u16`, exactly 24 |
| 12 | Total length `u32`, exactly the supplied input length |
| 16 | Record count `u32`, exactly the number of complete records |
| 20 | Four zero reserved bytes |

Each record is `name_len:u32 | node_len:u32 | generation:u64 | name | node`.
Name and node bytes must be nonempty; generation must be positive. Lengths are
full identities, with no u8 truncation. Repeated names are separate replica
rows, each preserving its node and service generation. Prepared, inactive and
tombstoned publications are excluded by the catalog producer.

The complete record set must fit **64 KiB**, including its header. Encoding
plans the entire set before fallible output allocation. The catalog borrows
its existing ordinary and replica registries through both passes; it does not
clone entries into a teardown/observation snapshot. These are existing userspace
mutex contexts, not new kernel guard or allocator qualification. An oversized
set returns `ERR_TOO_LARGE` instead of a prefix. Output allocation/mapping
rejection returns `ERR_RESOURCE_LIMIT`. No accepted publication is replayed.

The DNS receive adapter transfers the unique raw envelope once into an
`IncomingMessage` before the scalar attachment fallback guard. `CatalogReply`
owns reply authority, encoded bytes and typed memory/mapping backing through
preparation and consuming publication. Rejected moves return their payload
owner for the existing runtime cleanup policy. Abandonment releases output
before unused reply authority. This adds no kernel retirement receipt, custody,
physical retry or general unmap/close guarantee.

Decoding validates the entire count, each complete record, exact total length,
version, reserved fields and the absence of trailing data before exposing a
borrowed iterator. It allocates no row snapshot. The retired untagged
`count:u32`/u8-string page format is rejected. A bounded HTTP read rejects
lengths above its bound or returned backing; it never clamps a prefix for
another parser to accept.

## HTTP representation

Both node and cluster HTTP observations resolve one fresh owned DNS connection
per requested observation. An absent optional startup lookup is not cached
permanently, and a replaced service can be discovered on a later request. The
lookup uses ordinary pending-call readiness, with no background polling loop.
Existing RPC timeout/admission and producer-progress qualifications remain.

The node `/metrics` view renders:

```json
"dns": {
  "state": "leader",
  "term": 1,
  "catalog": {
    "schema": "charlotte.dns-catalog.v1",
    "count": 2,
    "entries": [
      {"name": "orders", "node": "node-a", "generation": 3},
      {"name": "orders", "node": "node-b", "generation": 4}
    ]
  },
  "registered_count": 2
}
```

The `entries` array preserves repeated names; there is no dictionary that can
collapse replica rows. If the catalog reply is unavailable or invalid,
`catalog` is `null`. A valid empty set has `count:0` and `entries:[]`.
`registered_count` comes independently from the status word and does not prove
that catalog observation succeeded. An unavailable DNS service renders `dns:null`.
Neither representation grants invocation authority or establishes a Raft read
barrier. Cluster JSON keeps its separate `charlotte.cluster.v2` grammar.

## Validation scope

Host controls cover a golden record, empty set, identities sharing a 255-byte
prefix, complete sets beyond one page, repeated replica names, exact 64 KiB
admission/rejection above it, malformed counts/lengths, reserved fields, retired
pages, truncation and trailing bytes. Catalog integration checks activation,
tombstones, per-replica generations and unchanged live state after output-limit
rejection. Existing owned-runtime tests cover returned-move rejection and
unique attachment cleanup; they do not inject every failure into the live DNS
service. Native HTTP controls require this exact schema/count and `alpha` row
on initial, EOF, abortive and idle recovery observations. See
[batch evidence](../reports/audits/2026-10-13-protocol-simplification-dns-catalog.md)
for actual guest results and retained failed runs.
