# Q4 waiter, timer and completion account principal funding

Baseline `a5813b29`. Six fixed shared account types now fund their actual Arc
payload and private-holder layouts against the original signed principal:
waiter, scheduler timer, completion timer, record, CQ and watch. This covers three
more existing catalog families. Q4 remains in progress; no finding disposition,
aggregate gate, production trust or physical-platform acceptance changes.

[Version-controlled evidence](evidence/2026-11-06-q4-reactor-principals.json)
contains source/input and artifact identities, selected policy, inline native
milestones, commands, preflights and limitations. Historical reports remain
unchanged. The current [24-pool checklist](../../reference/metadata-admission.md#principal-coverage-and-remaining-reconciliation)
identifies wider principal, overhead and joint-heap work.

## Ownership and shared implementation

Root preparation captures principal and storage class from its unpublished,
prebound translation account. Completion namespace preparation captures the exact
current root generation, principal and class together under one root-table hold.
The token grants no root lease or publication/cleanup authority. Permanent kernel
and trusted raw fixtures retain their explicit unidentified exemption; a signed
application uses its prebound verified account. Later promotion, a closing worker
or a successor ASID cannot select the refund recipient.

`klib::account_backing` shares the fixed-bundle constructor among namespace,
waiter, timer and completion families. It composes the existing typed family
charge, `ChargedAllocator` and `SponsoredAllocator`; it introduces no new
reference-counting scheme, private holder or principal registry. Each layout is
reserved before allocation and refunded after actual deallocation. The family
slot spans both allocations through final weak/allocator release. Rejection
returns unused prefixes outside local publication guards. Resource grants and
retirement checks stay with their typed adapters.

The existing boot-lifetime principal-history record gains one 16-byte vector.
Zero-sized purpose markers select buffer, namespace-account or reactor-account
bytes through the same history lookup. There is no runtime purpose field in each
sponsor. At 4096 history nodes the additional field contributes at most 64 KiB
requested layout; the existing history pool still bounds records and candidates.
Its general principal/excess accounting remains open.

## Capacity and limit choice

`principal_reactor_account_bytes` defaults to **1 MiB** per verified principal,
shared by sibling roots, retained generations and all six account types. It is
independent of the 1 MiB namespace-account and 16 MiB buffer allowances. Retained
reactor accounts therefore cannot consume that buffer funding for inputs or
cleanup scratch. Existing global counts, logical limits, heap reserves, protocols,
SDK and signed service bundles are unchanged.

| Account | Actual complete bundle bytes |
| --- | ---: |
| Waiter | 72 |
| Scheduler timer | 72 |
| Completion timer | 72 |
| Completion record | 64 |
| CQ | 80 |
| Watch | 64 |

A root needs 144 mandatory reactor bytes. A current completion namespace adds
280, giving 424 bytes for all six. In isolation the 1 MiB allowance accommodates
2473 such sets, or more roots without completion accounts; retained weak backing
reduces that headroom. This deliberately limits a single principal below some
whole-node capacities. It is no launch entitlement. Raising the allowance permits
more retained memory; lowering it can reject mandatory preparation earlier.
No exercised workload needs an increase or a logical-limit reduction.

Global count ceilings imply separate ordinary/total requested-layout bounds:
432/576 KiB for waiter accounts, 864/1152 KiB for timer accounts and 1440/1920 KiB
for completion accounts, including retained holders. Their combined bound is
2.671875/3.5625 MiB. This lies outside the catalog's 19 byte-denominated pool sum;
it is neither disjoint observed usage nor proof of joint heap fit. The version-2
policy census now includes the selected reactor-account principal ceiling.
The 24-pool catalog and 205.5/154.125 MiB independent byte-ceiling sums remain.

Each reactor-account reference now occupies 32 rather than 16 bytes; the private
holder remains 24. Existing byte-admitted enclosing nodes reserve their current
layouts; other reference-bearing storage remains separate Q4 work. Root policy nodes
still total 392 bytes; their memory-account node grows from 144 to 176, yielding
568 bytes per complete root. The count-only root capacity needs
2326528/1744896 total/ordinary bytes, within 4/3 MiB. The completion namespace
node grows from 160 to 224 bytes; all five namespace nodes total 696 bytes.
Twice complete-root count headroom needs 5701632/4276224 total/ordinary bytes,
within 8/6 MiB. Current references update these costs. These comparisons justify
keeping those pool ceilings and do not qualify the rest of the heap.

Runtime pool samples still report bundle counts, not principal byte usage.
Allocator excess, external references and wider payloads need their enclosing
families' accounting. No fairness guarantee or mutable administration API is added.

## Controls and qualification

Native fixtures prebind private principals at the trusted constructor boundary;
the combined security campaign separately exercises verified signed launch.
Actual allocation deltas reconcile all six layouts and sibling sharing.
384 heap-held constructor denials return original family reservations without
allocator entry. Four root prefixes include no waiter, incomplete waiter, waiter
only and incomplete scheduler backing. Rejection restores physical frames, root
counts, family slots and principal bytes without publication. Eight completion
namespace prefixes similarly reject complete/incomplete account boundaries while
preserving the established record account and CQ. A healthy principal still
prepares a namespace, and promotion cannot change original funding.

A real default-policy workload holds **16381 record-account bundles** alongside
its 144-byte root, making the 1048576-byte ceiling bind with 48 bytes unused and
global ordinary account slots available. Disposal and root close recover the
original baselines. Private fixture limits are restored without discarding live
charges. This is serialized allocation pressure, not sustained concurrent EL0
progress or essential-service fairness.

Existing lifetime controls now use funded roots. A real waiter charge, an actual
prepared event and cancellation allocation, completion weak backing, a retired
heap CQ and an owned watch entry preserve original funding across root close and
same-slot generation reuse. All six account types have weak-retention coverage;
individual final weak releases refund the old principal while the successor
remains unchanged. The private heap CQ has no physical mapping or consumer.

The existing complete unpublished-root abandonment fixture retains **256 namespace
plus 144 reactor bytes** under history, lifecycle/table, ledger, heap, physical
and account-pool guards. It publishes no ASID and adds no new intentional leak.
This qualifies that complete parent, not arbitrary account-reference destruction.
Shared host controls retain actual free-before-refund tracing, holder/payload
rejection, alignment, weak-only/allocator-clone retention, single-use allocation
and concurrent final-weak release. Native heap exhaustion is not forced.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting, including standalone byte-allocator fixtures | Passed |
| Full host suite and policy profiles | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19; 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each; 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed; 90-second budgets |
| Manual | Two successful LaTeX passes |

All five final campaigns use unchanged signed service-bundle identities from
the preceding namespace-account qualification. The 49 frozen source/input hashes
remained unchanged through qualification; artifact identities and milestones are
inline in the linked evidence. Native AMD DWARF independently confirms a 24-byte
private holder, 32-byte funded waiter-account reference and 224-byte completion
namespace node in that campaign's exact kernel. These passes establish only the
named allocation/retention paths under the current profile, not an aggregate,
production-trust or physical-platform guarantee.

## Preserved preflights and limits

The initial ARM compiler preflight caught account-test code relying on an Arc
import removed from its parent module, plus an unused waiter-module import.
The next formatter invocation caught a new import placed before the test module's
inner documentation comment. These were corrected. Strict minimal profiles then
caught four admission-module imports now used only by tests; those imports were
scoped to the test configuration. Diagnostics are retained inline in the evidence.

Preliminary ARM 19/19 and Intel 17/17 native passes are recorded separately from
the final frozen-source campaigns after import scoping. No native failure or
deadline increase occurred. The full manual built twice; existing overfull-box
warnings remain in unchanged paragraphs. The requested manual wording edits are
included, with `nprepare` corrected to `prepare`.

The ARM display second-grant-probe, Intel retirement-pin and ARM exit-watch
incidents remain unresolved. Passing headless campaigns neither explain nor close
them. Wider caller/IRQ and implicit-destruction review, remaining principal and
allocator coverage, physical recovery and sustained service/cleanup progress
remain Q4–Q8 work. Existing aggregate gates remain open.
