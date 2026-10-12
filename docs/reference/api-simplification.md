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
| S01: deployment descriptors | `CDEPLOY5`, version 5, exact 182-byte header | Complete and validated. Removes v1–v4 decoding, execution/shutdown/placement defaults, legacy constants, redundant decoded version field and short-header admission. Kernel/runtime, HTTP and operations pickup use the same current header bound. Signed release/trust documentation, Markdown guides and the LaTeX/PDF manual reflect the sole grammar. See [batch evidence](../reports/audits/2026-10-13-protocol-simplification-deployment.md). |

`DEFAULT_SHUTDOWN_GRACE_MS` remains an explicit built-in launch policy choice;
it is not a parser fallback. Current v5 signed bytes remain usable. Retired
`.cdep` files and their signed release/operations envelopes must be regenerated
from the current reviewed component plan. Persisted catalogs containing retired
descriptors cannot turn them into launch authority; storage-format removal is
separate work below.

## Remaining candidates

This is a source-backed starting inventory, not a completed whole-codebase audit.

| Order | Area | Candidate and coordinated work |
| --- | --- | --- |
| S02 | Discovery wire protocol | `charlotte-protocol-disco`: base-only responses and optional cluster suffix. Choose one complete payload, migrate builders/parsers/peers and replace old acceptance probes with rejection. |
| S03 | Raft persistence | `catten-services::disk_raft`: term/vote-only decode and split snapshot/log migration. Choose one canonical record and update object IDs, recovery fixtures and storage docs. Distinguish new empty stores from malformed/retired records; no implicit reset. |
| S04 | Deployment/catalog API | Packed-name versus named spawn/deploy/query, optional descriptor suffixes, singleton query prefixes and descriptor-less stored records. Migrate controllers, public tools, catalog replication and fixtures to one explicit signed/name-bound contract. |
| S05 | IPC receive ABI | Nine-register and authenticated envelopes; duplicate scalar/vector/blocked SDK/runtime entry points. Make authenticated metadata canonical and migrate services plus raw AArch64/x86 fixture emitters/decoders together. Preserve attachment ownership and exact pending delivery. |
| S06 | Wait ABI | Old CQ waits versus checked outcome/deadline waits. Migrate reactors and explicit timer policies; preserve event/deadline producer contracts and distinct admission rejection. |
| S07 | Tooling/export aliases | Deprecated socket capacity constant, singleton runner flags/environment fallbacks, public development-key argv exception and compatibility helpers. Update scripts, tests and examples with each removal. |
| S08 | Remaining formats / raw adapters | Audit object headers and owner-zero policy, optional DNS suffixes, launch/profile and operations-bundle variants, signature-note helpers, physical-address queries and mailbox word/capability APIs. Current image-tool outputs and serialized boot fixtures need explicit replacements, not deletion by keyword. |

A version number of 1 does not identify an obsolete protocol. Hardware interrupt
fallbacks and negotiated Kafka/S3 interoperability are separately supported
features. Kernel serialized fixture adapters can represent genuinely different
IRQ/root ownership contexts; remove them only when a coherent owning replacement
covers those callers. No simplification can remove capability escrow, cleanup
receipts, admission sponsorship or retained uncertain backing.
