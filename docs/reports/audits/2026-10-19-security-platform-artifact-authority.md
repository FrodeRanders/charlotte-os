# SEC-04 platform artifact authority — 19 October 2026

## Result and scope

Baseline `6a7c77c` commits the exact boot-receipt handoff. Source review found
that initial platform ELF loading, store verification and persistent-image
checks still used `CLUSTER_PUBLIC_KEY`, even when a signed boot policy selected
a different artifact key. The earlier fixture used development-signed platform
images, so its passing policy checks did not establish that those images used
the selected authority. No baseline unsafe native boot was reproduced.

This batch selects one immutable artifact authority before platform services
start and uses it through later loading and final policy publication. The
Secure Boot fixture now signs an isolated bundle with its policy's public test
artifact key. Substituting a development-signed name service or NVMe image
rejects at the loader's signature gate under otherwise authorized packaging.

SEC-04 remains partial and production remains disabled. Protected enrollment,
rollback-resistant state, serialized installation/recovery and recipient
custody are still missing. The [evidence manifest](evidence/2026-10-19-security-platform-artifact-authority.json)
records hashes, diagnostics, scoped model results and retained failed attempts.
Finding counts remain **29 corrected in scope, six partial and three open**.

## Selection and ownership

After BSP memory initialization, before APs or platform services start,
`initialize_boot_policy` prepares either explicit development policy or the
fixture's exact-installed signed policy. Verification and recipient preparation
run outside the slot guard. The existing boot slot holds the complete
`PreparedBootTrust`, including its zeroizing recipient owner, alongside the
immutable selected public policy/digest. A second selection rejects, even for
the same policy. Missing selection has no development-key fallback.

The name-service constructor, ordinary/platform loaders, store validation and
persistent-image verification read the selected artifact key. Signature
preflight remains before domain-root allocation. Explicit authorized
scoped/deployment-key loading keeps its separate caller-supplied key contract.

Steady-state composition takes the existing preparation once; its public
authority remains available. Final publication requires that selected public
policy and digest and rejects duplicate publication. Selection alone exposes
neither configured operational policy nor recipient callbacks. Manifests and
the final operational gate continue to share the installed view and exact
name-service handle.

Ordinary rejection returns the complete unused preparation after unlocking.
Its recipient storage zeroizes on Drop. There is no new registry, capability
cleanup, root lease, recovery custody or physical completion claim. The C18 row
in [cleanup/recovery](../../reference/cleanup-recovery.md) records this extension.
The receipt and slot still cannot serialize an external protected store.

## Execution and build evidence

The disposable Secure Boot matrix passes **16/16 cases**. Its valid Intel VT-d
and AMD-Vi guests each select revision 7 before NS startup and complete
**15 passed / zero failed / zero pending**, bitmap `0x1bbff`. Boot assertions
also verify that the NS image accepts the selected artifact key, rejects the
development key, and passes the persistent-image checker.

Two added cases compile separate immutable kernel snapshots containing one
development-signed substitution. Their configuration hashes and EFI packaging
authorize those exact kernels, so they test the inner artifact gate:

| Case | Required observed rejection |
|---|---|
| NS substitution | `[loader] name-service load failed: SignatureVerificationFailed`, after signed-policy selection and before NS startup |
| NVMe substitution | `[loader] domain load failed: SignatureVerificationFailed`, after selection and before steady-state publication |

The preceding 14 firmware/config/kernel/module/policy controls still pass.
Silence, startup failure, unrelated panic and late service publication cannot
substitute for the required rejection. Six host parser tests pass.

The harness copies the ordinary staged x86 bundle into its temporary owner and
signs the copy with public test seed 20, whose public artifact key is checked
against the policy. The signing file is mode 0600 inside the mode-0700 owner.
The test's normal storage image uses this same bundle. Kernel variants are
snapshotted before any later build; the original staged bundle is unchanged.
VMs are reaped and private keys/mutable images removed on owner exit. This
improves fixture authority routing, without making its public keys production
credentials.

The normal ARM development guest completes **17/0/0**, bitmap `0x1ffff`, with
selection before NS startup and the common handoff assertions. This is no ARM
Secure Boot or protected-state qualification. AArch64 bundled services build,
sign and pass final-source/signature freshness checks. Warning-denied kernel
Clippy passes for ARM and x86 `acpi,boot_trust_test`; formatting, syntax, signing
build-gate controls and diff-whitespace checks pass.

Failed host attempts are kept separately:

- An initial ARM check omitted `CATTEN_AARCH64_SERVICE_BUNDLE` and rejected at
  compilation. Supplying the staged bundle path passed.
- Sandboxed TLC could not open its local Java RMI socket (`Operation not
  permitted`). The scoped run with that socket allowed passed.
- Sandboxed ARM QEMU could not bind its diagnostic QMP socket (`Operation not
  permitted`) and exited before guest execution. A fresh instance with the
  socket allowed passed. No guest failure or causal kernel conclusion is
  inferred from those host restrictions.

## Model and manual correspondence

`CharlotteBootTrust` now separates authority selection, bootstrap loading and
later platform loading from final publication. Loaded-image authority is a
symbolic set; it abstracts signature verification and compresses all images
loaded before publication into one bootstrap transition. Publication requires
bootstrap loading and the same selected authority. Reboot clears volatile
selection and loading state. The conditional installer must fence state changes
once authority selection starts platform loading; this remains an unimplemented
protected-platform obligation.

Both complete safe configurations pass freshly with required action coverage:

| Instance | Generated | Distinct | Depth |
|---|---:|---:|---:|
| Fixture/development | 1,791 | 538 | 17 |
| Conditional protected installation | 5,508 | 1,548 | 20 |

All **12** required counterexamples fail on their named invariant and exercise
their named unsafe action. New controls expose development-key platform
fallback and selected-authority overwrite. The other models/configurations
are unchanged: **14 boot-trust cases** run freshly and **131 cases** reuse the
prior evidence chain. Inventory is **34 modules, 36 safe configurations and
109 expected counterexamples**. No fresh single full-suite run is claimed.

The Markdown references, conformance map and LaTeX manual describe the same
ordering and limits. The manual builds in temporary output, retains 234 pages,
and the changed chapter pages receive visual review with no chapter overflow.
Validation commands wrote only temporary PDF output. The tracked PDF changed
later during the session; that concurrent update is preserved separately.
This model does not prove actual
cryptography, stack/heap budgets, firmware or storage atomicity, key erasure,
custody, outer contexts or physical lifetime.

## Remaining work

The next SEC-04 boundary still needs a platform design and implementation for
authenticated enrollment, rollback-resistant exact-state installation,
interrupted-writer settlement/fencing and custody-bound recipient release.
Early selected-key routing is now shared with platform loading, but its
production inputs and enclosing installation fence are absent. Production
admission must remain disabled until those boundaries and their failure tests
exist.
