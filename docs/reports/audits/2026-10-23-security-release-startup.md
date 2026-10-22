# Security audit: release startup and uncertain responses

Date: 2026-10-23 (work began 2026-10-22). Reviewed baseline: `beefef88`.

Both deployment-ingress boot workers now invoke `release-apply` once. The client
retains the exact signed envelope and one monotonic budget through startup,
admission and component observation. Its typed transport boundary permits a new
attempt only before any HTTP write, or after a complete checked HTTP 409 response
containing the single numeric error `-23`. The
[tracked evidence](evidence/2026-10-23-security-release-startup.json) preserves
source/artifact hashes, failure diagnostics and corrected controls.

The audit remains **29 corrected / 6 partial / 3 open**. SEC-06, SEC-07 and SEC-18
remain partial; production trust is disabled.

## Why this needed changing

The preceding controller-relay run reported `Resource temporarily unavailable
(os error 35); submitted work may be committed`. Its shell worker then restarted
the entire release command and eventually reported readiness. That later success
could not resolve the first request's outcome. The same loop could repeat a POST
after receipt acceptance if readiness observation failed.

The old client already submitted once per command. The unsafe continuation was in
both boot workers. Moving qualified startup attempts inside one command removes
that outer restart and keeps the original envelope and deadline alive.

## Checked continuation

`exchange_checked` distinguishes failure before HTTP submission from uncertainty
after the first possible write. Every write/read and phase completion still uses
the existing remaining-budget checks. A write error, even one reporting no
progress, stays terminal conservatively. Socket disposal provides no remote
cancellation or rollback proof.

The only checked application rejection that allows another startup attempt is
HTTP 409 with one exact numeric `error: -23`. Duplicate, extended, quoted,
malformed or trailing records reject. DNS emits incompatible-target rejection
before proposal. Catalog application checks every component's compatibility
before publishing the planned release, deployment and operational records. The
negative result may itself be committed to Raft. The new catalog regression
compares complete persisted snapshots after rejection, with both absent and
previously installed releases, and then admits the same command after a valid
new-boot target report.

Other negative codes, including busy/timeout, cannot authorize another attempt.
Truncated/dropped responses, malformed receipts and accepted rollout failures
stop. A valid receipt leads only to observation of its captured component
generations. Standalone `release-notify` remains a single attempt. No CLI position,
HTTP record, signature, catalog grammar or kernel ABI changes.

QEMU host forwarding can accept TCP before the guest listener exists. The
workers therefore wait for `[deployd] listening port=7444` and the initial
single-node DNS leader marker in their existing bounded serial observation loop
before starting the one command. They pass only the original fixture budget left, publish readiness only on success, and stop the
run on worker failure. The markers delay the first attempt; they supply no
replay, rollback or application-completion proof.

## Validation and observations

The real CLI loopback matrix has seven methods and 21 cases. It covers closed-port
startup, checked target rejection, permanent rejection, one budget across startup
and accepted readiness, malformed/other rejections, dropped/truncated/stalled
responses, malformed receipts, stale/superseding/malformed observations, and both
actual boot-worker setup/launch blocks. Every uncertain or accepted failure has
one POST. The worker controls also prove that no command starts before the
listener and leader markers and no readiness marker appears after uncertainty.

The existing seven-method release-generation suite and seven-method/nine-case
ingress-deadline suite pass. Signer unit/integration tests, service library tests,
strict signer Clippy, workspace formatting, shell syntax and whitespace checks
pass: 93 signer tests and 224 service library tests. The evidence records their
commands, results and hashes. No formal model or server protocol changed; the
previous full model run remains the formal baseline.

Final signed RustFS upload/release/pull/launch controls pass on both targets:
AArch64 has **20 passed / 0 failed / 0 pending** deferred self-tests; x86 has
**18 / 0 / 0**. Each final worker observed two complete `409 / -23` rejections,
then one valid receipt and `greet` deployment/service generation 1 with desired/
ready replicas 1/1. These qualify ordinary startup and rollout; they do not prove
saturated admission, authenticated responses or recovery from a lost receipt.

An intermediate AArch64 run started the one command immediately after QEMU boot,
before the new listener gate. It failed to read the admission response with
`Resource temporarily unavailable (os error 35); submitted work may be committed`
and did not restart the command. The guest subsequently logged launch and
publication of `greet` generation 1. A separate read-only status query returned
`ready`, deployment/service generation 1, desired/ready replicas 1/1. This is a
concrete lost-receipt observation, not a successful apply result or permission to
replay. The response-loss cause remains undetermined; a later pass with the
startup gate cannot establish it. Preserve that failed run separately from the
later gated controls.

A second intermediate run, on x86 with only the listener gate, returned HTTP 503
with `{"error":-2}` while DNS still logged `Follower`. It stopped the worker and
captured `deployment-incomplete` after 19.626 seconds, without a second command.
No rollback or precise rejection origin follows from that response. The final
fixture adds the initial DNS leader marker before its first attempt; it does not
permit retries for `-2` or other busy/routing failures.

During the initial parallel native attempts, both runners failed while
recreating their shared Docker fixture, before guest boot (`No such container`). Initial host fixture
failures also remain recorded: delayed listening raced process startup, a bound
non-listening socket timed out on macOS, and a test target report was correctly
rejected for changing architecture within one boot nonce. Those fixture
corrections do not change the checked rejection or uncertainty policy. Sandbox
socket-bind denials are recorded separately from executable test results.

## Remaining boundaries

A stopped command cannot recover a lost receipt. Durable operation identity,
receipt reconciliation, authenticated ingress, protected operator capacity,
sustained flood fairness and DNS/Raft progress remain open. HTTP status and startup
backoff still use bounded host polling because this interface has no readiness
subscription. Blocking file I/O/stdout and OS timer/scheduling precision remain
outside preemption by the budget.

This extends the existing C18 host transaction qualification without a new root,
pin, registry, custody or recovery family. The earlier x86 publication lookup
failure and cancellation-stress reclamation timeout remain unresolved; no passing
release control attributes or resolves them.
