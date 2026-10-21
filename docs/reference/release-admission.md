# Release admission results

A successful release admission returns `CRLADM01`, captured by the catalog apply
operation while its complete component plan is still serialized. The release
generation is independent of each component deployment generation. Consumers must
use the component values when observing rollout readiness.

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 8 | `CRLADM01` |
| 8 | 32 | SHA-256 of the exact signed `CRELEASE` envelope |
| 40 | 8 | Release generation, little endian |
| 48 | 2 | Component count, little endian, 1..=16 |
| 50 | 6 | Zero reserved bytes |
| 56 | 8 × count | Component deployment generations in signed-descriptor order |

The length is exactly `56 + 8 × count`, at most 184 bytes. Every generation is
positive and at most `i64::MAX`. Missing, truncated, extended, mistagged or
noncanonical results reject. A consumer with the original envelope also checks
its digest and descriptor count before pairing names with generations. The digest
binds the record to a request; it supplies no peer authentication or signature.

DNS `OP_DEPLOY_RELEASE` (20) and clusterctl `OP_NOTIFY_RELEASE` (9) move receipt
memory on success and return its byte length. Errors are negative scalar results.
The earlier positive scalar release result is retired. Operations admission uses
the same catalog result but validates it completely before returning its existing
scalar release generation to its caller. Persistent records and signed envelope
formats are unchanged.

The release relay uses request tag `0x2b` and reply tag `0x2c`. The request body
is `session:u64 | request_id:u64 | caller_length:u8 | caller | envelope_length:u16 |
envelope`, with nonempty complete caller identity and the existing 3,584-byte
envelope bound. The reply body is `session:u64 | request_id:u64 | result_length:u16 |
result`. All integers are little endian. The result is a complete receipt or an
exact eight-byte negative error. Exact frame lengths are required; retired
`0x19/0x1a` frames reject. The follower checks expected leader, session and
nonreused request ID, then validates the receipt against its retained request.

HTTP `POST /v1/releases` returns HTTP 202 with one JSON field:
`{"admission":"<lowercase hex of complete CRLADM01>"}`. Errors retain the existing
HTTP/error-code mapping. Duplicate, unknown or missing fields, scalar release
successes and malformed hex reject. JSON whitespace is accepted; non-JSON Unicode
whitespace is preserved and rejected. The shared HTTP body reader retains any
additional separator or trailing data. CLI arguments
are unchanged. `release-notify` prints the validated body; `release-apply` borrows
component names from its retained envelope preparation and waits for each captured
generation. Older observations remain pending, matching ready generations
complete, and supersession or malformed HTTP-200 status stops without POST replay.

An exact release retry returns a receipt only if every current component still
has the exact signed descriptor bytes. It may report the current generation of
an unchanged descriptor whose placement was reconciled. A missing or superseded
component rejects without restoring older desired state. No receipt fields are
added to catalog snapshots: the retry check reads existing signed descriptors and
generations under the same serialized maps.

Local DNS admits reply memory/mapping and reply authority before submission or
relay; remote DNS prepares bounded terminal bytes before Raft submission. Pending
storage reserves before mutation. Commit, term change, timeout and redirection
consume the existing owner. No uncertain result authorizes replay. Clusterctl
forwards the validated owned memory, and deployd maps it through existing owned
runtime calls. These are scheduled userspace service contexts without kernel
subsystem guards. See the existing [C18 row](cleanup-recovery.md) and
[audit evidence](../reports/audits/2026-10-21-security-release-generation.md).

Host readiness still uses status polling because this HTTP interface has no
subscription. Apply captures one checked monotonic deadline before preparation
and carries it through admission and readiness. Connect, partial writes and reads
use the remaining budget; ingress endpoints must be numeric IPv4 or bracketed
IPv6 with explicit nonzero ports. Blocking host filesystem calls and stdout are
not preempted. See [tooling inputs](tooling-inputs.md).

A fetch/launch failure may leave committed work unready; coordinated rollback
and richer rollout policy remain controller work. Live allocation/mapping/wait
rejection, follower failure races and wider ownership contexts need further
qualification. Production trust remains disabled.
