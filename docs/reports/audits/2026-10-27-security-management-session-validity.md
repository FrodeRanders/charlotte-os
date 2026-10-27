# Established management credential validity

Date: 2026-10-27. Parent: `721ff016`. Follow-up to
[fresh certificate validity](2026-10-27-security-management-validity.md) and
[ingress availability](2026-10-27-security-ingress-availability.md).
SEC-10 remains partial. The 38-finding ledger remains **29 corrected in scope,
seven partial and two open**; production remains disabled.

## Corrected authority boundary

The parent checks client dates during the TLS handshake but can retain an
established identity after its credential expires or its UTC provider becomes
unavailable. Two added host regressions fail against that implementation:
`established session retained expired authority` and `established session ignored
lost UTC` (**0 passed / 2 failed**). Both advance the credential clock while
remaining inside the original monotonic session budget. This is an observed
engine authority defect, separate from the preliminary native failures below.

`Config` now retains the same shared time provider and exact enrollment/WebPKI
verifier used by Rustls. Every public session transition checks nonzero UTC,
observed UTC rollback and the existing monotonic budgets. Authenticated chains
are reverified when the observed UTC second changes; repeated fragments in that
second reuse the result. Finished triggers another check before identity
publication. This reuses WebPKI rather than adding a certificate parser or a
separate expiry-only verifier. Private-key possession remains Rustls's handshake
proof. Expiry, clock loss/rollback or another verification failure latches the
connection's error, removes its identity and erases staged input/output. Clock
restoration cannot revive it.

The unchecked identity peek is replaced by
`Session::identity(now_ms) -> Result<Option<Identity>, Error>`. Native transport
also checks before socket submission or pending-result observation. Its existing
containing connection retains the original pending RPC, session and socket;
expiry gives no cancellation, quiescence, physical cleanup or replay authority.
An already accepted controller mutation can complete after credential expiry.
The same existing close/Drop policy confirms loan cleanup or aborts. C18's
[owner-family row](../../reference/cleanup-recovery.md) records this boundary.

Idle connections retain their existing readiness waits. No periodic expiry
polling, new timer, registry or cleanup owner is added. Verification is cached
at the provider's existing Unix-second granularity; all eleven profile limits,
including the original 5-second handshake and 60-second session budgets, remain
unchanged and boot-configurable. The native provider still projects its existing
positive time-service anchor; these checks do not continuously query SNTP or
prove that time remains synchronized/authenticated.

Decision auditing adds `credential_utc`, the checked identity's UTC second.
Deployment ingress also audits the first complete header before its full body
arrives, allowing a held upload's authority to be identified without dispatching
a controller mutation. Complete requests retain their dispatch audit. Records
contain the existing certificate digest/role/permission and connection ID, with
no raw certificate, key or request body. The timestamp is diagnostic evidence,
not a trusted-time attestation.

## Native execution and evidence

Both architecture service builders complete signed bundles. The final commands
use fresh isolated storage, exact resource-bound kernels and these cases:

```sh
python3 scripts/run-management-startup-tests.py --architectures aarch64 \
  --cases chain-validity validity --reuse-bundle --timeout 150
python3 scripts/run-management-startup-tests.py --architectures x86_64 \
  --cases chain-validity validity --reuse-bundle --timeout 150 \
  --status-port 26443 --deployment-port 26744
```

The disposable complete resource enrolls expired/future intermediate chains
with currently valid reader and administrator leaves. OpenSSL verifies client
purpose with dates disabled, then requires the exact date error at depth 1.
All four combinations reject on each interface, followed by a verified healthy
reader: **16 issuer-date controls across ARM and x86**. A separate case repeats
the existing **16 fresh-leaf date controls**. Each requires its exact new native
date event and zero application bytes; silence, timeouts, a generic policy error
or server-verification failure cannot qualify. Status recovery is HTTP 200;
deployment's ordinary observation control is HTTP 404.

Each architecture additionally accepts a real signed policy mutation through an
enrolled administrator and reads back generation/sequence 1. It then holds a
second valid signed mutation short of the final body byte across intermediate
expiry. The issuer is issued after boot, with the same subject/public key and
unchanged pinned leaf; no guest clock, trust, enrollment or limit override is
used. The temporary campaign owner retains the disposable CA signer through
execution and removes it on exit.

The held header's native audit proves admission before expiry. A fresh reader
on that same service obtains checked native UTC past the issuer's recorded
`notAfter`, then the final byte is submitted. The exact held connection emits
`client certificate expired`, returns EOF with **zero application bytes**, and a
healthy reader confirms the complete policy is unchanged at generation/sequence
1. Thus **two established-upload controls** pass, in addition to the 32 fresh
handshake controls. They qualify client intermediate expiry during a native
administrator upload, rather than every established-session failure context.

| Architecture | Held episode (ASID / connection) | Admission UTC (Unix seconds) | Issuer notAfter | Later checked UTC | Policy readback |
| --- | --- | --- | --- | --- | --- |
| ARM | 130 / 14 | 1793137794 | 1793137800 | 1793137801 | generation 1 / sequence 1 |
| x86 | 137 / 14 | 1793137902 | 1793137908 | 1793137909 | generation 1 / sequence 1 |

ARM's kernel SHA-256 is
`8a7b10fbcf15832704b5df5db66fb3f1c8cf9ed315dcdd5010637ad70e0c7f26`;
its resource SHA-256 is
`eec0edf1dddecbf9339ff7f712cb1e2cdb670dc6e5d3659d73009ff34997d03a`.
Its two native cases each report 18 passed / 0 failed / 0 pending. On x86 both
cases report 16 passed / 0 failed / 0 pending; the kernel SHA-256 is
`0277d928dc672bf86130cbbdd874882a181b47f598e75ebcd7182b710d42ed4b`
and the resource SHA-256 is
`d70ef949991c5052df7f4271c9cd6c429be77c5815c4463c434cd43a62a3aeb3`.
Both architectures' signed service hashes, individual
controls, checked native audit events and credential windows are retained in the
[version-controlled evidence](2026-10-27-security-management-session-validity-evidence.json).
The report and evidence require no temporary logs to interpret these claims.

## Failed and preliminary attempts

These remain distinct from the final passing campaigns:

- The first ARM and x86 campaigns pass all eight fresh issuer controls each,
  but the held upload receives application bytes and fails qualification.
  Their probe waited against host wall time, without recording the exact short
  issuer window or checked native UTC. Those private issuers have since been
  deleted, and neither response content nor final policy readback was retained.
  This does **not** prove acceptance after native expiry; it is also not a
  rejection pass. The evidence preserves both failures, artifact hashes and
  header/completion audit episodes. Clock lag is a possible explanation, not an
  established cause. The corrected observer records the issuer window and
  requires same-service native UTC beyond it before attempting the last byte.
- A strengthened x86 repeat started before its new signed bundle had finished
  atomic publication. Its earlier adapter lacked `credential_utc`, and the
  observer fails with `session-validity lacked its exact native event` before
  submitting the last byte. The final campaign starts after completed bundle
  publication. A passing ARM repeat before the equivalent Clippy `if let`
  cleanup is retained separately from final-source execution.
- Two sandboxed x86 signing attempts terminate their signer with `Killed: 9`.
  Cause is not established. Atomic staging leaves the published bundle intact;
  authorized repeats complete. These failures do not qualify guest execution.
- A sandboxed host interoperability run cannot bind its local socket
  (`PermissionDenied / Operation not permitted`). The authorized full engine
  repeat passes. Routine compile diagnostics are omitted.

## Host validation and remaining scope

- **15 engine tests pass**, including established leaf/intermediate expiry,
  unavailable/rollback UTC, terminal failure after clock restoration, no expired
  input plaintext, and real OpenSSL interoperability. The intermediate test
  expires its issuer while its leaf remains valid.
- **257 service library tests pass** and **seven real management CLI tests
  pass**. The shared transport retains its existing ownership behavior.
- **28 Python observation tests pass**: seven chain/held-upload controls, six
  leaf validity, seven startup, five runner gate and three rotation controls.
  Negative controls reject partial application data, stale/ambiguous or wrong
  episodes, transport timeout, server-verification failure, and host-only or
  different-service clock evidence. They do not substitute for native runs.
- Strict Clippy passes for the host engine's full target set and both actual
  ARM/x86 management service targets. Both signed service bundles, kernel
  assembly-section gates, workspace rustfmt, Python/Bash syntax and whitespace
  checks pass. The Configuration chapter and its 254-page PDF are rebuilt;
  changed pages are visually checked, including an unsplit command example.
  No fresh full host-suite run or advisory scan is claimed; the lock is unchanged.

Server-certificate expiry, native UTC loss/rollback, broader established-session
faults, live factory/session retirement, ARM installed-resource rotation and broad
fault/pressure qualification remain separate. This does not qualify root-anchor
expiry, authenticate SNTP, supply protected provisioning/rollback state or add
cluster peer authentication. SEC-04/10 remain partial and SEC-08/09 remain open.
No numbered finding is closed by this batch.
