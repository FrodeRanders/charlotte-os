# CharlotteOS documentation

These notes describe what we are building, what we have tested, and what we
still need to understand. They are organized by purpose and how current they
are. Start with the LaTeX
[manual](manual-v2/charlotte.pdf) for the integrated system description, then
use the sections below for deeper design, implementation, or historical
context.

For knowledge from the security work, start with
[security engineering lessons](guides/security-engineering.md). The
[current security ledger](reference/security-remediation.md) gives unfinished
milestones and incidents; the [audit archive](reports/audits/README.md) is for
historical investigations.

## Which documents are authoritative?

Use this order when two documents disagree:

1. Source code and executable tests define current behavior.
2. The manual's implementation-status appendix and the living documents in
   [`reference/`](reference/README.md) describe the implemented contracts.
3. Documents in [`architecture/`](architecture/README.md) describe intended
   boundaries and direction; each may include an explicitly labelled status
   section.
4. Documents in [`platforms/`](platforms/README.md) are living bring-up notes
   for a particular target and can lag another platform.
5. Documents in [`reports/`](reports/README.md) are point-in-time evidence.
   They deliberately preserve findings that may since have been fixed.

A past boot or audit tells us what happened with a particular revision and
configuration. We need current checks before relying on it for a later build.

## Documentation map

| Area | Use it for |
|---|---|
| [`manual-v2`](manual-v2/README.md) | Integrated architecture, programming model, implementation status, cluster vision, and PDF build instructions |
| [`architecture/`](architecture/README.md) | Living designs, security boundaries, protocols, and future direction |
| [`reference/`](reference/README.md) | Code-facing invariants, conformance rules, and subsystem mechanics |
| [`guides/`](guides/README.md) | Repeatable contributor workflows: testing, userspace development, and networking |
| [`platforms/`](platforms/README.md) | AArch64, `sbsa-ref`, and x86-64 platform status |
| [`reports/`](reports/README.md) | Dated audits, debugging investigations, and milestone records |
| [`research/`](research/README.md) | Prior systems, their afterlives, and CharlotteOS's inheritance |
| [`tla/`](tla/README.md) | Executable TLA+ models, model-checking instructions, and Rust conformance map |
| [`figures.md`](figures.md) | Editable Mermaid sources and explanatory captions for the architecture figure set |
| [`presentations/charlotte-for-operations.md`](presentations/charlotte-for-operations.md) | Ops-facing slide outline covering the cluster model, development/operations separation, connectors, and production gaps |
| [`presentations/openshift-and-charlotte.md`](presentations/openshift-and-charlotte.md) | Slide outline comparing OpenShift's Linux-container platform with CharlotteOS's cluster-native operating-system direction |

## Maintenance rules

- Put durable design intent in `architecture/`; audit reports capture a point
  in time.
- Put implementation invariants beside the relevant code-facing material in
  `reference/` or `tla/`.
- Put commands that contributors should continue to run in `guides/`.
- Name point-in-time reports with an ISO date prefix when a date is known.
- Do not rewrite an old report to look current. Add a clearly dated follow-up
  or update the corresponding living document.
- Prefer relative Markdown links and validate them after moving or renaming
  documentation.
- Consolidate audit learning into current contracts, failure patterns and
  limitations. Update those explanations in place; keep patch chronology,
  experiment receipts and significant failure captures in dated reports.
- Keep the security ledger to current dispositions, finite acceptance gates and
  unresolved incidents. Documentation compaction must preserve open failures and
  qualification limits without repeating the reasoning for completed patches.
