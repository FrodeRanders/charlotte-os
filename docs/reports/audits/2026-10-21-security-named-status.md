# Security audit: complete named deployment observations

Date: 2026-10-21. Reviewed baseline: `bad21135`.

Standalone `deployment-status` now validates the complete rollout record before
printing or accepting readiness. The shared status reader also preserves body
suffixes and non-JSON whitespace that could previously disappear before apply's
codec. The [tracked evidence](evidence/2026-10-21-security-named-status.json)
retains baseline false successes, corrected controls and source/artifact hashes.
Findings remain **29 corrected / 6 partial / 3 open**. Production trust stays
disabled.

## Findings and correction

Read-only named status previously printed any HTTP-200 body. Its waiting mode
looked for the exact substring `"state":"ready"`, accepting incomplete, nested,
duplicate or inconsistent records while rejecting valid JSON with different
spacing. Invalid artifact names were rejected by each individual query attempt,
so a positive wait retried that known input error until its deadline expired.

The command now calls the existing six-field rollout codec in both modes.
Zero/omitted wait prints any valid current state. Positive wait requires a valid
ready record; valid pending observations continue under the original deadline.
Malformed HTTP-200 data terminates immediately without output or another query.
Names reject before waiting or network admission. The codec, field grammar and
server output are unchanged.

Review of the wrapper found another boundary shared by standalone status and
both apply clients. Splitting the response on every CRLF separator and selecting
the second segment discarded any body suffix following a second separator.
Broad Unicode `trim` also removed non-JSON whitespace before the strict codec.
A complete declared body could therefore supply a valid-looking prefix to an
otherwise strict decoder. Both defects reproduced false readiness success in
`deployment-apply` and `release-apply`.

The shared reader now separates header/body once, retains the complete declared
body and trims only space, tab, carriage return and line feed. A later separator
or Unicode no-break space remains available to the codec and causes rejection.
HTTP framing, signed inputs, receipt encoding, CLI arguments and runtime ABI are
unchanged. Apply retains its admitted-generation checks and never replays accepted
notification. This correction is scoped to deployment status bodies; other
standalone HTTP observations and admission-output helpers still need their own
complete input/output review.

Named status remains an observation of the current record. That record can change
while the command waits; a valid result is not tied to an earlier admission and
grants no replay, retirement or cleanup authority. The existing host socket,
public response strings and prepared apply owners keep their prior lifetimes.
Malformed status terminates those scopes normally. The
[C18 row](../../reference/cleanup-recovery.md) records the context without adding
an owner, registry, recovery family, custody or physical completion proof.

## Baseline and corrected controls

The new [status CLI matrix](../../../scripts/tests/test-deployment-status.py) uses
a bounded loopback HTTP/1.1 peer. It invokes the actual binary, records every
GET/result and joins its peer thread. Seven methods cover **27 cases**: three
read-only states, two ready JSON layouts, pending-to-ready progress, nine malformed
bodies in each mode, omitted wait, HTTP rejection and invalid-name admission.
The baseline release binary matches the preceding deadline report's artifact hash.

Baseline results are **20 expected failed controls / 7 passes**. All 18 malformed
observations falsely succeed; the pretty ready record fails to complete, and the
invalid-name command waits for its timer instead of reporting the input error
immediately. Separate targeted baseline controls add two false successes in each
apply client: a second separator with trailing data, and Unicode whitespace
around an otherwise valid record. Existing malformed-record controls continue
to reject; those passes are preserved separately from the new failures.

Both corrected profiles pass all 27 named-status cases. Each malformed record
causes one GET, no stdout and immediate error despite a later valid scripted row.
Invalid names admit no request or timer wait. Valid pending/ready observations
and JSON whitespace/order retain their intended behavior. These are read-only
controls; their peer rejects and records any unexpected POST.

Validation passes **81 Rust tests and 66 CLI cases per profile**. The CLI total
is 27 named-status, 15 descriptor-generation, 15 release-generation and nine
existing deadline cases. Strict Clippy, workspace formatting, the signed AArch64
service build and bundle freshness check pass. All **41 AArch64 / 37 x86_64 staged
images** and **180 formal inputs** retain their preceding hashes. The evidence
links the version-controlled prior manifest and records its hash, plus hashes of
the current binaries and reviewed source. No kernel/service/model behavior changes
and no fresh guest, x86 build or TLC run are claimed. Historical reports and the
manual PDF are unchanged.

## Remaining limits

Current-name observation cannot prove which historical admission became ready.
The HTTP interface has no readiness subscription or peer authentication. Blocking
host file I/O/stdout and OS scheduling can delay return despite the checked
network budget. Server-side progress, coordinated rollback, production trust,
SEC-15 and the wider audit findings remain separate work. These host controls
establish no kernel resource-quiescence or physical cleanup proof.
