# Architecture-aware executable selection

Implemented before resuming the API-removal sweep, on top of `a1641205`.
One logical component now carries signed binary variants. Placement filters
architecture, exact runtime ABI and required CPU features before capacity and
replica/affinity ranking, including explicit pins. Each concrete assignment
commits its chosen variant index. Agents use that pin; kernel admission
independently checks the actual ELF machine, complete digest and signed target.

The current formats are `CDEPLOY6` with a 152-byte header, mandatory execution
profiles in capacity reports, and catalog-v16 `CATALOGG` with per-replica pins.
Retired descriptors, snapshots and descriptor-less publication reject. No
production stores need conversion; future store evolution still requires an
explicit interruption-tested migration. The runtime/ISA contract and signing
commands are documented in [execution targets](../../reference/execution-targets.md).
Markdown guides, LaTeX/PDF, diagrams and contributor instructions were updated.

The kernel reports the intersection across configured LPs, conservatively
using baseline features until all have reported. Current optional integer
features are ARM LSE/CRC32 and x86 CMPXCHG16B/POPCNT. Runtime ABI matching is
exact. Report identity/session/boot fences remain in place; these observations
are not hardware attestation. Platform authentication, protected enrollment,
CPU hotplug and vector context contracts remain separate boundaries.

## Verification

- `scripts/run-host-tests.sh`: **418 passed, zero failed, four ignored** Rust
  tests, plus passing signing-policy, boot-result, secure-boot-result and syscall
  emitter checks. Tests cover ARM/x86 selection, ARM-only shortage, explicit
  incompatible/nonmember pins, unknown profiles, CPU feature and ABI mismatch,
  every-compatible-node policy, signed target/key/digest tampering, duplicate
  targets, exact query/snapshot pins and current-only rejection. A compatible
  existing pin survives a feature gain; a lost feature permits an explicitly
  committed replacement pin in a new generation with the same node set.
- AArch64 and x86 service bundles rebuilt/signed: **41 and 37 ELFs**. Strict
  service Clippy passes on both targets. Kernel Clippy passes on both S3 profiles;
  the mixed deployment profile is also checked with its explicit signed fixture.
  Formatting, shell syntax and whitespace checks pass.
- Central-store signed-ingress fixtures: ARM **20/0/0**, x86 **18/0/0**
  (`passed/failed/pending`). Both descriptors list ARM first and x86 second.
  ARM selects index 0; x86 logs `selected variant=1 architecture=X86_64 abi=1`,
  performs the S3 GET and verifies **32,544 bytes**. Both launch principal
  `0xa31bf89d72682689` and publish exact-generation readiness. Admission before
  the first committed CPU profile returns `-23`; after profile publication the
  same bounded fixture succeeds. The final HTTP mapping is `409 Conflict`.
- Linked, four-LP ARM/x86 guests: **ARM 22/0/0, x86 20/0/0**. Both expose
  runtime ABI 1/features 3, exchange profiles through the existing peer/Raft
  path, and use the same signed fixture bundle. x86 is leader; revisions 1 and
  3 pin ARM node `0x42e278ea`, revision 2 pins x86 node `0x42e27737`. ARM
  executes variant 0 and x86 variant 1. Both reach readiness in all three
  generations; cooperative retirement is acknowledged and the test-only
  zero-grace phase exercises forced retirement. Two completed remote denial
  replies form the post-migration causal barrier. Both runners exit zero.

Reproduction uses the normal development fixture keys and fresh instance disks:

```text
scripts/run-host-tests.sh
scripts/build-catten-services.sh --embed
scripts/build-catten-services-x86_64.sh
```

For each central-store run, set `CATTEN_SKIP_EMBED_BUILD=1`, use the ARM
`greet.elf` as `CATTEN_DEPLOY_ELF`, the x86 `greet.elf` as
`CATTEN_DEPLOY_VARIANT_ELF`, and distinct `CATTEN_DEPLOY_OBJECT_KEY` /
`CATTEN_DEPLOY_VARIANT_OBJECT_KEY`. Run `--deployment-ingress-test
--fresh-storage --timeout 240` on each architecture **serially**: Docker
certificates and ingress fixture files are shared.

For the mixed run, start both runners together, using `--deploy-test
--fresh-storage --timeout 300`, ARM `--net-listen 19057 --mac
52:54:00:12:34:01`, and x86 `--net-connect 127.0.0.1:19057 --mac
52:54:00:12:34:02`. The approved socket fixture uses
`CATTEN_ALLOW_SANDBOX_NETWORK=1`. Instances were `multiarch-complete-arm` and
`multiarch-complete-x86`. The helper prepares six current signed descriptors
(two pins, three revisions) in a test-only framed bundle. It does not install
an old-format reader or production trust policy.

## Failed runs and corrections

Passing repeats do not erase these observations:

1. Initial x86 signed-ingress run stopped before rollout at
   `memory/retirement/tests.rs:34`: **“table guard leaked into final
   invalidation”**. Its kernel hash was
   `e5a88198e41dae84de948d19a45340374f065d94f47f7e03e68613e347f0e1a6`;
   descriptor/release hashes were
   `0ab9c67131f3d328e22ac3e4d7bed34d4ca7ee23f943727372fd27f17ad9268b` /
   `e670299be3b27bb57135e98c4ec4d84874138288df7ff1d01cb45e3d6236d84b`.
   The same assertion stopped the first linked x86 guest. A global `try_lock`
   failure does not identify the owning LP. The fixture now uses the existing
   IRQ-preserving current-LP ownership observation and verifies that a locally
   held table guard is detected. It adds no wait, cleanup authority or production
   retirement change. The original capture does not establish which LP held the
   lock; it therefore cannot prove remote contention was the actual cause.
2. First linked ARM guest stopped at the descriptor-less controller check:
   **`unsigned deploy must not publish: Some(-13)`**. The fixture had confused
   untrusted-artifact `-11` with untrusted-descriptor `-13`. The corrected check
   retains upload identity, catalog absence and key-ceremony coverage. Initial
   linked boot-image hashes: ARM
   `2ab86ee96157af40b10679ef49522983c62193fc0a6e3f54deb1281a11feb042`,
   x86 `4d7d83c1fb44385c6af3a8680efec7628f468f11bbb45ef72926514ab995d472`.
3. The next linked pair launched both matching variants but expired waiting for
   the old positive `greet` forwarding probe. Automatic placement also moved
   its automatic test assignment; phase-specific signed pins
   now prevent that interference. Those boot-image hashes: ARM
   `6ee4ca7f50e2ddd780154f894627fb9b8ad592a369de1011a4ae1b345c4a76ad`,
   x86 `3db1d7ad6ff8ae7882a78db252796bc28c0d4a3fb9110bc7095e1925cf4dc6b8`.
   A pinned diagnostic repeat returned **`Some(-1)`** even after publication:
   the legacy local lookup is intentionally denied for policy-published entries.
   Pinned-repeat boot-image hashes: ARM
   `d259d41a0e7e5ea8223ff0f54a9cb6cc895d362468ff8bdd4434b41244a1a111`,
   x86 `414eed8c289588089131b539a274c2704e304aaea3994be7cb14a7f5f4054889`.
   The final fixture checks exact-generation readiness and that denial. Existing
   public echo coverage still proves positive cross-node invocation. **Authorized
   remote application invocation is separate work; this change does not weaken
   local service policy or claim that bridge is implemented.**
4. The first readiness-only repeat used an unregistered marker, which the normal
   publisher correctly could not resolve. The final phase-1 barrier republishes
   the follower's already-live public echo instead. Readiness-only failed-repeat
   boot-image hashes: ARM
   `9b03f71696a2978a3805fc0b80d2e2b4c788251390f8c86979f42c9bb3971283`,
   x86 `fe0eddda1ddb7b7c9c2440bdc62193353cc65aefa2a9d121081fae387dcf2f8c`. It prevents revision 2 from
   overtaking the follower's observation of revision 1. The raw kernel fixture
   consumes unused observed reply caps and completes calls outside IPC; its
   bounded readiness queries are not physical teardown proofs. Existing root,
   domain and retirement owners retain those obligations. See the C03/C18
   evidence in [cleanup/recovery](../../reference/cleanup-recovery.md).

## Artifact identities

Hashes below are SHA-256. They identify actual executed artifacts rather than
linking temporary logs or ignored output as evidence. A boot-image hash covers
its packaged kernel. Ingress runs preceded the final controller pin-repair
refinement; final mixed guests use the final service bundles and host tests
verify that refinement directly.

| Artifact | SHA-256 |
| --- | --- |
| ARM central-store boot image | `7597e4a6b7aa32f015584c5f63cdcf3d6d37cd2aff49f3e2e7610c9f26ccfd6a` |
| ARM central-store descriptor | `75d38392b54d0f7d7092e982d91d34c0b1fcd01b08b7503990ebc371a98a684d` |
| ARM central-store release | `fc7575e850ab4a3ad7d1be08ee4f44297fcb8ab574099e055040934c513cbf38` |
| x86 central-store boot image | `f2b08acb470a8443e15a598b0de4a34ae632e22a1a1293f2f9bdc11c59c8482d` |
| x86 central-store descriptor | `1a3001f79ed3379d8a4724de44d4640e4daa161f0bee66731c5cd8c36a31e89c` |
| x86 central-store release | `85540237734828f6868c23cdb7b1d13dbe4b84a8cc2bb20f5e6f2574807c81d3` |
| Final mixed ARM boot image | `f04d1527661580b0f85c6d4fe62221c0f17a6207686320489b7e3b1bcf4d9df8` |
| Final mixed x86 boot image | `10a8809bccca7b22b9272dae10f224a88a3ec1195950a0682f9ac99cf91c9d4d` |
| Identical signed fixture bundle on both nodes | `77089276011df8a010f1a25b925ad48e761653f8e1937a53c3f353239d473720` |
| ARM greet | `f091dca65f53d7b57d815c6fb11ba1dbec3e182796acfb1e4374a8c36642d7b7` |
| x86 greet | `f2ee8622cd78a1d480e77dea74ae38a6be9194da5d9d355d1b1d63385a0c00ad` |
| Final ARM DNS | `4a53d920cd1e03d215542c82ea97596174a77fd60da7a19b43f6dc2bf1b6b102` |
| Final x86 DNS | `9b01077198f9d14274eca28359945f8318128f3931c2a6480a8e5e1dbf78cd14` |
| Final ARM agent | `fa3db65a231c7695205366c57808ccaa29af639b89889c9aedf5d8b58ddabb64` |
| Final x86 agent | `5b3f1380a1c892230488421c1b47b058315c6c1776a41873d3322fd587590503` |

Sorted `filename:ELF-digest` manifests, newline terminated, contain 41 ARM and
37 x86 entries. Their hashes are ARM
`4f634899c0fb0368810db805efbdb6b55c744e970f93571f1f306c333516edde` and x86
`5775198f6dea5bebd789d48edcd3af9eba62bdae5f6a0ba201bee472eec6f69f`.
