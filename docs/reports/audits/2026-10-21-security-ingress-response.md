# Security audit: complete ingress response readers

Date: 2026-10-21. Reviewed baseline: `b779c8d7`.

All signer ingress consumers now share complete-body extraction. Scalar
notification commands validate the current positive-generation acknowledgement;
policy status validates its complete current record before output. The
[tracked evidence](evidence/2026-10-21-security-ingress-response.json) preserves
baseline false successes, corrected results and source/artifact hashes. Findings
remain **29 corrected / 6 partial / 3 open**; production trust stays disabled.

## Findings and correction

Operations-bundle, shutdown and ingress-policy notification previously treated
HTTP 202 as success without validating the generation body. Operations/shutdown
output also discarded data after a second body separator. Deployment and release
already used typed acknowledgement codecs, but broad Unicode trimming could
remove invalid whitespace before those codecs. Policy status printed any HTTP
200 body, including incomplete, duplicate or inconsistent records.

The shared `http_client::response_body` retains the whole declared body after
the first header/body separator and trims only JSON space, tab, CR and LF. Every
consumer uses it, including the previously corrected deployment-status reader.
A second separator, trailing data and non-JSON whitespace remain visible to the
record codec. The bounded HTTP framing decoder is unchanged.

All four scalar notification commands require exactly one positive generation
within the signed result range. Operations, shutdown and ingress-policy now use
the same existing codec as deployment. Release retains its complete digest/count
bound receipt validation. Invalid success records report an uncertain outcome
before success output and never trigger POST replay; the submitted request may
already be committed. Existing successful output stays the same.

The policy-observation codec requires all five current fields exactly once:
positive signed-range generation, positive u64 sequence, 0..=16 assignments,
positive not-before time and an expiry at or after it within the signed policy's
maximum validity span. Zero assignments is the explicit clear policy. Unknown,
missing, nested, duplicate, extended or invalid fields reject. JSON whitespace
and field order are accepted. This is a structural check: observations may be
expired or future-dated, and validation supplies no local UTC admission or peer
authentication authority. See [tooling inputs](../../reference/tooling-inputs.md).

Response strings and sockets remain in their existing host scopes, with their
original deadline and prepared request owners. Ordinary rejection closes those
scopes without remote rollback. The [C18 row](../../reference/cleanup-recovery.md)
records the boundary. No worker, owner family, registry, Charlotte capability/root,
physical completion proof, custody or retry policy is added. Server output,
HTTP grammar, signed input/receipt formats, CLI arguments and runtime ABI are
unchanged.

## Evidence and validation

The new [CLI matrix](../../../scripts/tests/test-ingress-response.py) creates
synthetic signed deployment, release, shutdown, clear-policy and operational
bundle inputs in owned private fixture directories. Generated signing keys and
version-controlled public development operations/recipient fixtures have no
production role. The loopback peer receives each complete payload once and
records method, path and length. No real credential is used or recorded.

Six methods cover **50 cases**. The baseline optimized binary matches the prior
named-status report's artifact hash. It produces **34 expected failed controls /
16 passes**: 24 invalid scalar acknowledgements accepted by the three previously
untyped notification commands, one Unicode acknowledgement accepted by deployment,
one Unicode receipt accepted by release, and eight invalid policy observations.
Existing deployment rejection controls and valid records pass. Each case's body,
exit status, output and request sequence are retained separately in the evidence.

Both corrected profiles pass all 50 response cases. Invalid notification success
is uncertain, emits no success output and submits one POST; malformed policy
observation emits no stdout and performs one GET. Valid compact/whitespace records,
explicit clear policies and HTTP rejection retain their intended behavior.

Validation passes **84 Rust tests and 116 CLI cases per profile**. The CLI total
includes the 50 response cases, 27 named-status, 15 descriptor-generation,
15 release-generation and nine deadline cases. Three new Rust controls cover
full-body preservation and valid/invalid policy bounds without UTC admission.
Strict Clippy, formatting, signed AArch64 build and bundle freshness pass. An
initial development compile failed on an escaped test string after formatting;
a raw-string `concat!` fixture corrects it. That failure is separate from the
baseline reader defects and preserved as a diagnostic in the evidence.

All **41 AArch64 / 37 x86_64 staged images** and **180 formal inputs** retain
their preceding hashes. The tracked prior manifests and their hashes identify
those unchanged inputs. No kernel/service/model behavior changes and no fresh
guest, x86 build or TLC execution are claimed. Historical reports and the manual
PDF are untouched.

## Remaining limits

Record structure and request correlation cannot authenticate an HTTP peer.
Scalar generations do not cryptographically bind a response to the submitted
operation. Readiness still polls, and named status observes current state rather
than a fixed historical admission. Blocking host file I/O/stdout and OS scheduling
can delay return. Server-side progress/teardown, coordinated rollback, production
trust, SEC-15 and existing audit findings remain separate. These controls supply
no kernel resource-quiescence or physical cleanup proof.
