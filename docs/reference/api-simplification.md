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
| S03: Raft persistence | Two-object pair: `CRFTSTA2` state and `CRFTLOG2` snapshot/log | Original S03 baseline: 402 host tests and four corrected fresh/reuse guest runs passed; the initial failed pair and timer-fixture correction are retained separately. Removes term/vote-only decode, split-snapshot migration, raw-entry fallback, independent store constructors and optional disk-to-memory fallback. Joint opening rejects incomplete/unsupported records; the transport uses owned runtime resources. S04b2b advances the pair tags to reject old command logs. See [contract](raft-persistence.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-raft.md). |
| S04a: catalog snapshots/query | `CATALOGH` version 17; complete deployment query record | Original S04a baseline: 410 host tests and four matching-architecture deployment guests passed. Current-only snapshots and mandatory query sections replace compatibility decoding; restore rejection precedes Raft startup/log/membership publication. The initial fixture and mixed-architecture failures are retained separately. The later target-selection change retires descriptor-less publication/restore; later S04b2a/b remove packed callers and redundant fields. See [contract](catalog-snapshots.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-catalog.md). |
| S04b1: scoped launch/retirement | Four-argument syscall 76 returns ASID and verified principal; one owning runtime API | Complete: 411 host tests and final ARM 20/0/0 / x86 18/0/0 signed-ingress guests pass. Removes packed scoped names, ASID-only/named wrappers, caller-chosen retirement names, the unused emitter and singleton retirement. The initial x86 timer-fixture failure and its isolated-worker correction are retained separately. See [contract](scoped-deployment-launch.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-launch.md). |
| Architecture prerequisite | CDEPLOY6 signed variants; exact per-replica pins; kernel-reported execution profiles | Complete: 418 host tests, ARM/x86 central-store rollout and linked mixed guests (22/0/0 and 20/0/0). Unknown/incompatible profiles reject; native launch rechecks target and image. Fixture corrections and the authorized remote-invocation gap are retained in [evidence](../reports/audits/2026-10-13-execution-targets.md). |
| S04b2a: deployment entry points | Signed launch only; full-name controller status; retired opcodes reject | Complete: 419 host tests, strict service/kernel Clippy on both architectures, ARM 22/0/0 and x86 20/0/0 linked guests. Descriptor-less syscall 64 and runtime/SDK wrappers, controller deploy opcode 2, DNS packed deploy/query opcodes 8/9 and optional DEP1 suffix are removed. Controller interface version 3 uses moved full names for status and rollout. See [contract](deployment-administration.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-administration.md). |
| S04b2b: lean deployment records | Descriptor-only admission; CDEPQR01 queries; catalog-v17; Raft pair-v2; CLOBSV2 observations | Complete: 425 host tests, strict service/kernel Clippy and passing mixed/S3 guests on both architectures. Initial DMA startup failure remains separately recorded and unresolved. Removes redundant name/object/node/digest fields from admission and object ID/primary/variant-zero digest from stored and queried deployments. Exact signed replica pins remain; retired formats reject, including log-only old stores. See [contract](catalog-snapshots.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-records.md). |
| S05: canonical IPC receive | Twelve-register envelope on 71/72/73; runtime ABI 2 | Complete: 427 host tests, strict Clippy, passing ARM/x86 linked and S3/HTTP guests, and ARM security qualification. Removes nine-register receive syscalls 22/27/53 and duplicate authenticated SDK/runtime names. Services and both raw fixture sets use the canonical envelope; signed variant matching rejects ABI 1. Host and native verification is recorded in the [contract](ipc-receive.md) and [batch evidence](../reports/audits/2026-10-13-protocol-simplification-ipc-receive.md). |
| S06: checked CQ waits | Syscall 88 with explicit timed flag; typed outcomes; runtime ABI 3 | Complete: 430 host tests, two Sitas host tests, strict Clippy, final ARM 22/0/0 / x86 20/0/0 linked guests and ARM 20/0/0 / x86 19/0/0 S3/HTTP guests. Removes unchecked 12/42 SDK/kernel adapters and ignored count thresholds. Ordinary reactors/backoff fail closed; periodic watchdogs and the sibling Sitas backend retain their deadlines. An intermittent ARM stall remains unresolved; failed runs and the pre-clock security qualification are preserved in the [contract](cq-wait.md) and [evidence](../reports/audits/2026-10-13-protocol-simplification-cq-wait.md). |

`DEFAULT_SHUTDOWN_GRACE_MS` remains an explicit built-in launch policy choice;
it is not a parser fallback. Only current v6 signed bytes are accepted. Retired
`.cdep` files and their signed release/operations envelopes must be regenerated
from the current reviewed component plan. Persisted catalogs containing retired
descriptors cannot turn them into launch authority; storage-format removal is
separate work below.

## Remaining candidates

This is a source-backed starting inventory, not a completed whole-codebase audit.

| Order | Area | Candidate and coordinated work |
| --- | --- | --- |
| S07 | Tooling/export aliases | Deprecated socket capacity constant, singleton runner flags/environment fallbacks, public development-key argv exception and compatibility helpers. Update scripts, tests and examples with each removal. |
| S08 | Remaining formats / raw adapters | Audit object headers and owner-zero policy, optional DNS suffixes, launch/profile and operations-bundle variants, signature-note helpers, physical-address queries and mailbox word/capability APIs. Current image-tool outputs and serialized boot fixtures need explicit replacements, not deletion by keyword. |

A version number of 1 does not identify an obsolete protocol. Hardware interrupt
fallbacks and negotiated Kafka/S3 interoperability are separately supported
features. Kernel serialized fixture adapters can represent genuinely different
IRQ/root ownership contexts; remove them only when a coherent owning replacement
covers those callers. No simplification can remove capability escrow, cleanup
receipts, admission sponsorship or retained uncertain backing.

Architecture-aware rollout now uses signed executable variants and exact per-replica pins. S01 and S04a formats advanced to CDEPLOY6 and catalog-v17 for that coherent replacement. Descriptor-less publication/restore rejects; raw fixtures now reject retired opcodes; their active rollout uses the current signed contract. See [execution targets](execution-targets.md).
