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
`release-sign` and `shutdown-sign` is now a **file path** rather than hexadecimal
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
Recipient generation keeps serialized private bytes in a zeroizing Rust owner
through file writing, including error returns. Hexadecimal file text reserves
its full length, including the newline, in an erasing owner before copying.
The selected X25519 private and
shared owners also require erasing Drop. See the [bounded memory inventory](../reference/secret-state-lifetimes.md)
for the remaining copy and termination limits.

Each independently generated public root still needs authorized provisioning
at the corresponding verifier. These commands do not provision an OS image.

The reader opens once, rejects symlinks and non-regular inputs on Unix,
checks the opened descriptor's metadata, bounds input to 4096 bytes, and wipes
text/decoded buffers on all exits. Blank lines and whole-line `#` comments
are supported. Ed25519 private files encode 64 bytes; X25519 private files
encode 32. Unix owner/mode checks do not audit ACLs, backup copies, host memory
or a hostile signing workstation. Real private-key file handling on non-Unix
hosts is refused until ACL enforcement exists.

## Plaintext operational profiles

`operations-seal` takes a nonempty regular plaintext file of at most 64 KiB.
On Unix, it rejects symlinks, directories and FIFOs using one opened descriptor.
Its bounded erasing buffer owns plaintext from the first copied byte through
sealing or read failure, including interrupted reads. Keep the source file and
its parent directories under the operator's access policy: the command does
not add private-key-style owner/mode checks for profile inputs. The byte limit
does not provide an I/O deadline or a snapshot against concurrent writes.
See the [profile-input audit](../reports/audits/2026-10-20-security-host-profile-input.md) for tests and remaining host-copy limits.

## Public trust-policy preflight

Prepare an unsigned `CTRUST1` public policy using the four public files above:

```sh
cluster-sign trust-policy-create /secure/signing/cluster.ctrust orders 1 \
  /secure/signing/artifact.pub /secure/signing/deployment.pub \
  /secure/signing/ops.pub /secure/signing/recipient.pub
cluster-sign trust-policy-check /secure/signing/cluster.ctrust orders 1
```

The final check argument is the minimum accepted policy sequence. It is public.
Creation is exclusive: it cannot overwrite an existing policy. Input is bounded
to 4096 bytes per public-key file and exactly 184 bytes for policy checking;
Unix readers reject symlinks, directories and FIFOs. Public files accept hex,
blank lines and whole-line comments. Pass public files only.

The shared `ProductionTrustCandidate` validator requires nonzero policy fields,
the expected cluster and revision floor, four distinct role keys, canonical
prime-order Ed25519 signing points and a canonical, contributory X25519 recipient.
It rejects the shipped development public keys in every role, their signing-key
sign aliases/conversions and conversion of a signing role into the recipient.
It cannot prove that independently supplied private keys are secret, were
generated independently, or have suitable custody.

The commands print a digest for review. The output has no signature. The cluster
and revision floor supplied to `trust-policy-check` are not protected state, and
changing them is not rollback protection. The signed format below binds the
exact public policy and checks revision lineage against explicitly supplied state.
No runtime gate accepts a candidate as authorization, and production builds
continue to refuse. Protected boot, enrollment, protected
revision state and recipient custody are the next prerequisites.

## Signed trust policy

Generate a separate bootstrap signing key, then sign an initial candidate:

```sh
cluster-sign generate /secure/signing/bootstrap.hex /secure/signing/bootstrap.pub
cluster-sign trust-policy-sign /secure/signing/cluster.cbtrust \
  /secure/signing/cluster.ctrust /secure/signing/bootstrap.hex orders enroll 1
cluster-sign trust-policy-verify /secure/signing/cluster.cbtrust \
  /secure/signing/bootstrap.pub orders enroll 1
```

The bootstrap private argument accepts a key-file path only, with the existing
owner/mode/zeroization checks and no legacy argv-secret exception. The bootstrap
key cannot be any development fixture or an admission-role key. Signed output is
created exclusively, after self-verification. A failed check writes no policy.

After authorized installation, retain the printed **accepted-digest** together
with the policy sequence. To sign revision 2 of an initial revision-1 policy,
prepare its new candidate and use the actual accepted digest in place of the
placeholder below. These digests are public and contain no private-key material.

```sh
cluster-sign trust-policy-sign /secure/signing/cluster-v2.cbtrust \
  /secure/signing/cluster-v2.ctrust /secure/signing/bootstrap.hex orders \
  installed 1 ACCEPTED_DIGEST_HEX
cluster-sign trust-policy-verify /secure/signing/cluster-v2.cbtrust \
  /secure/signing/bootstrap.pub orders installed 1 ACCEPTED_DIGEST_HEX
```

`installed` verifies either the exact accepted current record or its immediate
successor with the correct predecessor digest. Signing requires the immediate
successor. Older records, another policy at the accepted revision, skipped
revisions and a wrong predecessor reject. The accepted digest is the
domain-separated signed-fields digest printed by these commands; do not use
the candidate-file hash or complete signed-file hash instead.

The commands require an explicit `enroll` or `installed` mode and never switch
modes after failure. The signed record is bounded to 328 bytes; public-file
readers retain the Unix regular-file/no-follow policy. It contains public policy
and a signature, no private material.

These commands do not install trust or update protected state. CLI arguments and
host files do not establish a protected anchor or rollback protection. A future
installer must pin the key/cluster, persist acceptance state and publish authority
atomically; concurrent reuse of an old expectation is unsafe. Firmware/kernel
authentication and recipient-key custody are still missing, so production
builds remain disabled. See the [wire contract and integration gates](../reference/bootstrap-trust-policy.md).

Boot adapters use the separate `signed_policy::verify_for_boot` API and its
`VerifiedBootPolicy` receipt. It requires the exact independently supplied
installed revision/digest and rejects enrollment or an uninstalled successor.
The CLI's `trust-policy-verify` remains enrollment/installation verification;
its success does not produce boot authority or commit protected state.

## Build-script migration

Replace the retired secret-valued environment variable with a path:

```sh
unset CLUSTER_SIGN_PRIVATE_KEY
export CLUSTER_SIGN_KEY_FILE=/secure/signing/artifact.hex
```

Bundled platform services must match the artifact authority selected before
platform loading. Normal development boot selects the compiled development
policy, so leave this variable unset for that demonstration build; it reads
`tools/cluster-sign/dev-key.hex` directly. The Secure Boot fixture signs an
isolated bundle under its policy artifact key. Setting a host signing path does
not itself select a kernel policy or provide production enrollment. Never extract a real key into
a shell variable, `$(...)`, an argument or a CI log. The scripts refuse the
old variable without expanding its value, including under shell tracing.

Every signing command requires a key-file path, including public development
fixtures. Raw key hex in argv rejects without echoing its contents. The exact
public development fixture files are exempt from restrictive permission checks, by contents rather
than filename, since their bytes are intentionally public.

Broker packaging and Durga-generated command templates in sibling repositories
still need migration to file-path arguments. Both their public-fixture demos
and custom real-key callers must supply paths before using this signer.

See the [current security remediation criteria](../reference/security-remediation.md)
for remaining production-trust and credential-custody work, and the
[historical remediation ledger](../reports/audits/2026-10-03-security-remediation.md)
for earlier evidence.
