# Signing and development trust

Current CharlotteOS images use publicly known development fixtures. Build
output and the kernel boot log identify that fact. Do not place real Kafka,
S3 or other credentials under the development recipient key, even if their
envelope is signed and encrypted.

`CATTEN_TRUST_MODE` accepts `development` (the current default) or `production`.
Production mode deliberately refuses both scripted and direct kernel builds.
Protected boot, bootstrap-root provisioning and privileged recipient-key
custody are not yet implemented. Unknown or empty mode values also fail.
Changing a signing file is not a way to make a production image or change its
bootstrap public key. Scoped application launch can use independently supplied
artifact/deployment roots; that does not replace platform bootstrap trust.

## Private keys stay in files

The private-key positional argument to `elf-sign`, `deployment-sign`,
`release-sign` and `shutdown-sign` is now a **file path**, not hexadecimal
bytes. Operational signing and decryption commands also enforce the private
file policy. Public verification-key hex remains permitted: it is not secret.

Use a directory controlled by the signing operator, outside the repository,
build artifacts and shared temporary directories. On Unix, the private file
must be owned by the effective user with no group/other permissions (normally
0600 or 0400). Keep parent directories inaccessible to untrusted users and
apply the organization's backup, rotation and access controls.

Generate independent artifact, deployment and operational signing keys, and
a distinct X25519 recipient key. For example, in an already secured directory:

```sh
cluster-sign generate /secure/signing/artifact.hex /secure/signing/artifact.pub
cluster-sign generate /secure/signing/deployment.hex /secure/signing/deployment.pub
cluster-sign operations-signing-generate /secure/signing/ops.hex /secure/signing/ops.pub
cluster-sign operations-recipient-generate /secure/signing/recipient.hex /secure/signing/recipient.pub
cluster-sign elf-sign app.elf app /secure/signing/artifact.hex service 1 1 0 -
```

Generation uses exclusive creation and never replaces an existing key. Private
bytes are not printed; only output paths and public identity are reported.
Each independently generated public root still needs authorized provisioning
at the corresponding verifier. These commands do not provision an OS image.

The reader opens once, rejects symlinks and non-regular inputs on Unix,
checks the opened descriptor's metadata, bounds input to 4096 bytes, and wipes
text/decoded buffers on all exits. Blank lines and whole-line `#` comments
are supported. Ed25519 private files encode 64 bytes; X25519 private files
encode 32. Unix owner/mode checks do not audit ACLs, backup copies, host memory
or a hostile signing workstation. Real private-key file handling on non-Unix
hosts is refused until ACL enforcement exists.

## Build-script migration

Replace the retired secret-valued environment variable with a path:

```sh
unset CLUSTER_SIGN_PRIVATE_KEY
export CLUSTER_SIGN_KEY_FILE=/secure/signing/artifact.hex
```

Bundled platform services must still match the current compiled bootstrap
root. Normally leave this variable unset for the demonstration build, which
reads `tools/cluster-sign/dev-key.hex` directly. Never extract a real key into
a shell variable, `$(...)`, an argument or a CI log. The scripts refuse the
old variable without expanding its value, including under shell tracing.

Old out-of-tree demo scripts may still pass the exact public artifact fixture
as hex; the signer warns and accepts that fixture only. Other raw hex keys
are rejected with a migration error without echoing their contents. This
exception is not a safe way to pass a secret: a key already supplied in argv
has already been exposed by its caller. The exact public development fixture
files are also exempt from restrictive permission checks, by contents rather
than filename, since their bytes are intentionally public.

Broker packaging and Durga-generated command templates in sibling repositories
still need migration to file-path arguments. Their public-fixture demos remain
compatible; custom real-key callers must migrate before using this signer.

See the [security remediation ledger](../reports/audits/2026-10-03-security-remediation.md)
for remaining production-trust and credential-custody work.
