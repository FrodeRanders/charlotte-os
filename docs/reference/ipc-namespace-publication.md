# IPC namespace publication

Each root registration prepares an IPC namespace node and its endpoint and
connection/call/reply budget accounts before lifecycle or root-table publication
guards. All three allocations are fallible. `PreparingRegistration` retains them
with the unpublished root, unified capability namespace, memory sponsor and root
metadata. Rejection cancels unused preparation after local guards leave and then
disposes the unpublished root. Abandonment retains every partial field.

Root publication captures a first `AddressSpaceOperation` under the existing
lifecycle/table hold. The registration owner keeps this exact-generation lease
while those guards leave and the prepared namespace links under IPC alone. The
node stores that captured handle. No allocation, root lookup or account creation
runs inside this namespace insertion. The registration releases its lease after
IPC unlock and returns the root handle only after publication completes.

The lease prevents cleanup and slot reuse during this handoff. An immediate
close rejects with `OperationsInFlight`; a staged close must wait for the same
lease before starting subsystem cleanup. Other callers can observe a registered
root before its IPC namespace links; IPC admission rejects that interval.
Publication adds no lifecycle/table-to-IPC lock edge: existing IPC operations can
still look up roots under IPC. An abandoned handoff retains the containing owner
and lease. It has no retry or recovery entry point.

The BSP prepares the kernel namespace after heap setup, before schedulers/APs
start. Its immortal root needs no publication lease. Runtime endpoint creation,
connection/call admission and receiver publication require an existing namespace;
they never recreate one after close. Serialized fixtures explicitly prepare
synthetic namespaces using IDs above the root table's addressable range. These
fixtures receive no platform reserve classification.

## Cleanup and accounting

The namespace registry uses the shared `AdmittedMap` storage. After its existing
fenced token/loan/capability drain, cleanup detaches the empty namespace node under
IPC and releases that node and its account references after IPC unlock. A
detached owner's fallback retains the complete node and accounts without locks
or allocator entry. This extends the existing C10 family; root registration uses
the existing exact-root operation mechanism.

Delegated connections, closed endpoints and other outstanding record charges
retain their original budget accounts independently. Removing a namespace
cannot refund those charges or redirect them to a reused ASID. This local empty
node disposal needs no new physical cleanup proof; all existing loan/root proof
requirements remain in force.

This change covers namespace construction and its local registry disposal. It
does **not** make IPC operations fully fallible: per-capability, endpoint,
pending-call and reply-token maps still allocate under IPC. Their payload and
charge destructors, wider caller guards/IRQ masks, namespace/account byte and
principal ceilings, and sustained pressure progress remain SEC-07/18 work.
Ordered namespace lookup/insertion is linear; its hold time is not qualified by
the serialized tests.

See the [audit and execution evidence](../reports/audits/2026-10-24-security-ipc-namespace-publication.md)
and [cleanup coverage](cleanup-recovery.md).
