# Complete local name-service status

NS `OP_STATUS` (9) accepts arg0 zero with no memory or connection attachment.
The response moves one `CNSSTAT1` buffer and returns its exact meaningful byte
length. Malformed requests, allocation/mapping failure and rejected preparation
return `ERR_INVALID` (-2), never a partial catalog. The retired `NSST` page is
unsupported; there is no compatibility decoder or clamped-prefix fallback.
NS interface 2, CNSGEN01 generation requests, runtime ABI 3 and persistent
formats are unchanged.

The observation includes every active local publication, in unsigned-byte
lexicographic name order. Generation tombstones are excluded. The registered
count equals the encoded row count. Pending counts retained lookup requests,
including several requests for the same missing name, rather than distinct
missing names. These are reactor-local diagnostic observations, not
linearizable distributed reads, application grants or physical-cleanup proofs.

All integers are little-endian. The fixed header is 24 bytes:

| Offset | Field |
| --- | --- |
| 0 | Eight-byte magic `CNSSTAT1` |
| 8 | u16 format version, exactly 1 |
| 10 | u16 header length, exactly 24 |
| 12 | u32 total meaningful length, exactly the supplied buffer length |
| 16 | u32 active row count, at most 64 |
| 20 | u32 pending lookup-request count |

Each row is `[name_length:u32][generation:u64][name bytes]`. Names contain
1 through 256 bytes; binary bytes and the final byte remain part of the
identity. Generations are positive and at most i64::MAX. Names are unique and
strictly ordered. The maximum complete snapshot is **17,176 bytes**:
24 + 64 * (12 + 256). This fits the complete supported publication set;
oversized, truncated, extended, unsorted, duplicate or inconsistent records
reject before exposing any row. Encoding plans the whole set before fallible
allocation. Decoding borrows the validated buffer without allocating rows.

NS adopts the complete incoming transaction exactly once at its receive ABI
boundary. `StatusReply` retains output bytes, owning memory/mapping and reply
authority through preparation and consuming `reply_move`. Ordinary rejection
uses the existing runtime disposal policy; rejected publication is not replayed.
Mapping cleanup precedes unused reply authority on abandonment. This adds no
new custody, physical-release policy or resource-readiness loop.

HTTP reads the exact moved length within both the protocol bound and memory
capacity, validates the entire snapshot, then emits `ns.registered`,
`ns.pending` and the complete `ns.services` array. Unavailable or rejected
observations render `ns:null`. Printable names retain their full bytes; binary
names render as `hex:` followed by all bytes. The status listing itself does
not authorize lookup of protected publications.

Host tests cover the golden and empty sets, the complete 64-row ceiling,
255/256-byte binary names, invalid generations/order, every truncated prefix,
inconsistent headers/rows, trailers, oversize and retired pages. The native
HTTP fixture publishes 17 names sharing their first 255 bytes, retires one,
requires a multi-page snapshot with all 16 remaining names, verifies multiple
pending requests for one name and rejects nonzero arg0 or attachments. Host
HTTP liveness probes check those complete names and exact count after initial,
EOF, abortive-close and idle-client requests on both architectures.

Execution results and qualification limits are recorded in
[batch S08b2c4d](../reports/audits/2026-10-15-ns-status-snapshot.md).
