# Exact-root retirement Pending diagnostics

Date: 22 October 2026. Reviewed baseline: `fc9b0f9a`.

The earlier x86 cancellation-stress reclamation timeout remains unresolved.
The [preceding report](2026-10-22-security-immediate-send.md) records its panic,
captured stack and passing repeat separately. The stack identifies the final
`flood.stop()` after both application scopes retired, but its generic
`AddressSpace(OperationDrainTimedOut)` does not identify the pending subsystem.
This follow-up records that distinction on the existing close owner. It changes
no close deadline, admission fence, completion proof or retry policy. The ledger
remains 29 corrected, six partial and three open; production trust stays disabled.

## Observations carried by the owner

`ClosingAddressSpace` retains one inline `Option<PendingClose>`. Every actual
Pending return sets its stage:

| Stage | Captured observation |
|---|---|
| Initial operations | Lease count before subsystem cleanup |
| IPC | Selected token/call, captured caller/server roots, checked claim epoch, completing/failure flags and loan count |
| Memory operations | Lease count before memory namespace cleanup |
| Memory | Selected object, object/authority phase, transfer/revocation flags and retirement/DMA/copy pin counts |
| Final operations | Lease count before sealing/extracting the root |

The lease count is sampled under the same table hold that validates the close
owner's table/generation and rejects its preflight. Memory observations are
captured under the existing object registry at the actual Pending branch.
IPC selection precedes cancellation preparation. A rejection can follow a
changed or vanished token episode or failed peer admission, so its selected
epoch/flags are an earlier observation, not proof of a current claim. Missing
captured peer identity remains `None`; it does not become the kernel principal.

The supervisor's first explicit terminal-error observation logs its captured
root ID/generation, error and last Pending sample while the close owner is still
live, after local lifecycle/table/subsystem guards leave. It then retains its
existing failed-close fence and resources. Subsequent polls return the cached
error without another log or cleanup attempt. Errors without a retained Pending
owner report `None`, which proves no cleanup or absence of outstanding work.

The diagnostic is Copy data inside the same owner. Capturing it allocates no
storage, acquires no operation lease, and adds no root, pin, registry, quota
family or recovery custody. Pending wrappers stay inline to avoid adding
allocation admission after closing begins. No destructor logs, waits, invokes
callbacks or releases fields because of these observations. Existing typed
receipts alone authorize physical cleanup and charge/slot release. C02 records
this scope; broader outer-caller and destructor qualifications remain separate.

## Verification and failed controls

The [version-controlled evidence](evidence/2026-10-22-root-retirement-diagnostics.json)
records source, kernel, image, bundle and capture hashes with selected diagnostics.

Existing real-root controls now assert two leases, then one, on successive
Pending polls. They still require old-operation completion before root release,
exact generation reuse, cached terminal failure and retained original charges.
The abandoned namespace cancellation control checks the exact token/call,
captured peer roots, active claim and epoch 1. It retains the abandoned claim and
both roots. The unmapped revocation control checks the selected authority/object,
Revoking state, one retirement pin, no transfer/DMA/copy pin, and completion only
after the existing loan owner finishes. These are assertions against actual
kernel owners and registries, not an independent simulator.

Initial boots failed on both architectures because the new revocation control
incorrectly expected zero retirement pins. Both observed `left: 1, right: 0` at
`memory/object/namespace_close/tests.rs:127`. `LoanRevocation::prepare` owns one
`MappingRetirementPin`, so the corrected assertion expects one. The diagnostic
matched the actual retained owner. These failed intermediate runs remain
separate from passing runs; they did not reach the live cancellation-stress
case and do not reproduce its historical reclamation timeout. Their exact
artifacts and selected failure diagnostics are recorded; the intermediate
source tree was not committed.

| Corrected run | Deferred tests (passed / failed / pending) | Final cancellation traffic |
|---|---|---|
| x86-64 first instrumented run | 17 / 0 / 0 | 16,136 requests retired |
| AArch64 final run | 19 / 0 / 0 | 17,848 requests retired |
| x86-64 final-source repeat | 17 / 0 / 0 | 16,596 requests retired |

Each passing boot also exercises the deliberate zero-budget timeout. For
example, x86 records
`root=15:6 error=AddressSpace(OperationDrainTimedOut) pending=Some(InitialOperations { leases: 1 })`.
That is the existing retained-owner fixture, not a new live stress failure.
The first instrumented x86 pass precedes a `let...else` to `?` cleanup and
Clippy attributes for inline owner storage; final-source runs cover both
architectures. Kernel and boot artifacts differ between fresh fixtures. The
signed service bundles match the baseline and remain unchanged.

Strict kernel Clippy passes on both targets. Formatting and diff checks pass;
the AArch64 signed-bundle reproduction/signature check passes. The manual's
LaTeX sources describe these observations and correct adjacent stale statements
about bulk IPC/device cleanup guards; its existing PDF remains unchanged.

The root-retirement finite model is unchanged. These fields observe existing
Pending transitions and drive no new transition or cleanup decision. Its
conformance map records the diagnostic boundary; no model result is claimed
for identifying a stuck native operation or proving physical completion.

## Remaining investigation

The historical live reclamation timeout did not recur in these instrumented
runs. None identifies its causal operation or rules out a lease, IPC claim,
memory blocker, scheduling delay or another cause. The next occurrence must
retain the full captured root and report its stage before further diagnosis.
An observation is not permission to retry an abandoned close, clear a claim,
widen a deadline or free uncertain backing. SEC-06/07/18 remain partial.
