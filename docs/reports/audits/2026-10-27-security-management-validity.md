# Native management certificate-validity qualification

Date: 2026-10-27. Parent: `b5208b65`. Follow-up to
[signed-policy restart/rotation](2026-10-27-security-management-rotation.md).
SEC-10 remains partial; the ledger remains **29 corrected in scope, seven
partial and two open**. This batch qualifies existing certificate-date
validation and adds bounded native rejection diagnostics; it does not correct
an observed acceptance of expired credentials.

## Boundary and implementation

Exact leaf enrollment already precedes Rustls/WebPKI verification. It does not
bypass certificate dates, chain/constraints, client purpose or handshake proof.
The engine now exposes `ClientRejection::{Policy, Expired, NotYetValid}` through
`Error::client_rejection()`, replacing the policy-only Boolean. The native
adapter logs one static category with its existing local connection ID. It does
not log raw verifier/context contents or construct an authenticated identity for
a rejected peer. Both contextual and context-free Rustls validity errors map to
the same bounded categories. No listener, ownership, deadline, trust or limit
policy changes are introduced.

The disposable credential generator can issue four additional leaves under the
same trusted CA: expired and future-dated readers and administrators. The
complete signed/encrypted boot resource explicitly enrolls those four hashes
alongside the two ordinary operators. OpenSSL independently verifies each
control's signature, chain and client purpose with date checking disabled, then
requires its specific date rejection with current-time checking enabled. The
CA key is removed after issuance; one temporary campaign owner retains the
remaining fixture keys/profiles until both guests terminate.

The test fixes certificate windows around host UTC rather than substituting
guest time or weakening native verification:

| Control | notBefore (UTC) | notAfter (UTC) |
| --- | --- | --- |
| Both expired roles | 2026-10-25 18:28:56 | 2026-10-26 18:28:56 |
| Both future roles | 2026-10-28 18:28:56 | 2026-10-29 18:28:56 |

The host-only `validity` startup assertion requires a new, exact native
expiry/future event for each attempted client, no HTTP response and a verified
healthy reader after every rejection. The event captures service ASID and local
connection ID, distinguishing new episodes from stale log entries. Generic
policy rejection, wrong-date category, transport silence/timeouts and server
certificate verification failures cannot satisfy the assertion. Transport close
and delayed UART-file delivery share one original probe deadline.

The verified TLS/HTTP exchange helper moves from the rotation campaign into
the existing common management probe module. It retains the original total
budget, bounded pre-HTTP listener-handoff retries and no replay after HTTP
submission. The startup campaign and both architecture runners share the new
assertion through their existing common boot helper. This is test configuration,
not an alternate service trust or clock source.

## Executed results

Both bundled-service builds complete through the repository's architecture
builders. The native command was:

```sh
python3 scripts/run-management-startup-tests.py \
  --architectures x86_64 aarch64 --cases validity --reuse-bundle --timeout 150
```

Each guest boots fresh isolated storage with the exact same development-policy-
bound resource. All four invalid-date client certificates are tested on TLS
status port 443 and deployment port 7444: **16 rejection controls and 16 paired
healthy-reader recoveries pass across both architectures**. Status controls
recover with HTTP 200 and deployment observation with the expected HTTP 404.
The existing startup reader observations and early mutation-denial checks pass
too. No invalid-date peer receives an HTTP response.

| Architecture | Kernel suite | Kernel SHA-256 |
| --- | --- | --- |
| x86-64 | 16 passed / 0 failed / 0 pending, bitmap `0x3bbff` | `215d5634c6bf58af0bd678a15a0e76a5d9ad2870af8c1fc5f9c9b91bf81e944d` |
| AArch64 | 18 passed / 0 failed / 0 pending, bitmap `0x3ffff` | `1e1723c36c43e79e461f823a31a575eb065229816d9aa2b486e080f5b00f6ba0` |

Resource SHA-256:
`c44c8722813de52fc5c587355cd6b36dffe2b669b2fad26ab571ba9fb9454f0c`.
Native time-service observations are 2026-10-27 18:30:14 UTC on x86 and
18:32:11 UTC on ARM, between the expired/future windows. They are observations
of the current SNTP/holdover source, not authenticated security-time evidence.
The final native attempts pass. Earlier rotation/startup failures remain in their
historical reports; the preliminary probe limitation below is retained separately.

The [tracked evidence](2026-10-27-security-management-validity-evidence.json)
retains all sixteen role/date/interface outcomes, exact native events and
healthy audit records, credential windows/hashes, kernel/resource/service/source
hashes and host checks. It needs no temporary captures to interpret the results.

## Host validation and limits

Review found a negative-probe acceptance gap in the parent exchange helper: a
single application byte followed by EOF becomes `EOFError` while waiting for a
complete HTTP response. Negative probes accepted that error when their native
rejection event existed. A controlled stream using the parent helper reproduces
that result; the current helper rejects immediately with `unauthorized client
received application bytes`. Both validity and rotation negative exchanges now
explicitly disallow every application byte, including partial headers.

The preliminary native campaign passed its category/recovery checks on both
architectures, but did not independently exclude partial application data. No
unauthorized application byte was observed from either guest. After correcting
the helper and adding partial-response regression controls, the complete native
campaign was repeated on both architectures. The results above are the strict
repeat; the earlier artifact hashes and qualification limit remain in the evidence.

- **11 TLS engine tests pass**, including actual handshake classification for
  pinned future/expired peers and loss of the clock, with no authenticated
  identity after rejection. Existing unknown-enrollment and other authentication
  controls retain their distinct outcomes.
- **36 Python tests pass:** six validity-observation controls, seven startup
  observation controls, five runner-gate controls, three rotation-observation
  controls, eight actual build-input controls and seven real-CLI deployment-
  status controls with the ordinary shared TLS fixture. The new rejection tests
  cover stale, wrong-category and generic-policy events, server verification,
  transport timeout, partial application bytes, complete role/date coverage and
  healthy recovery.
  The startup, runner-gate, rotation and validity observation suites are now
  registered in `scripts/run-host-tests.sh`. These are targeted suite results;
  a fresh full host-suite run is not claimed.
- Strict Clippy passes for the host engine and both actual management service
  targets; native assembly-section checks, workspace rustfmt, Python/shell syntax
  and whitespace checks pass. Both service bundles are rebuilt. The dependency
  lock is unchanged and no new advisory scan is claimed.

This qualifies fresh TLS handshakes with already expired/not-yet-valid enrolled
clients on both architectures. It does not qualify expiry during an established
session, issuer/intermediate expiry, server-certificate expiry, live factory or
session retirement, native loss/rollback of time, or authenticated time provenance.
No protocol format or adjustable capacity changes, so the Configuration chapter
and PDF remain unchanged. SEC-09 remains open. Protected installation/custody and
the observed older-kernel rollback gap remain SEC-04 work; ARM installed-policy
rotation, leader movement and broader fault/pressure qualification remain SEC-10
work. No numbered finding is closed.
