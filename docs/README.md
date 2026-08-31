# CharlotteOS documentation

The Markdown documentation is organized by **purpose and freshness**, rather
than by subsystem alone. Start with the LaTeX
[manual](manual-v2/charlotte.pdf) for the integrated system description, then
use the sections below for deeper design, implementation, or historical
context.

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

The distinction matters: a successful historical boot or an audit statement
is evidence about one revision and configuration, not a standing guarantee.

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

## Maintenance rules

- Put durable design intent in `architecture/`, not in an audit report.
- Put implementation invariants beside the relevant code-facing material in
  `reference/` or `tla/`.
- Put commands that contributors should continue to run in `guides/`.
- Name point-in-time reports with an ISO date prefix when a date is known.
- Do not rewrite an old report to look current. Add a clearly dated follow-up
  or update the corresponding living document.
- Prefer relative Markdown links and validate them after moving or renaming
  documentation.
