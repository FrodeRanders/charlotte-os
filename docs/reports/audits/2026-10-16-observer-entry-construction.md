# Shared observer-entry construction ownership

Date: 2026-10-16. Baseline `793f508`, following initial completion admission.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and scope

`PreparedRegistration::try_new` passed an initialized `EntryNode<C>` containing
an owning weak callback reference into its fallible Box allocator. A rejected
allocation could therefore destroy that reference inside the allocator callback,
including deallocation of a callback's final weak-only backing. The original
charge remained a separate local, with no complete inert owner spanning both
payload and empty backing phases.

Waiter sponsors and completion callback preparation share this constructor. The
timer producer payloads inspected first retain their own enclosing owners;
this batch addresses the shared entry constructor where a resource-bearing
payload still crossed allocation. It is a source finding, not attribution of
historical panic, exact-worker-exit, stack, persistence or NIC failures.

## Complete partial-entry owner

`PreparingEntry<C>` captures the original weak observer and exact entry charge
before allocating empty `Box<MaybeUninit<EntryNode<C>>>` storage. The allocator
receives no payload or charge. Rejection leaves all original fields with the
same owner until explicit cancellation after allocator return. Cancellation
frees unused empty backing, then the weak reference, then original admission.
Remaining fields stay inert between those disposal steps.

Successful completion initializes that same private Box once with ID zero,
no successor and the exact original weak observer. It transfers the initialized
node and its outside charge into the existing `PreparedRegistration`. This
performs no allocation, callback, list lock or disposal. There is no new unsafe
code, observer upgrade, public entry identity, queue publication or readiness
transition during initialization.

Abandonment before or after empty allocation retains all fields without
allocation, deallocation, budget refund, callback, lock or logging work. This
is terminal retention, not a registered waiter, Pending, physical quiescence
or retry/custody owner. Ordinary allocation/cancellation still requires a
qualified containing guard/IRQ context; explicit disposal does not remove the
caller's unrelated guards.

Waiter and watch charge types remain distinct and retain their original accounts
and captured classifications. One Box per entry, node layout, public errors,
list capacity/ID checks, borrowed publication, token cancellation and source
notification contracts remain unchanged. Completed unused preparation, list
entries, registration tokens and notification batches retain their existing
active cleanup policies. No compatibility allocator path, new registry, owner
family, physical proof, deadline or ABI change is added.

## Native controls and retained cost

Serialized pre-AP controls exercise the production generic implementation with
private original watch accounts and actual heap/list guard holds:

- Allocate empty storage for 64 preparations, then initialize them beneath
  simultaneously held heap and actual list-state guards. Check the exact weak
  observer identity, initial ID and empty successor without allocation/disposal.
- After guard exit, dispose 32 unused completed preparations. Publish the other
  32 through borrowed allocation-free registration under a held heap guard, then
  cancel their actual tokens outside guards. Charges and weak references return
  exactly; no callback runs.
- Reject allocation beneath held heap/list guards while the owner retains its
  original weak reference and charge. Explicit post-guard cancellation checks
  context and original admission before backing, weak and charge disposal.
  Unused successful empty storage follows the same cancellation path.
- The ordinary constructor receives a callback's final weak reference and a
  rejecting allocator that holds heap/list guards. Final weak backing frees
  only after that callback and its guards return, before original admission.
- Abandon both partial phases under heap/list guards after the original strong
  callback references have gone. Both final weak-only allocations and original
  charges remain; enclosing IRQ state is preserved.

The last control deliberately retains **two original ordinary watch charges,
two weak-only callback control blocks and one empty entry Box** per guest.
The two charges retain one private original watch account. Its temporary list
and independent list charge are released after the guards. No list charge,
registration token, waiter sponsor, record/timer charge, authority, user root or
physical pin is retained by this new control. Earlier fixture costs are recorded
separately. This is terminal fixture retention until guest shutdown, never a
physical retry or recovery interface. The fixture runs after the existing full
list-admission controls.

Existing waiter/CQ/IPC and completion callback tests exercise the same new helper,
including success, quota/readiness rejection, cancellation, source close and
exact captured-identity replacement. This batch qualifies shared partial entry
construction, not completed `PreparedRegistration` abandonment, callback-containing
preparation, source/batch/token destructors, aggregate metadata admission or
every outer guard/IRQ context. Existing C16/C18 and local waiter publication rows
are updated in [cleanup/recovery](../../reference/cleanup-recovery.md).

## Preserved pre-boot image failure

The first Intel security runner exits 1 before data-image creation or QEMU:
`Disk full` while copying inputs into its fixed 64 MiB FAT32 boot image. The
host filesystem still had about 160 GiB available. Its kernel ELF is 65,861,080
bytes, EFI loader 368,640 bytes and config 308 bytes: **66,230,028 input bytes**,
exceeding a separately measured empty 64 MiB FAT volume's **66,058,752 free
bytes**, before EFI directory overhead. This establishes a container-capacity
failure, with no guest panic or CPU capture to attribute.

[Separate failure evidence](evidence/2026-10-16-observer-entry-image-failure.json)
retains the exact failed command/environment, kernel/loader/config and reviewed
Rust hashes, pre-fix runner hashes, exit, geometry observation and concise
failure diagnostics. The atomic helper removed its failed temporary image;
no partial image hash or guest UART exists. No old artifact was deleted.

All runners and the standalone creator now share a 128 MiB disposable image
default, matching the prior ARM/RISC-V capacity. Explicit creator size overrides
remain checked. Data-store sizes and existing persistent disks are unchanged.
Two real-mtools host regressions verify that the default packages/extracts a
payload larger than 64 MiB without truncation, and that explicit undersizing
rejects without replacing prior output or leaving temporary images.

Intel security is repeated under a distinct instance with fresh signed fixture
inputs. Runtime Rust stays frozen. The first four guest profiles used pre-fix
runner scripts (128 MiB ARM, 64 MiB Intel/AMD); the Intel security repeat uses
the shared 128 MiB default. Evidence records both tooling snapshots and actual
image sizes. Earlier guest results are retained as observations of their exact
inputs, not a claim that the final scripts were used for all profiles.

## Validation and evidence

The host runner passes **509 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls, including both image
regressions using actual mtools. Shell syntax checks pass for all five changed
scripts. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting,
diff whitespace and relative documentation links pass.

Six reviewed Rust files were frozen before the five-profile matrix. Six tooling
files were frozen after the documented capacity correction and before its
repeat. Every profile uses four LPs, fresh storage, a 180-second guest budget, denied Rust
warnings, assembly-section checks and automatic all-CPU failure capture.
All five completed guest runs exit zero with diagnostic result `complete`, zero
failed/pending. The earlier pre-boot runner failure remains separately recorded above:

| Profile | Deferred tests passed | Completion |
| --- | --- | --- |
| ARM / SMMUv3, no network | 17 | 11.454607 s |
| Intel VT-d, no network | 15 | 14.498231 s |
| AMD-Vi, no network | 15 | 19.872938 s |
| ARM / SMMUv3, security | 19 | 57.023228 s |
| Intel VT-d, security | 17 | 62.547153 s |

The new entry-construction controls pass on every profile: 320 private
initializations beneath held heap/list guards, exact weak observer identity,
160 unused completed disposals and 160 publications/token cancellations total,
retaining rejection, final-weak disposal after allocator return, and both guarded
partial abandonment phases. The new terminally retained fixture costs are
itemized above and in the evidence. Existing initial-admission, record-construction,
event/node preparation, cancellation handback and callback/waiter disposal
controls also pass. Each profile completes 64 scheduled IPC close/exit episodes
with exact exit watches (320 total).
Both security profiles complete two scoped EL0 generations. ARM retains
certified runtime migration; x86 qualifies only never-dispatched migration.
These are bounded regression observations, not sustained fairness, hostile
pressure or physical hardware qualification. No guest panic, deadline or
persistence fail-stop was observed. The preceding Intel attempt failed before
boot as preserved above; it is not replaced by this pass.

[Version-controlled evidence](evidence/2026-10-16-observer-entry-construction.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations, and new and prior fixture costs.
The five commands use `--smp 4 --fresh-storage --timeout 180`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance entry-construction-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance entry-construction-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance entry-construction-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance entry-construction-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance entry-construction-intel-security-repeat
```

All use `CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; the two no-network
kernel ELF hashes match. Security profiles add `CATTEN_ALLOW_SANDBOX_NETWORK=1`
and HTTP/deployment ports 19080/19081 on ARM, 19180/19181 on Intel.
All staged service hashes and run inventories match the prior baseline; no
service/runtime library was edited or rebuilt. Frozen Rust and final tooling
hashes are rechecked after the matrix; documentation is finalized afterward. Published results do
not depend on temporary logs or ignored artifacts and are no full-tree or
platform attestation.

The earlier introduced temporary-guard failures remain separately preserved in
[unpublished ownership evidence](evidence/2026-10-16-timer-unpublished-failures.json).
Older unattributed samples remain in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md).
These passes do not establish their causes or close the wider audit gaps.
