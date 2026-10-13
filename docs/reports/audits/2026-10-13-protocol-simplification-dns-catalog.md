# S08b2b: complete DNS catalog observations

Date: 2026-10-13. Baseline: local `ce558d6f`, clean tree. This is one internal
API/wire simplification batch, not closure of a numbered security finding.
Current contract: [DNS catalog observations](../../reference/dns-catalog.md).

## Replaced behavior

The old DNS `OP_CATALOG` writer allocated one page, wrote the complete catalog
count, then stopped when rows no longer fit. It shortened names/nodes to 255
bytes. HTTP accepted readable prefixes and used an empty entries dictionary
with the status count when decoding failed. Repeated replica names became
repeated JSON object keys. There was no tagged complete-set grammar.

The sole current representation is `CDNSCAT1`, version 1, with an exact
24-byte header: total length, exact row count and zero reserved bytes. Records
carry full u32 name/node lengths and positive service generations. Encoding
plans the whole set before one fallible output allocation and rejects above
64 KiB; decoding validates the entire set before exposing an allocation-free
borrowed iterator. Retired untagged pages, malformed counts, truncated records
and trailing bytes reject. Persistent catalog-v18 and Raft pair-v3 are unchanged.

`NameCatalog::catalog_result` replaces the unused public cloned-entry API. It
borrows the existing ordinary and replica registries through both encoding
passes, preserving only active publications. Existing userspace lock order and
allocation context remain; no new kernel guard/allocator qualification is
claimed. The local receive branch adopts the complete unique raw envelope
once before any scalar attachment fallback guard. `CatalogReply` contains the
reply token, encoded bytes and typed memory/mapping backing through preparation
and consuming move. Invalid request attachments are consumed. Output-limit,
invalid-record and resource-admission failures remain distinct. Rejected reply
moves return the memory owner; the existing runtime fallback disposes it
without replay. C18 records the containing transaction and its limits.

HTTP reads the exact meaningful byte length, rejects lengths above its bound
or backing, and decodes the complete set. Node JSON uses the schema
`charlotte.dns-catalog.v1` and an entries array, retaining replica rows. A
rejected/unavailable catalog is null; a valid empty set is explicitly empty.
Status `registered_count` remains separate observation, not proof of a dump.

Native qualification exposed a startup-only optional DNS connection cache:
node metrics could keep reporting DNS absent after its publication, while the
cluster route already had a fresh-lookup fallback. Both routes now use one
fresh owned lookup per requested observation, removing the cached optional
alias/fallback. The accepted lookup uses existing pending-call readiness;
there is no background polling, replay or new notification registry. These
reads remain local diagnostics, not linearizable invocation authority.

## Source and documentation coverage

In-tree catalog producers/consumers are DNS, its catalog adapter, the shared
codec and HTTP. The retired header constant, cloned `entries()` API, unsafe
one-page writer, partial HTTP parser, unused u64 parser helper and cached DNS
field are removed. Both architecture HTTP runners invoke the same host
catalog/liveness controls. No independent consumer of the old dictionary was
found. AGENTS, testing guide, catalog/HTTP references, API ledger, LaTeX and the
tracked manual are updated. The manual builds to 212 pages; changed paragraph
and transitions on pages 89--91 were rendered and visually reviewed. Existing
unrelated long-link warnings are not a claim of whole-manual layout repair.

## Failed runs retained separately

Initial service builds succeed but warn about the obsolete HTTP `u64_at`
helper. It is removed before strict checks; final service and native bundles
are rebuilt. No warning suppression is added.

### Initial ARM HTTP catalog control fails

Instance `s08b2b-http-arm`, four LPs, fresh storage, HTTP/deployment ports
19128/19129, --http-test --timeout 240. Runner exits **1**. The guest itself
finishes **19/0/0** at **17.154001 s** (0x23ffff).

The fixture records DNS serving at 14.279740 s, leader at 17.021044 s,
registration result 1 at 17.152329 s and alpha catalog publication at
17.152603 s. The first host node-metrics catalog check raises
`TypeError: 'NoneType' object is not subscriptable` at
`test-http-keyhole-liveness.py:45`, reading `document["dns"]["catalog"]`.
Node JSON contains `dns:null` despite that fixture publication. The script
reports both keyholes unvalidated and retains failure; it does not replace
this with an empty table or retry malformed JSON. Deployment controls pass,
including trickle rejection in **30.13 s**, with bounded reset retries.

The startup-cache source path is removed, and final fresh-resolution controls
pass below. This is not proof of a DNS retirement/restart race, all optional
source availability, or repaired generic TCP teardown.

### Initial x86 NIC startup fails before HTTP

Instance `s08b2b-http-x86`, four LPs, fresh storage, ports 19130/19131,
same HTTP feature profile and --timeout 240. Runner exits **1**.
At **12.711459 s**, NIC ASID 106 rejects a three-page bidirectional DMA map:
`DmaMapError { status: 14, phase: 3, cause: 12 }`. Startup records stage 4,
`DmaDomainMap`, detail `0xc030e`; subsequent three-page and 128-page maps retain
the same diagnostic. Phase 3 is backend admission/maintenance and cause 12
is OperationInFlight. The diagnostic does not establish whether work started
and cannot authorize replay; the existing failure owner retains that uncertainty.

The router remains at stage 1. At **43.224586 s**, final guest result is
**15 passed / 1 failed / 1 pending**, passed bitmap 0x1bbff, failed net bitmap
0x20000, pending HTTP bitmap 0x200000. Panic at `self_test/el0_net.rs:105:22`:
`self-test deadline expired while waiting for EL0 frame router startup`.
HTTP never reaches listening or catalog validation. The host ultimately
reports an unvalidated keyhole. The final passing run changes DNS/HTTP artifacts
and timing, so it is not an identical-artifact causal comparison. The exact
contending owner is not established; this failure remains unresolved alongside
the earlier DMA-map incidents.

## Final validation

- Initial complete host run: **445 passed / 0 failed / 4 ignored**, including
  existing Python/signing controls. The changed service package then repeats
  **114/0/0**, adding the catalog integration control and replica observations.
  Current distinct host coverage is **446/0/4**; the entire suite is not rerun
  merely to repeat unchanged packages after that test-only addition.
- Codec controls cover a golden vector, empty set, full identities sharing a
  255-byte prefix, sets beyond one page, replica rows, exact 64 KiB admission,
  rejection above the ceiling, invalid generation/identity, every truncated
  prefix, wrong counts/version/header/reserved fields and trailing data.
- Catalog integration checks invisible preparation, activation, output beyond
  one page, unchanged live snapshot after output-limit rejection, tombstone
  removal and per-node replica generations. Runtime tests cover unique
  IncomingMessage cleanup, failed reply moves returning payload ownership and
  mapping-unmap failure preserving its owner. Live DNS allocation/mapping/
  cancellation failure injection is not performed; these existing owner tests
  do not qualify uncertain kernel physical cleanup or every outer context.
- Both signed bundles rebuild. Strict service Clippy (`--bins`) and kernel
  Clippy with `-D warnings` pass on both architectures. Actual HTTP feature
  profile: `acpi,virtio_net_test,tcpip_net_test,http_net_test`; native runners
  enforce executable/read-only assembly sections before boot.
- Rust formatting, source diff checks, shell syntax and Python syntax pass.

### Final native ARM and x86 HTTP

Final instances `s08b2b-final-http-arm/x86`, four LPs each, fresh storage,
--http-test --timeout 240. ARM ports **19132/19133**; x86 **19134/19135**.
Both runners exit **0**. ARM finishes **19/0/0** at **16.240356 s**
(0x23ffff); x86 finishes **17/0/0** at **23.280680 s** (0x23bbff).

Both node JSON captures retain exactly one alpha row:
`schema=charlotte.dns-catalog.v1`, `count=1`, `entries=[{name:alpha,
node:charlotte:42e21edb,generation:1}]`, independent `registered_count=1`.
The host requires the schema, array/count, nonempty identities, positive
generation and alpha row on initial, EOF, abortive and idle recovery requests.
All eleven node/deployment controls pass. Deployment trickle rejection is
**30.14 s ARM / 30.15 s x86**. Both runs still require ConnectionResetError
retries up to **2/2** for node and deployment recovery. Passing bounded recovery
is not clean transport delivery, exactly-once invocation or resolved TCP teardown.
Synchronous host probes remain outside a hard runner wall-clock deadline;
--timeout controls the existing loop budget. No wait budget is extended here.

## Remaining work

S08b2c retains lossy u8 DNS remote call/query/registration writers, scalar/named
entry-point variants, operational release-tail policies, physical-address
queries and mailbox adapters. Current connector-binding and serialized fixture
ownership/IRQ contexts require coherent replacements before removal. Earlier
ARM stalls, DMA-map contention, HTTP intermittence, shared-CQ per-waiter
readiness and production trust/custody remain unresolved. No new retirement
registry, cleanup receipt or hardware/physical retry authority is added.

## SHA-256 artifacts

Read-only `fs-inspect.py::ObjectStore.read_object` checks the retained v4 images'
content hashes and recovers the exact staged DNS/HTTP artifacts for both failed
and final runs. Object IDs are `0xfffe2f18f4769a5c` (DNS) and
`0xfffe34c925b26453` (HTTP). Their byte lengths match runner staging records;
SHA-256 values are retained inline below. Kernel and manual hashes describe
their separate artifacts, not a claim that initial and final service bundles
are identical.

| Artifact | SHA-256 |
| --- | --- |
| Initial/final ARM HTTP kernel | `335886d2f9e1bf90e52a8555d97dab426efac4893ccdfefe3a784a836e8a30d5` |
| Initial failed x86 HTTP kernel | `4d1839f334d51fd5d6b1b1c9faebf26c391ec12bf0851f8e0e480d3cc5742026` |
| Final x86 HTTP kernel | `4c0961d55b418314040df21883eede60548901adf3a6446d497b2ad2a69a5b35` |
| Initial signed ARM DNS | `d83123042998141d92c43051f6954c178832f9e661947002199a0be8b6fd37e2` |
| Initial signed ARM HTTP | `c0ad385fa02c0c2f10544e6c4cf475e8f0e30b6adc202956de65cd6795481297` |
| Initial signed x86 DNS | `2c9ea1eb56ff35b25453ba09e61b6e9495b355c7c4dae2bf1a5eebe615cdcbbf` |
| Initial signed x86 HTTP | `2599654c04ad71591c88e2790602263990b4fbbfa651828e4c1b075863262d15` |
| Final signed ARM DNS | `d69183145cf25f83828a28bb67de889fad5b803f090199e33545de122f36666d` |
| Final signed ARM HTTP | `db74350977ff0d88a4eb6a81c36b2b09a8938d6995b85311856d847487b68313` |
| Final signed x86 DNS | `a58fa48fd09ee00758eef95a95f43d963e1780d9029cd589afc8b4cec336dcaf` |
| Final signed x86 HTTP | `d692a01215845acba94daefac2bb092b7abd3b9f95433b2a453f2a2d7f493c4d` |
| Final 212-page manual | `af601194d3f25be45deec02b840a546993e4407e94853f03fac96592bc146033` |
