# Executable variants and node eligibility

One signed `CDEPLOY6` descriptor identifies one logical component revision,
its common placement, resource limits and capability grants, and one to eight
binary variants. A release still contains one descriptor per logical component.
Each variant binds an immutable SHA-256 and object key to architecture, runtime
ABI, required CPU features and selection priority. Variant targets must be
distinct. The descriptor's aggregate size remains bounded to 3,584 bytes.

The current execution contract is 64-bit little-endian ELF, either AArch64
(`e_machine=183`) or x86_64 (`e_machine=62`), and Charlotte runtime ABI 2. ABI
matching is exact. A kernel/userspace ABI change must bump the shared
`RUNTIME_ABI` and rebuild/sign components; an absent or old ABI is never filled
in by a decoder. Microarchitecture names, vendor names and CPU generations
are not compatibility tests. ABI 2 makes the twelve-register authenticated
[IPC receive envelope](ipc-receive.md) canonical and retires receive syscalls
22/27/53. Rebuild and re-sign both primary and additional variants; ABI 1
variants fail exact target matching on ABI 2 nodes.

| Feature bit | AArch64 | x86_64 |
| --- | --- | --- |
| 0 | LSE atomic instructions | CMPXCHG16B |
| 1 | CRC32 instructions | POPCNT |

Only these integer extensions are advertised. The kernel records each boot
LP's hardware features and intersects them across all schedulable LPs. Until
all configured LPs report, the profile advertises baseline features only.
This does not implement CPU hotplug. Any future LP admission must preserve the
intersection used by running binaries, or retire affected deployments before
scheduling on a less capable LP. Vector ISA profiles need a corresponding
kernel context-save contract before they can be added.

`NodePressure` (syscall 82) returns free frames, usable frames, CPU occupancy,
packed architecture/ABI and features. Architecture occupies bits 0–15 of the
fourth return, bits 16–31 are zero, and ABI occupies bits 32–63. AArch64 returns
these in x0–x4; x86_64 in rax/rdi/rsi/rdx/r10. The 58-byte `rcapacity` payload
includes the canonical 16-byte target after its frame/load fields. Replicated
capacity records and catalog-v17 snapshots retain the profile. Profile changes
within a reporter boot nonce reject; a fresh incarnation can replace it. The
existing peer source/session checks bind a report to its node. This is not
hardware attestation; production platform authentication remains a separate
trust boundary.

## Placement and committed selection

The leader filters non-draining voting members by architecture, exact ABI and
feature inclusion **before** ranking capacity, replica count or affinity.
Unknown/stale profiles cannot receive new assignments. Explicit node pins must
also be eligible and compatible. `EVERY_ELIGIBLE_NODE` means every compatible,
capacity-qualified member for this component. An ARM-only release can run on
ARM members of a mixed cluster; it cannot count x86 members toward its requested
replicas. A shortage returns `ERR_INCOMPATIBLE_TARGET` (-23), separately from
memory exhaustion or unsatisfiable affinity.

Among compatible variants, selection prefers the highest signed priority,
then the most required feature bits, then earliest signed order. The leader
commits a variant index beside each replica node. Apply validates every pin
against the applied node profile before publishing any component. Queries,
reassignment commands and snapshots retain those exact indices; a later CPU
report cannot silently select another executable for an existing generation.
If a new node incarnation loses support for its pin, the controller may commit
a compatible replacement pin in a new generation even when the node set stays
the same. Gaining features leaves a compatible existing pin alone.
Each replica's content identity comes from its selected signed variant.
There is no retained primary node, local object ID or variant-zero digest.

The agent uses its committed index, rechecks local CPU support, and fetches
that variant's object key and digest. When no central connector is registered, a local cache hit is permitted only after verifying the exact selected digest and signed artifact name; otherwise acquisition uses the central store. Its scoped launch retains the existing
owned artifact/descriptor/domain resources. Kernel admission independently
checks the actual ELF machine, full digest, signed target, signature/name and
placement metadata before namespace creation. Operational connector pickup
uses the same signed variant gate. A forged capacity report therefore cannot
make a wrong-architecture executable pass the kernel gate.

## Publishing

Build and sign each binary's CLS2 note for the **same logical artifact name**,
with coherent role/placement flags and common grants/limits. Upload each final
signed ELF under its own immutable key. Signing reads the actual ELF path to
derive machine and digest:

```text
cluster-sign deployment-sign orders.cdep orders releases/orders-arm.elf \
  ./orders-arm.elf 0 42 8 8 15000 deployment-private.hex \
  --variant=releases/orders-x86.elf,./orders-x86.elf,0,0,2 \
  --variant=releases/orders-arm-lse.elf,./orders-arm-lse.elf,1,10,2 \
  --replicas=3 --spread-replicas orders=publish
cluster-sign release-sign orders.crelease orders 42 deployment-private.hex orders.cdep
cluster-sign release-apply orders.crelease 127.0.0.1:8081 120
```

`--variant=OBJECT_KEY,ELF,FEATURE_MASK,PRIORITY,ABI` repeats for additional
variants. `--features=MASK` supplies requirements for the first binary, whose
ABI is the current runtime ABI and priority is zero. Unknown feature bits,
zero ABI, duplicate targets and oversized descriptors reject. ELF machine is
derived rather than supplied, but instruction requirements still come from
the reviewed build configuration; the signer does not disassemble an ELF to
infer every instruction. Baseline repository targets need no optional integer
feature bits. Build configurations using other extensions must be rejected or
have the execution contract extended before rollout.

## Current formats

`CDEPLOY6`, version 6, has a 152-byte header: sequence/node, name/variant/grant
counts, stack/thread limits, shutdown grace, placement policy, signer ID and
64-byte signature at offset 88. After the name, each variant has target (16),
priority (2), key length (2), digest (32), then key bytes. Grants follow all
variants. Targets encode architecture:u16, reserved-zero:u16, ABI:u32 and
features:u64. All scalars are little endian. Only this grammar is accepted.

Catalog snapshots use `CATALOGH` version 17. Versions 1–16 are retired;
descriptor-less assignments and query/snapshot records reject. Existing
pre-production fixtures must be regenerated explicitly. Future deployed-store
changes require an explicit interruption-tested migration, never a silent reset.
See [catalog snapshots](catalog-snapshots.md) and
[validation evidence](../reports/audits/2026-10-13-execution-targets.md).

Hardware enumeration follows the Arm ID_AA64ISAR0 fields documented in the
[Arm architecture supplement](https://documentation-service.arm.com/static/61b75264a2506b46a5c32065)
and CPUID.01H:ECX in the
[Intel architecture manuals](https://cdrdv2-public.intel.com/868137/325462-089-sdm-vol-1-2abcd-3abcd-4.pdf).
