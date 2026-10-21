# Internal API and protocol simplification

Started 2026-10-13. Internal contracts have no backward-compatibility requirement.
Each boundary should expose one current grammar/API with explicit required
fields. Update writers, consumers, SDKs, scripts, fixtures and living Markdown
and LaTeX documentation together. Historical audit reports retain their original
observations; link current contracts instead of rewriting their evidence.

## Removal and naming order

1. Find every active producer/consumer, including assembly fixtures and stored
   records. Distinguish old-version support from present hardware, boot-fixture
   ownership contexts and external protocol negotiation.
2. Select the current contract. Remove old parsers, overloads, aliases and
   compatibility defaults; migrate callers in the same batch. Retired inputs
   reject explicitly, even if their signatures are valid. Do not reinterpret
   an unsupported/corrupt record as another format.
3. Compile and test the complete batch before changing magic/version names.
   Keeping the latest magic is sufficient. Any optional reset to version 1
   is a later, separately validated rename after the superseded grammar is gone;
   do not reuse a retired magic while its old interpretation remains accepted.
4. Update living documentation and retain concise execution evidence. Rebuild
   signed artifacts and regenerate obsolete deployment/fixture files as needed;
   never silently erase a developer's stored data.

## Batch ledger

| Batch | Current contract | Scope / status |
| --- | --- | --- |
| S01: deployment descriptors | `CDEPLOY6`, version 6, exact 152-byte header | Complete and validated. Original S01 removed v1–v4 decoding, execution/shutdown/placement defaults, legacy constants, redundant decoded version field and short-header admission. Kernel/runtime, HTTP and operations pickup use the same current header bound. Signed release/trust documentation, Markdown guides and the LaTeX/PDF manual reflect the sole grammar. See [batch evidence](../reports/audits/2026-10-13-protocol-simplification-deployment.md). |
| S02: discovery advertisements | EtherType `0x88B6`, version 1, complete declared-length advertisement | Complete; host tests and a cross-architecture guest repeat passed, with the initial completion-wait fixture failure retained separately. Removes base-only and optional cluster-field APIs and unknown-role fallback. Both probes and responses use one typed payload; old zero-length headers, truncation and invalid roles reject before learning/responding. Ethernet padding cannot synthesize missing fields. See [contract](discovery-protocol.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-discovery.md). |
| S03: Raft persistence | Two-object pair: `CRFTSTA3` state and `CRFTLOG3` snapshot/log | Original S03 baseline: 402 host tests and four corrected fresh/reuse guest runs passed; the initial failed pair and timer-fixture correction are retained separately. Removes term/vote-only decode, split-snapshot migration, raw-entry fallback, independent store constructors and optional disk-to-memory fallback. Joint opening rejects incomplete/unsupported records; the transport uses owned runtime resources. S04b2b advanced to pair-v2; S08b1 advances to v3 for explicit registration generations. See [contract](raft-persistence.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-raft.md). |
| S04a: catalog snapshots/query | `CATALOGI` version 18; complete deployment query record | Original S04a baseline: 410 host tests and four matching-architecture deployment guests passed. Current-only snapshots and mandatory query sections replace compatibility decoding; restore rejection precedes Raft startup/log/membership publication. The initial fixture and mixed-architecture failures are retained separately. The later target-selection change retires descriptor-less publication/restore; later S04b2a/b remove packed callers and redundant fields. S08b1 rejects older registration stores with catalog-v18. See [contract](catalog-snapshots.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-catalog.md). |
| S04b1: scoped launch/retirement | Four-argument syscall 76 returns ASID and verified principal; one owning runtime API | Complete: 411 host tests and final ARM 20/0/0 / x86 18/0/0 signed-ingress guests pass. Removes packed scoped names, ASID-only/named wrappers, caller-chosen retirement names, the unused emitter and singleton retirement. The initial x86 timer-fixture failure and its isolated-worker correction are retained separately. See [contract](scoped-deployment-launch.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-launch.md). |
| Architecture prerequisite | CDEPLOY6 signed variants; exact per-replica pins; kernel-reported execution profiles | Complete: 418 host tests, ARM/x86 central-store rollout and linked mixed guests (22/0/0 and 20/0/0). Unknown/incompatible profiles reject; native launch rechecks target and image. Fixture corrections and the authorized remote-invocation gap are retained in [evidence](../reports/audits/2026-10-13-execution-targets.md). |
| S04b2a: deployment entry points | Signed launch only; full-name controller status; retired opcodes reject | Complete: 419 host tests, strict service/kernel Clippy on both architectures, ARM 22/0/0 and x86 20/0/0 linked guests. Descriptor-less syscall 64 and runtime/SDK wrappers, controller deploy opcode 2, DNS packed deploy/query opcodes 8/9 and optional DEP1 suffix are removed. Controller interface version 3 uses moved full names for status and rollout. See [contract](deployment-administration.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-administration.md). |
| S04b2b: lean deployment records | Descriptor-only admission; CDEPQR01 queries; catalog-v18; Raft pair-v3; CLOBSV2 observations | Original S04b2b baseline: 425 host tests, strict service/kernel Clippy and passing mixed/S3 guests on both architectures. Initial DMA startup failure remains separately recorded and unresolved. Removes redundant name/object/node/digest fields from admission and object ID/primary/variant-zero digest from stored and queried deployments. Exact signed replica pins remain; S08b1 advances persistence tags again. Retired formats reject, including log-only old stores. See [contract](catalog-snapshots.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-records.md). |
| S05: canonical IPC receive | Twelve-register envelope on 71/72/73; runtime ABI 2 | Complete: 427 host tests, strict Clippy, passing ARM/x86 linked and S3/HTTP guests, and ARM security qualification. Removes nine-register receive syscalls 22/27/53 and duplicate authenticated SDK/runtime names. Services and both raw fixture sets use the canonical envelope; signed variant matching rejects ABI 1. Host and native verification is recorded in the [contract](ipc-receive.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-ipc-receive.md). |
| S06: checked CQ waits | Syscall 88 with explicit timed flag; typed outcomes; runtime ABI 3 | Complete: 430 host tests, two Sitas host tests, strict Clippy, final ARM 22/0/0 / x86 20/0/0 linked guests and ARM 20/0/0 / x86 19/0/0 S3/HTTP guests. Removes unchecked 12/42 SDK/kernel adapters and ignored count thresholds. Ordinary reactors/backoff fail closed; periodic watchdogs and the sibling Sitas backend retain their deadlines. An intermittent ARM stall remains unresolved; failed runs and the pre-clock security qualification are preserved in the [contract](cq-wait.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-cq-wait.md). |
| S07: tooling/export aliases | File-only signing keys; repeatable cluster-service table; launch-owned socket capacity | Complete: 430 host tests, six tooling boundary tests, 18 exported SDK signer tests, strict Clippy and ARM 19/0/0 / x86 17/0/0 table/HTTP guests. Removes public-fixture argv signing, paired service-name flag, singleton build-variable fallback/two parsers, and unused socket-capacity export. Empty names and retired variables reject explicitly. Current platform/named policies remain distinct. Failed validations and qualification limits are retained in the [contract](tooling-inputs.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-tooling.md). |
| S08a: object-store metadata | COBJSTR4 / CCOBJHD4, explicit owners and fixed records; blank-only formatting | Complete: 436 host tests, five image/inspector tests, strict Clippy, ARM 20/0/0 security/HTTP and x86 17/0/0 blank-store guests. Retired native fixture rejects with selection error and unchanged image. Removes owner-zero public access, extensible header-length acceptance, synthesized empty directory records and invalid/layout reformatting. No migration or physical quiescence claim. See [contract](object-store-format.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-object-store.md). |
| S08b1: registration / S3 profiles | Explicit registration generation; catalog-v18 / pair-v3; CHS3PF1 launch memory only | Complete: 439 host tests, strict Clippy, ARM 22/0/0 / x86 20/0/0 linked guests and ARM 20/0/0 / x86 19/0/0 TLS ingress. Removes suffix-free registration/separate encoder, S3 manifest fallback/exports/defaults/duplicate validator, and unused signature-range alias. First x86 EOF recovery failed; unchanged-source repeat passes with reset qualifications retained. See [catalog contract](catalog-snapshots.md), [S3 configuration](s3-client.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-registration-profiles.md). |
| S08b2a: Kafka authentication profile | `CHKAFP7`, version 7, exact 112-byte header and closed authentication choice | Complete: 441 host tests, strict service/kernel Clippy on both architectures and ARM 20/0/0 native TLS/SCRAM/mTLS Kafka leader-failure recovery. Removes section iteration, optional/critical extensions, the temporary credential allocation and unused packed-name export. Unknown/retired modes reject; existing capability and zeroizing owners remain. Native Kafka execution on x86 and broader audit gaps remain unqualified. See [contract](kafka-client.md#current-profile-wire-contract) and [evidence](../reports/audits/2026-10-13-protocol-simplification-kafka-profile.md). |
| S08b2b: complete DNS catalog observations | `CDNSCAT1`, exact 24-byte header, complete set bounded to 64 KiB | Complete: 446 current host tests, strict service/kernel Clippy on both architectures, ARM 19/0/0 and x86 17/0/0 HTTP guests with exact catalog and EOF/abortive/idle controls. Removes the untagged one-page writer, identity truncation, cloned-entry API, prefix parser, invented empty fallback and startup-only DNS connection cache. First ARM catalog-null failure and first x86 NIC DMA-map failure remain separately recorded; the latter is unresolved. See [contract](dns-catalog.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-dns-catalog.md). |
| S08b2c1: correlated DNS query relay | `0x24/0x25`, exact u32 identity lengths bounded to 4 KiB each | Complete: 449 host tests, strict service/kernel Clippy on both architectures and ARM 22/0/0 / x86 20/0/0 linked deployment guests. Removes lossy u8 query identities and infallible codec allocations; retired query tags reject. Request preparation precedes pending publication with the original deadline; source/session/query/leader/generation fences remain. See [contract](dns-query-relay.md) and [evidence](../reports/audits/2026-10-14-protocol-simplification-dns-query-relay.md). |
| S08b2c2: generation-fenced DNS invocation | `0x26/0x27`, complete bounded u32 identities; exact fixed reply; prepared reply owner | Complete with the DMA-map readiness follow-up: 452 host tests, final services repeat 120/0/0, signed builds and strict service/kernel Clippy on both architectures. Removes lossy request identities and prefix/trailer decoding; reply storage prepares before local admission and stays in its existing destination. Follower invocation retains the original query deadline; unrepresentable packed local names reject. Earlier codec-source linked pair passed; both original final-source pairs failed before DNS on NIC backend DMA-map rejection. The [map-readiness follow-up](../reports/audits/2026-10-14-dma-map-readiness.md) now passes two linked pairs (ARM 22/0/0 / x86 20/0/0) with unchanged signed final DNS bytes, including real ordinary/exclusive map waits. Failed captures and historical attribution remain separate. See [contract](dns-invocation-relay.md) and [evidence](../reports/audits/2026-10-14-protocol-simplification-dns-invocation-relay.md). |
| S08b2c3: correlated DNS registration/retraction | `0x28/0x29/0x2a`, exact u32 identities; captured peer/session/request replies | Complete: 457 host tests, strict service/kernel Clippy and signed builds on both architectures; final linked ARM 22/0/0 and x86 20/0/0, HTTP ARM 19/0/0 and x86 17/0/0 with all 11 recovery controls per guest. Removes truncated registration/retraction, name-only reply matching and missing-publication leader fallback. Existing phases retain owned connections/reply authority; terminal reply bytes prepare before Raft mutation. Full-name local publication/observation uses owned copied memory. First catalog-marker mismatch was corrected through the explicit event operation; second linked IPC exact-exit timeout and subsequent storage errors remain unresolved, with diagnostic-only probes added. See [contract](dns-registration-relay.md) and [retained batch evidence](../reports/audits/2026-10-14-protocol-simplification-dns-registration-relay.md). |
| S08b2c4a: owned full-name DNS invocation | DNS interface 4, mandatory CDNSCL01 memory frame; owned named lookup and invoke on one CQ | Complete: 467 host tests, strict service/kernel Clippy on both architectures, and final linked ARM 22/0/0 / x86 20/0/0. Replaces packed-name invocation and raw local call/connection stages with one retaining operation and typed client reply authority. Both RPC phases bind CQ readiness and carry one original kernel-monotonic deadline; accepted invocation binding rejection is uncertain, never replayable. Full-name generation unregister is replaced in S08b2c4b; other raw adapters remain candidates. See [contract](dns-local-invocation.md) and [evidence](../reports/audits/2026-10-14-protocol-simplification-dns-local-invocation.md). |
| S08b2c4b: full-name NS generation operations | NS interface 2, CNSGEN01 observation/retirement and owned CQ cleanup | Implemented with scoped native proof: 474 host tests and strict service/kernel Clippy on both architectures. The new DNS controls and three rollout generations execute on both guests; x86 finishes 20/0/0, while ARM retains a reproduced historical IPC exit-watch failure. Replaces packed observation/retirement and raw unregister handles, preserving captured principal/registrar/manager and exact-generation checks. Original deadlines and admission-versus-accepted ownership survive; failures never replay. Other packed APIs, raw watch/transport contexts and dead third-party NS binding retirement remain candidates. See [contract](name-service-generations.md) and [evidence](../reports/audits/2026-10-14-protocol-simplification-name-generations.md). |

`DEFAULT_SHUTDOWN_GRACE_MS` remains an explicit built-in launch policy choice;
it is not a parser fallback. Only current v6 signed bytes are accepted. Retired
`.cdep` files and their signed release/operations envelopes must be regenerated
from the current reviewed component plan. Persisted catalogs containing retired
descriptors cannot turn them into launch authority; storage-format removal is
separate work below.

S08b2c4c removes unused NS bearer-key opcodes 5/6, their SDK exports,
registration/deferred-lookup key fields and permissive key-memory decoder.
Unsupported requests now adopt their complete incoming owners before rejection.
Public bootstrap naming remains separate from authorized application discovery;
protected bindings cannot be resolved or replaced through it. The opt-in
security fixture now uses CNSGEN01 and separately rejects retired metadata and
bearer shapes. x86 gains the existing ARM security-suite runner option with
architecture-specific independently signed fixture roots. See
[removal evidence](../reports/audits/2026-10-15-ns-bearer-removal.md).

S08b2c4d replaces lossy NSST status pages with complete bounded CNSSTAT1
snapshots and one owning reply transaction; HTTP validates the complete set and
renders unavailable observations as null. Host/native full-name, multi-page,
pending-request-count and recovery controls pass; the initial invalid-caller
fixture failure remains separate. See
[status evidence](../reports/audits/2026-10-15-ns-status-snapshot.md).

## Remaining candidates

This is a source-backed starting inventory, not a completed whole-codebase audit.

| Order | Area | Candidate and coordinated work |
| --- | --- | --- |
| IPC readiness intermittence | Scheduled endpoint close / exit watch | After the initial 192 passing episodes, S08b2c4b reproduced the exact-exit timeout on both x86 and ARM with source/stage/live/staged diagnostics. The [investigation](../reports/audits/2026-10-14-ipc-close-exit-investigation.md) retains both failures. Later [park](../reports/audits/2026-10-14-park-deferred-delivery.md) and [first-dispatch](../reports/audits/2026-10-15-first-dispatch-delivery.md) corrections have negative controls and passing native suites, without establishing historical IPC or later Raft persistence causality. Preserve original deadlines, complete owners and exact completion requirements; broader C16/C18/G1/G7 remain open. See the [current review](../reports/audits/2026-10-15-audit-process-review.md) for separate unresolved failure families. |
| Remaining DMA contexts | Masked SVC / unmap / close / historical contention | Map/map-exclusive now borrow the existing root/claim/pin owner through fixed-slot admission in qualified IRQ-enabled scheduled contexts; real VT-d/SMMUv3 tests and both linked pairs pass. Masked ARM SVC still rejects busy waiting; unmap/close readiness, AMD-Vi runtime, wider pressure and exact attribution of the original NIC contender remain open. Diagnostics permit no replay. See [follow-up evidence](../reports/audits/2026-10-14-dma-map-readiness.md). |
| External NS publication retirement | NS-owned persistent endpoint-death producer | Implemented: each publication owns its connection and exact close watch; event-only default-CQ readiness confirms the owned result before logical retirement. Replacement/stale hints cannot clear a new generation; caller principal checks remain. NS admits 64 live publications plus one replacement slot before thread publication. Wider raw adapters and physical cleanup remain separate. See [contract](name-service-generations.md) and [evidence](../reports/audits/2026-10-14-ns-endpoint-death.md). |
| S08b2c4 | Remaining formats / raw adapters | S08b2a removes Kafka authentication sections; S08b2b replaces the lossy DNS catalog dump and HTTP reader; S08b2c1/c2 replace query/invocation identities and framing. S08b2c3 replaces registration/retraction tags and removes the empty-resource service-registration fallback. S08b2c4a replaces client/local invocation with owned full-name CQ-bound calls; S08b2c4b replaces generation observation/retirement and raw cleanup handles. Replace remaining local packed-name and scalar/named entry points, operational release-tail representations, physical-address queries and mailbox word/capability APIs coherently with rejection before admission. Current releases with/without connector bindings and serialized boot fixtures need explicit replacements, not deletion by keyword. |

A version number of 1 does not identify an obsolete protocol. Hardware interrupt
fallbacks and negotiated Kafka/S3 interoperability are separately supported
features. Kernel serialized fixture adapters can represent genuinely different
IRQ/root ownership contexts; remove them only when a coherent owning replacement
covers those callers. No simplification can remove capability escrow, cleanup
receipts, admission sponsorship or retained uncertain backing.

Architecture-aware rollout now uses signed executable variants and exact per-replica pins. S01 and S04a formats advanced to CDEPLOY6 and catalog-v17 for that coherent replacement. Descriptor-less publication/restore rejects; raw fixtures now reject retired opcodes; their active rollout uses the current signed contract. See [execution targets](execution-targets.md).

S08b1 subsequently advances registration persistence to catalog-v18 and pair-v3;
current contracts above reflect those tags, while historical reports retain
their original observations.

The [park-boundary follow-up](../reports/audits/2026-10-14-park-deferred-delivery.md)
corrects deferred producer delivery before guard-free wait publication, following
the ARM/x86 IPC timeout recurrence. It changes no protocol grammar or runtime
ABI and adds no replay/physical-release authority. Its deterministic negative
control is separate from historical exact-exit and Raft persistence attribution,
which remain open. The NS endpoint-death follow-up below supplies the third-party
publication producer; raw receive/watch/transport ownership and the other
recorded simplification candidates remain.

The [NS endpoint-death follow-up](../reports/audits/2026-10-14-ns-endpoint-death.md)
implements the third-party local binding producer recorded in S08b2c4b. It
replaces NS's scalar owning registry connections and temporary observation
watches with one owning publication and shared-CQ intake. NS interface 2,
CNSGEN01, runtime ABI 3 and persistence grammars remain unchanged; no caller
retirement privilege is expanded. Raw request/reply/watch/transport adapters
and historical failure attribution remain separate work.

The NS status batch S08b2c4d replaces the lossy NSST page with one complete
bounded CNSSTAT1 observation. Active rows preserve all 256 identity bytes and
positive generations; exact row/request counts replace registry/tombstone and
distinct-wait-name counts. The owning server transaction and whole-set HTTP
reader reject partial, retired or malformed observations. See the
[status contract](name-service-status.md) and
[execution evidence](../reports/audits/2026-10-15-ns-status-snapshot.md).
Remaining packed/public entry points and wider raw-adapter, metadata, IRQ/Drop
and physical-cleanup contexts remain separate; historical IPC/persistence
attribution stays open.

The [first-dispatch follow-up](../reports/audits/2026-10-15-first-dispatch-delivery.md)
corrects a second concrete delivery gap: fresh trampolines bypassed the older
yield continuation containing post-switch staging and deferred delivery. Both
architectures now complete that handoff on the incoming stack before restoring
its register frame. This changes no protocol or runtime ABI; historical IPC
exit/persistence attribution and the remaining simplification candidates stay open.

The [waiter source-preparation follow-up](../reports/audits/2026-10-15-waiter-source-preparation.md)
removes the fixture-only numeric `CqObservable`; its two registrations and the
IRQ fixture use the existing exact `CqInterest`/source capture. Ordinary CQ
registrations share production's prepared publication; the IRQ fixture retains
its deliberate cursor-independent registration after exact source capture.
No wire grammar,
storage format, runtime ABI or numbered simplification batch changes; the
remaining protocol/raw-adapter candidates above are unchanged.

The [exact timer-cancellation follow-up](../reports/audits/2026-10-16-timer-cancellation-identity.md)
removes the private numeric cancellation selector, state field and global
wrapping counter. The queue borrows its handle's existing charged state owner
and compares exact allocation identity. No scalar compatibility branch remains;
public completion operation IDs, runtime ABI, wire/storage grammars and numbered
protocol batches are unchanged. This closes one internal identity gap without
qualifying broader timer queue/Drop/IRQ contexts or historical failures.

The [initial completion admission follow-up](../reports/audits/2026-10-16-completion-initial-admission.md)
replaces owning record-factory arguments with a borrowed complete transaction.
The [shared observer-entry follow-up](../reports/audits/2026-10-16-observer-entry-construction.md)
replaces initialized owning node arguments with empty-storage allocation; the
original weak observer and charge stay in the preparation owner. Each private
factory has one current contract and no compatibility branch. Wire/storage
formats, runtime ABI, numbered protocol batches and remaining raw-adapter
candidates are unchanged; completed/containing destructor contexts remain open.

The [callback-owner follow-up](../reports/audits/2026-10-16-completion-callback-ownership.md)
uses a borrowed complete transaction for private completion-callback entry
factories too. This private test seam has one contract; ordinary preparation
outcomes use explicit finish. Public observation types, runtime
ABI, wire/storage grammars and numbered protocol batches are unchanged.

The [exit-watch owner follow-up](../reports/audits/2026-10-16-thread-exit-watch-ownership.md)
replaces the private scheduler preparation hook's owning list/observer/charge
arguments with one borrowed complete owner. It has one current contract;
ordinary outcomes explicitly finish after local guards. Public observation
errors, runtime ABI, wire/storage grammars and numbered batches are unchanged.

The [waiter-owner follow-up](../reports/audits/2026-10-16-waiter-preparation-ownership.md)
keeps one private borrowed entry-factory contract and replaces implicit containing
preparation cleanup with consuming finish in direct, IPC and CQ adapters. Public
source/registration interfaces and errors, runtime ABI, wire/storage grammars
and numbered batches are unchanged.

The [completed-entry follow-up](../reports/audits/2026-10-16-completed-observer-entry-ownership.md)
removes implicit cleanup from the private completed preparation contract. Direct
registration and all containing owners consume explicit finish. Public registration interfaces and errors, runtime
ABI, wire/storage grammars and numbered batches are unchanged.

The [lazy-source owner follow-up](../reports/audits/2026-10-16-lazy-waiter-source-ownership.md)
replaces the private owning candidate-preparation helper with a borrowed complete
transaction. Production and fixtures share that one contract and consuming finish.
Public waiter interfaces/errors, runtime ABI, wire/storage grammars and numbered
batches are unchanged.

The [19 October read-fence replacement](../reports/audits/2026-10-19-raft-read-fences.md)
removes the synchronous receive-time `can_serve_linearizable_read` query path.
Queries now require `begin_read`, borrowed `read_ready`, and consuming
`handle_client_query(PendingRead)`. DNS removes its blocking raw lookup helper,
adopts lookup/call transactions before scalar fallback guards, and retains typed
reply authority across local/relayed routing. OP_LOOKUP returns placement
metadata, eliminating attempted re-delegation of non-mintable NS lookup grants;
local results carry no capability. Shared protobuf, no-op grammar,
storage versions and runtime ABI remain unchanged. Bounded observations and
raw transport adapters remain candidates for separate qualification; no numbered
protocol-format batch is added.

The [snapshot continuation correction](../reports/audits/2026-10-19-raft-snapshot-streams.md)
makes the core's `PendingSnapshot` and its node field private. The receiver owns
leader ID, RPC term, boundary, offset and bytes together; callers use the existing
snapshot handler. No external callers used the mutable buffer. Wire/protobuf,
snapshot envelope and persistent formats remain unchanged, without an older
unbound receive path or a new numbered format batch.

The [SEC-04 boot-receipt boundary](../reports/audits/2026-10-19-security-boot-receipts.md)
requires `VerifiedBootPolicy` at `PreparedBootTrust::verified`. Shared
`verify_for_boot` supplies the exact-installed check; the fixture's separate
comparison is removed. Enrollment/rotation keep `verify` and `VerifiedPolicy`.
There is no conversion or fallback between these receipts. The successor-rejection
diagnostic is now `UninstalledRevision`, with fixture/parser tests updated.
Public record grammar, CLI modes, runtime ABI and production refusal are unchanged.

The [early artifact-authority follow-up](../reports/audits/2026-10-19-security-platform-artifact-authority.md)
removes the fixed development-key path from ordinary/platform ELF loading,
store verification and persistent-image checks. They require the immutable key
selected in the existing boot slot before platform startup. Composition takes
its preparation once; publication requires the selected public policy/digest.
Explicit authorized scoped/deployment-key APIs remain. No old-key fallback,
wire-format change or new numbered format batch is added.

The [SEC-15 hash-state follow-up](../reports/audits/2026-10-19-security-hash-state-erasure.md)
removes S3/SCRAM's duplicate HMAC helpers in favor of shared
`charlotte_launch::sha256::hmac_sha256`, returning `Zeroizing<[u8; 32]>`.
SCRAM PBKDF2 also returns its existing result owner. Plain-array helper returns
and compatibility wrappers are removed; SHA's public digest output stays an
array. Algorithms, wire formats and persistent records are unchanged.

The [SCRAM phase follow-up](../reports/audits/2026-10-19-security-scram-phases.md)
removes the mutable, reusable exchange API. Consuming initial challenge handling
returns one `ClientFinal` containing proof/verifier owners; consuming final
handling retires it on every outcome. The connector encodes borrowed proof
bytes from that phase. No mutable compatibility adapter remains. Kafka framing,
SCRAM bytes and persistent records are unchanged.

The [HPKE erasure follow-up](../reports/audits/2026-10-19-security-hpke-erasure.md)
changes `operations::generate_recipient_keypair` to return private bytes as
`Zeroizing<[u8; 32]>`, beginning before serialization. Its caller keeps that
owner directly; the plain-array return and late wrapping are removed. Public
bytes remain arrays. The HPKE suite, envelope grammar and stored key format
are unchanged; no compatibility adapter or format-number change is needed.

The [Base64 scratch follow-up](../reports/audits/2026-10-19-security-base64-scratch.md)
returns SCRAM encoded/decoded output as zeroizing owners, beginning inside each
helper. Proof, verifier and nonce callers keep those owners directly; their
late wrapping is removed. The decoder reserves the whole output bound before
copying. There is no plain-return compatibility wrapper. Padding behavior,
accepted grammar, Kafka framing and persistent records are unchanged.

The [SASL frame follow-up](../reports/audits/2026-10-19-security-sasl-frames.md)
returns authentication requests in `Zeroizing<Vec<u8>>` and makes reply parsing
consume the same input-owner type. The reply retains its entire frame and
exposes `auth_bytes()` as a borrow; the copied public vector and its derived
Clone/Debug implementations are removed. Header/final-length methods remain
shared with the ordinary encoder. No borrowed-input parser or plain request
compatibility wrapper remains. Kafka wire fields and version numbers are
unchanged, including nullable authentication bytes and broker-error metadata.

The [receive-owner follow-up](../reports/audits/2026-10-19-security-receive-owners.md)
returns TLS/plain local chunks and Kafka routed replies as zeroizing owners.
The broker's growable accumulator/split-copy path is replaced by `FrameReceiver`,
which owns its partial frame and existing chunk together. Header validation
precedes full-frame reservation; cached bytes are consumed before more input.
Empty, excessive or failed input fences the episode until reset. New-stream
startup resets before authentication. S3 keeps the shared owner type through
its buffers; no plain-return compatibility adapter remains. Wire grammars and
version numbers are unchanged. The cross-layer host test package belongs to
the root workspace and uses service libraries without freestanding entry points.

The [S3 response-buffer follow-up](../reports/audits/2026-10-19-security-s3-response-buffers.md)
replaces the public append-to-caller decoder API with one owned output per
call. `ChunkedDecoder::decode` reserves from input length, returns an erasing
owner and fences rejection. Its callers and fixtures use that single API;
no append compatibility entry point remains. `ResponseHeadBuffer` supplies
bounded head accumulation without growth. HTTP wire grammar and S3 operation
IDs are unchanged; this changes local parsing ownership and terminal fencing.

The [S3 request-owner follow-up](../reports/audits/2026-10-19-security-s3-request-owners.md)
changes authorization/signature fields to erasing owners and removes the
aggregate Debug implementation. The service's private growable formatter is
replaced by the library's borrowed `RequestHead` and owned `request_head`
serializer. No plain-output compatibility formatter remains. Valid S3 request
bytes/signatures are unchanged; malformed fields reject before header output
allocation. Launch profiles retain their earlier validation boundary.

The [canonical-signing follow-up](../reports/audits/2026-10-20-security-s3-canonical-owners.md)
returns owned canonical URI/query text and owned `Signature` metadata fields.
`canonical_query` now returns a checked `Result`; no bare-string compatibility
helper remains. Temporary encoded/normalized entries and joined text use the
same owner policy. Callers and fixtures compile against the current API.
SigV4 canonical bytes and valid HTTP requests are unchanged.

The [response-metadata follow-up](../reports/audits/2026-10-20-security-s3-metadata-owners.md)
replaces borrowed-input `ResponseHead::parse` and its returned offset with a
consuming parser and borrowed metadata/body accessors. Independent String
fields and aggregate Clone/Debug/comparison derives are removed. The existing
accumulator transfers its header owner; the service borrows views for metadata
encoding. No compatibility parser remains. HTTP grammar, trimming, repeated
metadata behavior, parser error statuses and public S3 wire records are unchanged.

The [request-key/raw-path follow-up](../reports/audits/2026-10-20-security-s3-path-owners.md)
moves the private byte-only request into `charlotte_s3::request::OwnedRequest`,
using the shared protocol decoder and an erasing key owner. `key()` borrows and
`object_path` returns a reserved erasing owner. The unused Clone derive and
private plain key/path implementation are removed. The service supplies its
validated profile fields and preserves the existing IPC mapping boundary.
Wire formats, request limits/flags and path-policy bytes remain unchanged.

The [profile-owner follow-up](../reports/audits/2026-10-20-security-s3-profile-owners.md) replaces the service's private field-copy
profile with `charlotte_s3::profile::OwnedProfile`. One immutable erasing frame
lends typed views; the view's `authority` formatter returns a reserved erasing
string. The old plain field copies/authority formatter are removed without a
compatibility path. Current CHS3PF1 validation, launch-memory mapping scope,
rights, TLS configuration and HTTP authority bytes are preserved.

The [profile-encoding follow-up](../reports/audits/2026-10-20-security-s3-profile-encoding.md) changes wire `Profile::encode` to return
an erasing byte owner and removes aggregate profile Debug. The kernel caller
uses that owner directly; fixtures compare profiles without formatting their
credentials. No plain-output compatibility encoder remains. CHS3PF1 grammar,
validation and launch-memory ownership are preserved. The protocol crate adds
an edge to the existing pinned zeroize dependency; no third-party version changes.

The [Kafka profile-encoding follow-up](../reports/audits/2026-10-20-security-kafka-profile-encoding.md) changes `Profile::encode` to return
an erasing byte owner and removes aggregate Debug from Profile/Authentication.
The kernel caller takes that owner directly; fixtures compare profiles without
automatic credential formatting. No plain-output compatibility encoder remains.
CHKAFP7 bytes, digest, validation and launch-memory ownership are preserved.
The protocol crate adds an edge to the existing pinned zeroize dependency,
without a third-party version change.

The [SCRAM initial-owner follow-up](../reports/audits/2026-10-20-security-scram-initial-owners.md) removes the private escaped-username
formatter and writes directly into the existing reserved transcript. The initial
phase's nonce changes to an erasing owner; its public constructor and consuming
phase API stay the same. No scratch/compatibility formatter remains. Existing
nonce policy, UTF-8 username/escape bytes and proof/verifier behavior are preserved.

The [host key-file follow-up](../reports/audits/2026-10-20-security-host-key-encoding.md) replaces the key writer's generic hex plus
newline append with `encode_key_file`, which returns one reserved erasing owner.
No plain-output key-file fallback remains; generic public digest/identity hex
formatting keeps its existing use. File syntax, commands, private-file policy
and exclusive creation are unchanged. The helper is private to the host tool.

The [host profile-input follow-up](../reports/audits/2026-10-20-security-host-profile-input.md) replaces the private unbounded
`fs::read` expression in `operations-seal` with one bounded erasing input owner.
There is no alternate plaintext reader. Unix no-follow regular-file admission
and early size rejection apply to the same command; arguments, COPSENC1 bytes,
cryptography and runtime APIs are unchanged. The helper stays private to the tool.

The [public-key input follow-up](../reports/audits/2026-10-21-security-public-key-input.md) removes the unbounded public-key
`read_to_string` path and the trust module's separate normalization/parser.
Public key and policy files share the existing bounded regular-file reader;
public key files share one hex decoder. The 4096-byte limit now covers ordinary
operational and ingress-policy file arguments too. Public hex arguments,
private-key policy, command syntax and signed formats are unchanged.

The [operational-input follow-up](../reports/audits/2026-10-21-security-operational-input.md) removes whole-file `fs::read` from
all operational envelope/bundle command inputs. They share bounded regular-file
admission with public policy/key files, using existing codec maxima. Bundle
signing checks its binding count before referenced reads and iterates those
triples directly, removing the temporary path vector. Command syntax, signed
formats, cryptography and receiver authority are unchanged. The control-input
follow-up below also covers structured release/deployment records; artifact
reads are covered by the later artifact-input follow-up.

The [control-input follow-up](../reports/audits/2026-10-21-security-control-input.md) removes whole-file reads from the
remaining structured deployment, release, shutdown and ingress-policy input
paths. They use the same bounded regular-file reader with current codec limits.
Release signing checks its existing descriptor-count maximum before referenced
reads. Apply preparation order, command syntax, public hex arguments, signed
formats and receiver authority are unchanged. The artifact-input follow-up below covers raw ELF and arbitrary-file hash
inputs with their own admission policies.

The [artifact-input follow-up](../reports/audits/2026-10-21-security-artifact-input.md) replaces the remaining four whole-file
ELF reads with bounded admission using the existing 4 MiB launch maximum, and
replaces whole-file `sha256` loading with a streaming helper. Descriptor opening
is shared with current bounded readers. Note construction preflights the complete
signed output size, and variant-count admission precedes image reads. No alternate
unbounded artifact/hash path remains. CLI syntax, signature bytes, wire formats
and runtime ABI are preserved; no general file-size limit is added to `sha256`.

The [artifact-edit follow-up](../reports/audits/2026-10-21-security-artifact-edit.md) replaces ELF signing's path-based
read/write pair with one `ArtifactEdit` owner. Descriptor admission and bounded
reading are shared through borrowed helpers, retaining read-only access for
other consumers. ELF commit consumes the captured read/write file and image;
there is no path-based publication fallback. Command arguments, signing bytes,
permissions policy and runtime formats stay the same. Human output now says
"artifact opened at" the supplied path to describe the selected object.

The [control-output follow-up](../reports/audits/2026-10-21-security-control-output.md) removes the three remaining path-based
`fs::write` publication calls in the signer. Deployment, release and shutdown
signing share `write_new_file` with other new-file commands; existing paths reject
without an overwrite option or alternate publication path. Boot fixtures allocate
fresh private directories for signed records. Signed formats, command arguments
and successful bytes stay the same. The output-retention follow-up below removes
failed-write pathname cleanup.

The [output-retention follow-up](../reports/audits/2026-10-21-security-output-retention.md) removes the shared failed-write unlink
and three key-generator rollback deletions. One private `write_new_keypair`
composes the existing erasing key encoders and exclusive writer; its errors
identify incomplete generation and previously written private output. Created
files remain available for inspection. No overwrite/cleanup compatibility branch
remains. Formats, successful bytes, arguments and runtime ABI are unchanged.

The [fixture-publication follow-up](../reports/audits/2026-10-21-security-fixture-publication.md) replaces the local helper's final
`Path.write_bytes` with exclusive staging and one named replacement. The existing
six-record grammar, descriptor preparation, CLI arguments and cache admission
stay the same; there is no in-place publication fallback. Staging is retained
on failure. This changes host publication policy without changing a wire format
or the signer's exclusive-output contract.

The [deployment-generation follow-up](../reports/audits/2026-10-21-security-deployment-generation.md) replaces descriptor-apply's ready
substring test with the current acknowledgement and six-field readiness codec.
It requires complete unambiguous scalar records and retains each admitted
generation through observation; there is no missing-generation or substring
fallback. Server encoding, HTTP routes, CLI arguments and runtime ABI are
unchanged. `release-apply` and standalone named status still need separate
qualification; the release acknowledgement has no component-generation map.
