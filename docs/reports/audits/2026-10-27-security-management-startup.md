# Management startup rejection and host acceptance gates

Date: 2026-10-27. Parent: `edf65bd7`. This follows the
[native TLS migration](2026-10-27-security-management-tls-native.md).
SEC-10 remains partial; the ledger remains 29 corrected in scope, seven partial
and two open. Production trust, installed-state rotation and custody remain
separate work.

## Corrections

The initial missing-resource x86 boot exposed a regression that the earlier
positive TLS campaign did not exercise. The deployment service faulted in
`deployd::serve` before it could report disabled admission:

```text
FATAL USER FAULT: ASID=137 vector=14 error=0x6
RIP=VAddr(0x209f3) address=VAddr(0xffff70)
```

Kernel SHA-256:
`d9a667cc216f2043c1d652f84282509180dde4510ac86af7dfa00034fbe90d5e`.
The service ELF matches the preceding migration's signed x86 artifact; the
address resolved to its entry function. The entry frame exists before the
credential check. `spawn_management_service` selected the general adaptive
four-page stack when no profile existed, while its profile-backed branch used
the central management reservation. Both paths now use the same
`management::USER_STACK_PAGES = 32` policy (131072 reserved bytes, committed on
demand). No new limit or privilege classification is introduced.

That boot's registered kernel suite completed despite the management-domain
fault. The new startup assertion correctly rejected it. Kernel-suite success
alone does not prove that the management services reached their intended state.

The x86 HTTPS runner had a separate acceptance defect: identity-test failure
cleared `HTTP_LIVENESS_OK`, while final acceptance read `DEPLOY_LIVENESS_OK`.
Healthy later metrics could therefore mask a failed identity control. It now
clears the actual gate. Tests execute the current ARM/x86 blocks with controlled
identity/liveness failures and healthy metrics. The tracked historical x86 block
from `edf65bd7` accepts the same failed identity control, providing a negative
control without depending on Git history being present in a shallow checkout.
The prior positive native run really passed its identity probes; this correction
does not convert those observations into failed TLS handshakes.

Management Python probes reject optimized Python so runtime assertions cannot
silently disappear. The existing tooling-input suite now compiles the actual
kernel build script and its shared grammar through a private Cargo executable
harness; it no longer assumes the build script has no dependencies. Its copied
lock and temporary workspace leave the repository lock unchanged.

## Startup campaign and evidence

`scripts/run-management-startup-tests.py` owns the disposable keys, input variants
and child process groups. Each case rebuilds its exact kernel input and boots
fresh isolated storage. Reused signed service bundles are identified by their
artifact hashes; their runtime sources are unchanged from the parent. The normal
command builds bundles through the architecture-specific builders first.

The runner option `--management-startup-test EXPECTATION` changes only host
assertions. It does not choose kernel keys, install policy, substitute credentials
or synthesize a missing resource. It requires an isolated timed user-network
boot, and terminal rejection modes require `--no-failure-capture`. A passing
deliberate rejection makes no kernel self-test-success claim.

One public marker records the operational admission-policy revision immediately
after successful publication, outside the policy guard. Artifact authority was
already selected earlier in boot; the negative checks concern final operational
policy and steady-state service publication.

| Case | Required evidence |
| --- | --- |
| Valid | Exactly one ordered policy publication, completed suite, verified TLS reader status/deployment observations and reader mutation rejection |
| Missing resource | Both explicit disabled-state entries, ordered single publication and completed suite |
| Authenticated invalid DER | Same disabled-state evidence; an otherwise valid signed/HPKE profile carries a synthetic invalid private-key DER encoding |
| Wrong policy context | Exact `WrongPolicy`, terminal `boot admission trust rejected`, no operational policy or steady-state publication |
| Altered signature | Exact `Signature`, same terminal gate and publication exclusions |
| Correctly re-signed damaged ciphertext | Host public signature/context verification succeeds, then native `Authentication` at the same terminal pre-publication gate |

The [tracked evidence](2026-10-27-security-management-startup-evidence.json)
records each executed result, kernel/resource/service/source hashes, assertion
controls and preliminary failures. It supplies the native observations inline;
temporary captures are not required to interpret a result.

| Executed check | Result |
| --- | --- |
| Native startup cases | **12 passed**: all six cases on x86 and AArch64 |
| Valid, missing and invalid-DER boot suites | **16/0/0 on x86**, **18/0/0 on ARM**, for each of those three cases |
| Deliberate envelope rejection | Exact causes and terminal gate on both architectures; no operational trust/steady-state publication or self-test-success claim |
| Host assertion/fixture regression tests | **18 passed**: five runner-gate, seven startup-observation, six tooling-input controls |
| Strict kernel checks | Both actual target JSONs pass Clippy with warnings denied; native assembly permission checks pass |
| Formatting and syntax | Workspace rustfmt, shell/Python syntax and whitespace checks pass |
| Disabled-port observations | **12** EOF/reset/refusal observations and **four** inconclusive timeouts, retained separately |

The signed status, deployment and security-probe ELF hashes on both architectures
match the parent migration evidence. Runtime service sources, TLS formats and
the root dependency lock are unchanged. No fresh advisory scan is claimed by
this fixture/launch-policy correction.

The wrong-context case alters the envelope binding; it is not an installed-policy
revision-advance or rollback test. The damaged-ciphertext case uses only the
repository's publicly known development signing seed, kept in the temporary
fixture owner. No new signing command, crypto primitive, production enrollment
path or private-key argument is added.

## Transport observations and failed preliminaries

After the stack correction, the first disabled-port probe ran before DHCP and
timed out. Waiting for DHCP did not produce a terminal TCP result either. Both
failed probe runs are preserved separately, including the second kernel hash
`5af18ba35fb926de0fcd30f7ee4b9cfc854cc505f99fa5346641bb07df927e84`.
The latter run reached both disabled entries and its kernel suite completed.
No cause for the missing terminal transport result was established.

SLIRP's host listener can accept a TCP connection while the guest supplies no
terminal result. The final campaign reports EOF/reset/refusal separately from
bounded transport timeouts. A timeout is inconclusive and is not counted as
rejection, physical closure, readiness or quiescence proof. Disabled-startup
acceptance requires the two explicit service events and exactly one ordered
publication, independently of these transport observations. No returned
application data or successful TLS handshake is permitted. Full wire-level
closure remains unqualified where the observation timed out.

The pre-publication envelope cases instead require their exact native cause and
terminal boot gate, with no operational trust/service publication. They do not
infer successful rejection from an unstarted network stack or host-forwarder
silence. Assertion tests reject missing/duplicate/late publication, unrelated
panics, wrong/prefix-only causes, application data and timeout-only evidence.

## Remaining scope

This batch qualifies startup admission in development fixtures and fixes the
missing-profile entry-frame and x86 host-gate regressions. It does not establish
installed-resource rotation/revocation, expiry across restart, multi-node leader
movement, broad pressure or new adapter fault-path coverage. Those remain SEC-10
work. SEC-04 protected installation/custody, SEC-08 peer authentication, SEC-09
security time and SEC-15 complete erasure retain their existing limitations.
The Configuration chapter's central stack and TLS-limit values are unchanged;
the missing-profile path now follows that documented stack policy.
