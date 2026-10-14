# Complete bounded NS status observations

Batch S08b2c4d, 2026-10-15. Baseline `3a2462a8`.
Scope: NS OP_STATUS grammar, owning request/reply preparation, HTTP observation
and coordinated host/native fixtures and living documentation.

## Source-backed finding and replacement

The removed NSST writer clamped a valid 256-byte name to 255, stopped at a
4 KiB page boundary, but reported registry size including tombstones and omitted
rows. Its pending count measured distinct names rather than retained requests.
The HTTP reader accepted a readable prefix. This could present an incomplete
catalog as a complete one without preserving identity.

`charlotte_protocol_ns::snapshot` now defines only CNSSTAT1: a 24-byte tagged
header with exact total length, active count and retained lookup-request count,
followed by full-name/positive-generation rows. All 64 supported live publications
fit the 17,176-byte ceiling with complete 256-byte names. Encoding plans the
entire ordered set before fallible allocation. Decoding validates every row,
count, length, name order and trailer before exposing borrowed records. Retired
NSST, partial/extended, oversized, invalid-generation and duplicate/unsorted
observations reject; neither side has a compatibility or shortening path.

NS consumes the complete incoming transaction once before any scalar fallback.
Its StatusReply retains bytes, memory/mapping and reply authority through
allocation, copy, explicit unmap and consuming reply_move. Rejected submission
returns its memory owner to existing runtime disposal without replay. HTTP
retains owned returned memory, checks the exact reported length against the
protocol bound and backing capacity, then validates the whole snapshot before
rendering complete names. Unavailable/rejected observations render ns:null.
Generation tombstones are excluded; pending counts requests, including multiple
waiters for one name. These remain diagnostic local observations, not application
grants, distributed linearizable reads or physical-cleanup proofs.

NS interface 2, CNSGEN01, runtime ABI 3 and persistence grammars are unchanged.
The retired tag is not reused. The unused permissive HTTP u32 field reader and
old NS status SDK offsets are removed. Authorization-audit pages and other
services' status formats are distinct boundaries and remain unchanged.

## Validation contract

Four new host codec tests cover golden/empty records, all 64 full-length binary
names, 255-byte names, invalid lengths/generations/order, every truncated prefix,
inconsistent counts/framing, retired pages, trailers and oversize.

The opt-in native HTTP fixture publishes 17 256-byte names sharing their first
255 bytes and retires the seventeenth through CNSGEN01. It requires a current
snapshot larger than one page, exactly 16 matching live rows, complete final
identity bytes and no retired row. Two accepted pending lookups for one missing
name verify request counts; their subsequent publication resolves both calls
and each returned connection is explicitly closed. Nonzero arg0 and copied
memory/connection attachments reject with ERR_INVALID and no returned resources;
the original copied memory remains unloaned and closes explicitly. The fresh captured ordinary
fixture namespace retains its endpoint and stable no-role test principal so host HTTP requests verify the
same complete names after initial, EOF, abortive-close and idle-client recovery.

Allocation/map failure, cancellation and rejected returned-memory moves rely on
the existing typed runtime owner policy; this batch does not inject each failure
into the NS handler or claim a new physical-release qualification. No routine
resource polling or new readiness registry is added. Native fixture startup
status polling remains an explicitly bounded existing diagnostic fixture.

## Qualification limits

Markdown contracts, the simplification ledger, C18 cleanup evidence, AGENTS and
the LaTeX networking chapter change together. Historical reports retain their
original observations. Remaining local packed/public APIs, raw adapters,
metadata/outer IRQ/Drop contexts and physical cleanup remain separate work.
Historical IPC exact-exit and subsequent Raft persistence attribution remain
open; passing execution here cannot retrospectively establish their cause.

## Preserved initial fixture failure

The initial fixtures used ASID zero for the exact-generation retirement request.
Both architectures rejected it with ERR_NOT_FOUND (-1), while the fixture expected
generation 1: ARM at 15.623938 s, 18 passed / 1 failed / 0 pending; x86 at
18.141408 s, 16 passed / 1 failed / 0 pending. The guard rejects an invalid
DomainIdentity (ASID zero); this is a fixture authorization mismatch, not evidence
that retirement completed. No host HTTP round trip was reached.

The corrected fixture prepares a fresh ordinary captured root and one explicit
stable test principal with zero manager/admin roles before verifier/request
publication. The existing namespace table retains the root/endpoint for the host
checks; neither live authority nor boot policy is overwritten. Registration and
retirement now use the same captured ordinary caller and authenticated envelope.
The production authorization guard is unchanged.

Initial kernel hashes: ARM
`b2f27b563a9b19029f22ba7923bc46f1b24541fcaa4348c6fe332f7d93d6d7c2`;
x86
`4a750ed67c53f3468149b0759171d5b19b94d1d066e1e31ef85c89df5792ab9e`. Corrected results
remain separate from these failures.

## Corrected execution results

Host suite: **483 passed, 0 failed, 4 ignored**, plus existing signing, boot,
input and object-store controls. Strict service Clippy passed on both targets;
strict kernel Clippy passed with acpi/virtio_net_test/http_net_test against both
current signed bundles after the fixture correction. Workspace rustfmt, shell
syntax, Python compilation/tooling-contract controls and diff checks passed.
Both service builds finished and NS/DNS/Echo/HTTP independently verified before
any guest launched. No later signing or bundle mutation occurred during guests.

Both corrected native runs used fresh v4 stores, four LPs and 300-second budgets:

| Result | ARM | x86 |
| --- | --- | --- |
| Complete native self-test suite | **19/0/0**, 14.614014 s, bitmap `0x23ffff` | **17/0/0**, 18.388705 s, bitmap `0x23bbff` |
| Complete NS observation | 34 active rows, 14.611700 s | 32 active rows, 18.387771 s |
| Host node status after pending-request publication | registered 35, pending 0, 35 service rows, 16 full-length names | registered 33, pending 0, 33 service rows, 16 full-length names |
| Runner exit | 0 | 0 |

Each native observation included exactly the 16 live 256-byte names and excluded
the retired seventeenth. Host checks required the entire expected name set and
registered count equal to the array length on initial, EOF, abortive-close and
idle-client recovery requests. Both also passed the existing DNS catalog/cluster
view and deployment malformed/oversized, EOF, abortive reply, idle deadline and
trickled-body controls (trickle rejected in 30.07 s ARM / 30.18 s x86).
Transport resets occurred in the existing bounded liveness recovery probes and
recovered within their three-attempt policy; malformed HTTP/JSON is not retried.
These are observations of recovery, not an attribution of the resets.

All **128** scheduled IPC claim/endpoint close episodes in these corrected
guests retained the original exact exit-watch requirement and passed: ARM
10.215664 s; x86 13.679564 s. No historical exact-exit timeout or subsequent
Raft persistence error was observed. This does not close their historical cause
or qualify physical root/stack release from return markers or absence. Native
runners also checked assembly section permissions (1 ARM / 249 x86 entries).

## Executed artifact identity

| Signed payload | Bytes | SHA-256 |
| --- | --- | --- |
| ARM ns | 178,008 | `6c473655f837bc398f1108ef6300038f5635a0b8d66286345a02506a04712eaa` |
| ARM dns | 920,464 | `7aa05784ebc9074e0f6db485bfdd384eda893d4e4bc41618e46409a4374a4693` |
| ARM echo | 96,152 | `32b3fb44455924f6ea17b9e9e9534407e32376e5933ffcbb0bee5ac0f2bbd406` |
| ARM httpd | 164,800 | `f90f528bf1533f2f7163a65f01d94d0556bda107abaf590c51d7512fb98b3787` |
| x86 ns | 121,296 | `86f16dc65605950eac1897a54b1c986b31c2244f1b2b79c68f74c66e1be4df28` |
| x86 dns | 942,512 | `969af8bd0f5faa3abd3454dc7a17dba6c4c06ba437652b449d167826dd1b1827` |
| x86 echo | 32,264 | `501eba2ae7823687cbb3168a67857536aa2013c5d38671090b1265f98b395fef` |
| x86 httpd | 100,024 | `5b245557c58de73e42ec65c3628ff30a14d13ba9261145a83090f0e4a6e5f535` |

Corrected executed kernel SHA-256 identities: ARM
`f2bfaa8d2b7b1989a21feaaa7a636ad49f3698418f562656858a8c8a2f14691d`;
x86 `0be71738e9f7a82c20c087621bc6d2228274fa89c7189d44c1bac43349f73c53`.
These are the captured runner payloads, not later build-cache observations.
Initial failed fixture kernel identities remain separately recorded above.

The 214-page manual rebuilt through two successful LaTeX passes; updated page
92 was rendered and visually inspected without clipping or overlap. PDF
SHA-256: `a06114b19fed512d1d96c888dc5e92b44bddbf8dd10fc9ebabf713dd6dc04122`.
All published evidence is inline or version-controlled, with no temporary logs
or ignored build artifacts serving as evidence links.
