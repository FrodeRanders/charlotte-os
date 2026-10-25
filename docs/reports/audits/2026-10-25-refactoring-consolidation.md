# Audit follow-up: refactoring consolidation

Date: 2026-10-25. Reviewed baseline: `100965a0`.

This batch reduces repeated admission mechanics, separates a complete memory
transfer transaction from the object registry, and restores a concise C16/C18
coverage map. It changes no configured capacity, capability ABI, wire format,
root sponsorship, recovery authority or finding status. SEC-07/18 remain partial;
the ledger remains **29 corrected / 6 partial / 3 open**.

## Admission mechanics

The pure lifecycle crate's `AdmissionPool<N>` combines total and ordinary
`VectorBudget` counters. Reservations and refunds update both counters together;
limit, overflow and underflow rejection leave both unchanged. Platform work
continues to obey the total ceiling. The counter owns no resource, does no
allocation, takes no lock and has no cleanup destructor.

Six existing families use it: root metadata, principal history, thread metadata,
scratch nodes, snapshot buffers and mapping nodes. Each keeps its own static
lock, limits, typed `Charge`, captured classification and amount, and existing
allocation-lifetime release path. Thread metadata still reserves principal
admission first and drops that original principal charge only after its node
lock leaves. No generic resource owner or additional account/registry is added.

Three new counter tests cover the reserved platform share, original-classification
refunds, independent dimensions, ordinary rejection after total admission would
fit, checked arithmetic and atomic underflow rejection. Policy-profile pressure
tests now exercise this actual shared counter instead of manually composing two
independent budgets. Existing native family controls retain their pressure,
retention, cancellation and generation-reuse coverage.

## Transfer boundary and documentation

[`object/transfer.rs`](../../../crates/catten/src/memory/object/transfer.rs)
contains move/copy/loan preparation, the complete `PreparedTransfer` owner,
atomic authority/payload publication, delivery visibility and ordinary cleanup.
The original module reexports the same entry points; it retains registry,
backing, mapping, close and revocation policy. Its size falls from 2686 to 2032
lines. No cleanup phase is detached from its source escrow, pins, destination
metadata or private backing. Existing containing transfer/backing destructors
remain active; this extraction does not qualify unknown outer IRQ/guard contexts.

Source comparison confirms the extracted code matches the baseline after
normalizing relative module paths, fixture-only parent visibility, whitespace
and optional trailing commas. Two fields retain their original object-family
fixture visibility; no caller outside that family gains direct field access.

The [cleanup coverage map](../../reference/cleanup-recovery.md#receipt-coverage-map)
now summarizes C16/C18 owners, continuation rules and remaining gaps. All wording
and evidence links from those two original detail cells are preserved in the
[qualification notes](../../reference/cleanup-recovery-details.md), with only
line wrapping and paragraph breaks changed. A normalized-whitespace comparison
confirms preservation. Historical audit reports are unchanged.

This is one bounded consolidation, not completion of the wider memory/IPC/
completion decomposition. Repeated bookkeeping in other contexts must be
qualified before adopting this helper; similar spelling alone does not establish
the same ownership or refund contract.

## Validation

Execution results and source/artifact hashes are recorded in the accompanying
[evidence](evidence/2026-10-25-refactoring-consolidation.json).

- Actual lifecycle crate compiled directly with `rustc --edition 2024 --test
  crates/charlotte-lifecycle/src/lib.rs -D warnings`: **34 tests pass**.
- Resource-policy harness: **3 Python tests pass**; default/larger profiles each
  execute **16 Rust tests**, and all 11 invalid profiles reject compilation.
- Strict base-kernel Clippy passes for AArch64 and x86_64, using their signed
  service-bundle inputs and `--no-default-features --features acpi -- -D warnings`.
- Rebuilt and signed AArch64 service/user bundles and the x86_64 service bundle.
- Four-LP native security/HTTP runs pass:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs execute all six admission-family pressure controls, borrower storage,
capability-preparation rejection/retention and mixed IPC authority publication.
No native failure occurs in these executions. Historical first-entry,
metrics/root-close/mapping failures remain unattributed; these passes do not
explain them. AMD-Vi is not rerun for this shared-counter/module extraction;
no IOMMU backend implementation changes. Formal model inputs are unchanged.

Workspace formatting, whitespace and local references pass. Documentation
preservation and extraction comparisons are recorded above; runtime execution
does not qualify arbitrary destructor contexts or sustained hostile pressure.

Initial host Cargo attempts failed before test execution: the workspace's
kernel `build-std` configuration produced duplicate `core::sized` language items;
adding `std,panic_unwind` also produced conflicting allocation handlers/panic
strategies. The direct-rustc run above executes the actual dependency-free crate,
not an alternate implementation. Initial AArch64 Clippy lacked the required
`CATTEN_AARCH64_SERVICE_BUNDLE` input and rejected eight `env!` expansions;
the configured invocation passes. These are validation setup failures, separate
from native execution and its historical intermittent failures.
