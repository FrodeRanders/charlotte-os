# SEC-04 exact boot-verification receipts — 19 October 2026

## Result and scope

Baseline `d224a13` commits the Raft snapshot-stream correction. This batch
returns to SEC-04 and narrows the shared boot handoff: kernel preparation now
requires `VerifiedBootPolicy`, produced only by exact-installed verification.
The broader enrollment/rotation receipt cannot be passed directly to it.

The x86 Secure Boot fixture already rejected an uninstalled successor with its
own assertion. No baseline unsafe native boot was reproduced. This change moves
that rule into the shared API and makes it part of the kernel constructor's
input contract, so a future adapter cannot simply pass a generic verification
result and omit the separate comparison.

SEC-04 remains partial. Production remains disabled. Protected enrollment,
rollback-resistant storage, serialized installation/recovery, authenticated
platform-service roots and recipient custody are still unimplemented. The
[evidence manifest](evidence/2026-10-19-security-boot-receipts.json) records the
source, model, host/native results, artifact hashes and failed attempts.

## Shared contract

`signed_policy::verify` keeps its enrollment/installation behavior: it accepts
the exact current record or a valid direct successor with the expected
predecessor. `verify_for_boot` uses those same authentication and lineage checks,
requires an installed expectation, then compares the exact sequence/digest.

- Enrollment rejects with `InvalidExpectation`.
- An authentic but uninstalled successor rejects with `UninstalledRevision`.
- Rollback, conflicting current digests, wrong context/signatures and malformed
  records retain their existing rejection paths.
- Success returns a private-field `VerifiedBootPolicy`. It has no conversion
  from `VerifiedPolicy`; the kernel handoff accepts only the boot receipt.

The fixture uses that shared function and removes its separate assertion. Its
native successor diagnostic and result-parser regression now require
`UninstalledRevision` before publication. The old assertion marker, an unrelated
panic, silence or a rejection followed by publication cannot pass that case.

Neither verifier reads or commits installed state. `installed_expectation()` on
a generic receipt derives proposed public state; it does not prove installation.
A caller could supply that proposed state itself, so the enclosing adapter must
authenticate its inputs and retain the installation/publication fence through
recipient acquisition and publication. The typed receipt captures state once;
it does not revalidate a concurrently changing protected store.

The existing zeroizing recipient owner, cluster/public-key checks, one-shot
publication and common manifest/kernel view remain. No resource cleanup owner,
recovery registry, physical completion proof or key custody is added. Public
policy bytes, CLI verification modes, persistent formats and runtime ABI are
unchanged.

## Validation

### Host and build checks

The shared-protocol package passes **56 tests**, including **12 signed-policy
tests**. Three new tests distinguish installed boot selection from enrollment,
two authentic competing successors, independently advanced state, rollback/fork
rejection, invalid context/signature/framing and the maximum u64 revision. The
reproducible kernel-fixture test also checks its previous/conflicting/successor
records through the new boot verifier.

The signer passes **18 tests**; enrollment/rotation CLI behavior remains intact.
The Secure Boot evidence parser passes **five tests**, including strict
successor diagnostics and rejection-before-publication. Warning-denied Clippy
passes for all shared-protocol targets. Shell signing-policy controls still
reject production/unknown modes early. ARM and x86 service bundles build/sign;
the ARM final-source/signature check passes. Formatting, shell/Python syntax,
documentation targets and diff whitespace checks pass.

### Native execution and retained failures

The full disposable x86 Secure Boot matrix passes **14/14 cases**. Intel VT-d
and AMD-Vi each reach the signed-policy handoff and report **15 passed / zero
failed / zero pending**, bitmap `0x1bbff`. The other cases reject unsigned,
foreign-signed or altered loaders; substituted config/kernel/policy bytes; bad
inner signatures; old/conflicting/uninstalled policies; and missing/duplicate
modules. Inner-policy failures precede trust/service publication. Evidence
retains the exact diagnostic for each case and hashes its complete logs.

The first native attempt failed **before QEMU execution**: its fixed 64 MiB FAT
image helper reported `Disk full`. Inputs included a 66,334,264-byte debug kernel
and the loader, plus filesystem metadata; the helper did not log the precise
copy substep. The host filesystem had ample free space. No
boot result is inferred from that failure. The fixture now uses a fixed
**128 MiB** media bound; the complete matrix repeats in a fresh output owner.
The failed preparation log and passing repeat remain separate. Disposable keys
and mutable VM images are removed by the existing containing owner.

The guide also passed an unsupported `--embed` argument to the x86 builder.
That initial command exited with usage before building. The corrected command
and guide use `scripts/build-catten-services-x86_64.sh` without that argument.

The normal ARM development boot passes **17/0/0**, bitmap `0x1ffff`, including
the migrated common handoff tests. It exercises development selection and local
public fixture slots, without claiming ARM Secure Boot or protected enrollment.

### Formal and manual correspondence

Both unchanged `CharlotteBootTrust` instances run freshly:

| Instance | Generated | Distinct | Depth |
|---|---:|---:|---:|
| Fixture/development | 689 | 234 | 15 |
| Conditional protected installation | 2,580 | 796 | 18 |

All required actions have positive total successor coverage; all ten weakened
configurations fail on their named invariant/action. The existing exact-state
guards already modeled the fixture's rule, so this API change needs a new source
mapping rather than a different model transition. Conditional live-state guards
still require an unimplemented enclosing installation/publication fence.

All **178 model/config/harness fingerprints** match the committed baseline.
The **12 boot-trust cases** run freshly; **131 unchanged cases** reuse the prior
evidence chain. Inventory remains **34 modules, 36 safe cases and 107 expected
counterexamples**. No fresh single full-suite run is claimed.

The manual builds in temporary output and keeps the tracked user PDF unchanged.
The updated boot subsection receives visual review; unchanged chapter/contents
rasters are compared with the preceding reviewed render. Intermediate long-type
overflow is corrected before the final render. The living trust reference,
guide, API record and conformance map describe the same receipt distinction.

## Remaining SEC-04 work

The next implementation boundary is protected policy installation and recovery:
authenticated platform enrollment, an atomic rollback-resistant exact-state
commit, settlement/fencing after interrupted writes, serialization through boot
publication, authenticated initial service roots and custody-bound recipient
release. The conditional model specifies some ordering obligations, but neither
it nor this public receipt API implements that platform boundary. Finding counts
remain **29 corrected in scope, six partial and three open**.
