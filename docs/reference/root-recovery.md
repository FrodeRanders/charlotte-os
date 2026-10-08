# Bounded final-root recovery

`memory::retirement::recovery` retains **complete detached user roots** rejected
by final invalidation, after device/IPC/memory namespace cleanup and cleanup-lease
sealing have completed. It owns each `RetiredAddressSpace`, including its exact
software slot, hardware tag, private hierarchy and original backing accounts.
No admission can reconstruct an owner from an ASID, frame address or diagnostic.

## Admission and explicit retry

The registry has eight inline slots and allocates no heap storage. Final-root
release transfers its owner only after invalidation fails before physical release.
A full registry or exhausted serial space returns the original owner outside its
masking guard; the caller quarantines it. A saturating rejected-admission counter
records this capacity loss. There is no spill list or allocation-based fallback.
Only recovered history slots may be reused, with a fresh checked serial. Pending,
exhausted, abandoned and quarantined slots never recycle.

A ticket contains a slot index and serial; it conveys diagnostic identity, not
userspace authority. The trusted kernel `retry(ticket)` boundary rejects masked
interrupt state before claiming a record. Claiming moves the unique root owner
out and marks its record running. Invalid/stale, competing, exhausted or terminal
claims reject before callbacks or physical work. The registry guard leaves before
invalidation, allocator access, root/account destruction and slot completion.
There are at most two explicit attempts per receipt. Each production x86 attempt
retains the existing three fresh epoch-fenced rendezvous attempts. Rejection
returns the same complete owner and retains its slot and charges.

After confirmed invalidation, `RetiredEntry::release_value_with` explicitly runs
the owning physical walk, then destroys the disarmed payload and returns its
linear slot token. Both architecture walkers report rejected frames; they disarm
before physical release and retain whole original charges on failure. Confirmed
invalidation permits software slot/hardware tag completion even on a physical
rejection, as in the [existing retirement contract](address-space-retirement.md#physical-release-and-charges).
The registry records such backing as terminal quarantine and never revisits its
addresses. A payload/physical interruption retains the slot when completion was
not reached. No physical release or slot completion occurs in receipt Drop.

## States and diagnostics

| State | Ownership and permitted action |
| --- | --- |
| Awaiting retry | Complete root remains owned; an explicit admitted attempt can retry invalidation. |
| Running | One attempt owns the root outside serialization; competing claims remain busy. |
| Retry limit | Complete owner remains retained after two failed attempts; no further retry is admitted. |
| Recovered | Invalidation and every physical release succeeded; record history is reusable with a new serial. |
| Quarantined | Physical release rejected backing; its original charges remain consumed and the record is terminal. |
| Abandoned | Completion ownership was lost before final status publication; no retry is authorized, including when a controlled walk had already completed. |

The attempt borrows the registry, whose per-slot abandonment cells are stable
outside its mutex. Its Drop performs only an atomic mark and the root receipt's
quarantining Drop. It never locks, logs, invokes callbacks, frees backing or clears
admission. Completion disarms that mark under the registry hold **before**
publishing reusable history, so an old attempt cannot mark a successor slot.
Diagnostic page counts describe the original captured accounts, not live refunds.
Recovered confirms this final root walk and slot completion; previously quarantined
provisional pages still consume their original charges and are not reclaimed by
this registry. These counts exclude quarantine outside registered final-root retry.

A bounded kernel snapshot copies at most eight records. Thread-statistics wire
version 8 additionally exposes six aggregate state counts, rejected admissions
and capacity only to a validated `SystemObserver`. Ordinary caller snapshots
contain zeros in all eight fields. `observe` forwards them in its existing
`OP_THREAD_SNAPSHOT` owned-memory response; this grants no mutation authority.
Recovered counts reflect retained history, not lifetime cumulative recoveries.

## Boundaries still required

This is C03 in the [cross-category cleanup/recovery map](cleanup-recovery.md).
Its custody/claim protocol is the first implemented example; it supplies no
generic retry proof for the other categories. Common controller and supervisor
reconciliation work is tracked as G5/G6 there, after context and ownership
qualification in G1–G3.

No automatic worker or authenticated operator retry endpoint is added. A trusted
controller must call retry outside unrelated guards and retain its own policy
state; recovery does not clear a supervisor's cached failure or authorize a
restart. Interrupt-state rejection does not prove that every nonmasking lock is
absent. Registry admission cannot recover abandoned owners, partially released
hierarchies, loan/mapping/scratch claims, device state or kernel-stack reservations.
Subsystem-cleanup failures before final root detachment remain fenced.

Retry limits and quarantine have no force-clear path. A stronger authenticated
reset/reboot boundary and platform qualification are still required for any
future reclamation of uncertain backing. **SEC-18 and R18-3 remain partial.**
Execution evidence: [2026-10-08 recovery audit](../reports/audits/2026-10-08-security-root-recovery.md).
