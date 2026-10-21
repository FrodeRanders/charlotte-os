# Current tooling inputs

Signing commands accept private-key **file paths only**, including public
development fixtures. They never accept the fixture's hexadecimal bytes in
argv. Files retain bounded no-follow reads, real-key ownership/permission
checks and zeroizing buffers. Known public fixture contents retain their
existing permission exemption; filenames do not confer that exemption.
`CLUSTER_SIGN_KEY_FILE` supplies a path to build/signing scripts; the retired
secret-valued variable still rejects without expanding its contents.
See [signing and trust](../guides/signing-and-trust.md).

Public-key file arguments use the trust-policy reader's 4096-byte bound and
regular-file checks across operational and ingress-policy commands as well.
On Unix, one no-follow/nonblocking open rejects final-component symlinks,
directories and FIFOs before decoding. All these file readers accept the same
hexadecimal text, blank lines and whole-line comments. Public-key files have
no private-key owner/mode gate; caller-selected keys still need authorized trust
provisioning. Commands that define public hex arguments retain that interface.
See the [public-key input report](../reports/audits/2026-10-21-security-public-key-input.md).

The AArch64 runner accepts repeated `--cluster-service [NAME=]IPv4:PORT`
assignments. Named entries use deployment placement and exact-generation
readiness. An entry without a name selects the platform-service policy over
admitted, non-draining members. These are distinct current policies, not old
and new formats. An explicitly empty name rejects.

Builds consume one semicolon-separated `CATTEN_CLUSTER_SERVICES` table, for
example `orders=10.0.2.42:8080;10.0.2.43:80`. An absent/empty table configures
no bootstrap service assignments. The first committed signed ingress policy
replaces this bootstrap authority. `--cluster-service-name` is removed, and
`CATTEN_CLUSTER_VIP`, `CATTEN_CLUSTER_TCP_PORT` and
`CATTEN_CLUSTER_SERVICE_NAME` reject at kernel build time even when empty;
their values are not printed or used to synthesize a table. Runner-local
VIP/port values still select the first assignment's SLIRP host-forwarding
target and are not a second kernel policy source.

The unused `catten_services::socket::MAX_SOCKETS` export is removed. Socket
capacity comes from the privileged TCP/IP launch policy, with separate total
slots and per-principal socket/buffer quotas. The deprecated constant's 16
was never the current stack capacity. No wire opcode, runtime ABI or admission
budget changes in this batch.

The exported application SDK carries the same file-only signer. Sibling broker
and Durga callers previously recorded as passing key bytes must migrate their
call sites to paths; this repository does not maintain their argv exception.
See [S07 evidence](../reports/audits/2026-10-13-protocol-simplification-tooling.md).


Boot runners and the standalone image creator share one 128 MiB disposable
FAT32 default, including the complete debug ELF and EFI loader. The creator's
explicit `--size-mib` still overrides it; insufficient capacity rejects through
atomic staging without replacing prior output. This boot container is separate
from persistent data-store sizing and does not reformat existing data disks.
See the [preserved capacity failure and correction](../reports/audits/2026-10-16-observer-entry-construction.md).

Key generation encodes the complete lowercase-hex record and its trailing
newline in one reserved erasing buffer. The existing file writer borrows that
owner through creation/write; ordinary success or rejection drops the same
owner. The former newline append caused output growth after hex encoding.
Private modes, exclusive creation, file-only readers and successful output
messages are unchanged. Failure retention follows the policy below. Source key
owners, disk/OS buffers and host custody retain their separate policies. See the
[key-file report](../reports/audits/2026-10-20-security-host-key-encoding.md).

`operations-seal` reads plaintext from a nonempty regular file of at most
64 KiB. On Unix it opens once with no-follow and nonblocking flags, then checks
the opened descriptor before reading; symlinks, directories and FIFOs reject.
One fixed erasing destination, including an extra limit-detection byte, retains
partial reads through error or success. Empty/oversized profiles already failed
later envelope validation; they now reject before reading. Input permissions
and ACLs remain the operator's responsibility. This byte bound supplies no I/O
deadline or snapshot against concurrent file mutation. Arguments, envelope bytes
and exclusive output creation are unchanged. See the [profile-input report](../reports/audits/2026-10-20-security-host-profile-input.md).

Operational ciphertext commands share the bounded regular-file reader before
verification, decryption, bundle signing or notification. `COPSENC1` input is
at most 66,044 bytes; `COPSBND2` input is at most 1,048,064 bytes. Bundle signing
also bounds its referenced release to 3,584 bytes and checks its 1..=8 bindings
before opening that release or any referenced envelope. Each envelope uses the
same standalone bound. Unix input opens reject final-component symlinks,
directories and FIFOs. Rejected notification input does not connect to ingress;
notification still relies on the receiver for authenticated admission. These
file checks add no trust authority or I/O deadline. Structured control records
use the bounds below; artifact and arbitrary-file hash inputs use the policies
recorded at the end of this page.
See the [operational-input report](../reports/audits/2026-10-21-security-operational-input.md).

Structured control inputs also use this shared reader before verification,
signing references, notification or apply. Deployment descriptors and release
files are bounded to 3,584 bytes, shutdown files to 144 bytes and ingress
policies to 1,184 bytes. Release signing checks 1..=16 descriptor references
before reading them. Deployment apply still prepares all supplied descriptors
before its first notification; each file has the descriptor bound. It has no
new aggregate batch limit. Unix opens reject final-component symlinks and
non-regular sources. Format/signature checks and receiver authorization remain
in place. Artifact inputs and the `sha256` command use the policies below. See the
[control-input report](../reports/audits/2026-10-21-security-control-input.md).

ELF sign/verify and deployment primary/variant inputs use the shared regular-file
reader with the launch ABI's 4 MiB artifact maximum. Signing checks the projected
note/string/section-table output size before allocation and writing; adding a
note cannot produce an artifact beyond that maximum. Deployment signing admits
at most eight variants, including the primary, before opening images.

`sha256` accepts regular files without the artifact size cap and hashes them to
EOF through one 16 KiB erasing buffer. Empty-file and digest output syntax are
unchanged. On Unix, both artifact and hash inputs reject final-component
symlinks, directories and FIFOs. No production `fs::read` remains in the signer;
whole-image operations retain bounded buffers. Neither streaming nor descriptor
capture provides a content snapshot or I/O deadline. In-place signing retains
its opened descriptor through rewriting as described below; write atomicity
remains separate. See the [artifact-input report](../reports/audits/2026-10-21-security-artifact-input.md).

`elf-sign` keeps one read/write descriptor and its prepared image in `ArtifactEdit`.
Its consuming commit rewinds, writes and sets length through that descriptor;
it never reopens, creates or removes the pathname. A replacement symlink or
regular file is left untouched. If the original is renamed, that original file
is updated; if it is unlinked, signing does not recreate the path. Success
therefore refers to the opened file, without guaranteeing the path's current
binding. The output identifies the artifact as opened at the supplied path.
Dropping an edit without commit leaves its bytes unpublished. A commit error
consumes the owner and may leave a partial rewrite; no implicit retry, rollback,
fsync or atomic/durable publication is promised. See the [artifact-edit report](../reports/audits/2026-10-21-security-artifact-edit.md).

`deployment-sign`, `release-sign` and `shutdown-sign` create new output files
through the shared exclusive writer. Existing regular files, hard links and
final-component symlinks reject before writing; choose a fresh output name for
each signed record. Successful output bytes and verification are unchanged.
The boot runners keep deployment/shutdown records in fresh private run directories,
with deployment worker logs beside their records. Local deployment fixtures keep
each run's six signed parts in a fresh private directory; the assembled fixture
bundle uses the staged replacement described below. These directories remain
available for inspection. Failed writes follow the retention policy below; parent-directory
integrity and durable atomic publication still need separate qualification. See the [control-output report](../reports/audits/2026-10-21-security-control-output.md).

New-file output failures close the captured descriptor and leave any written file
alone. The shared writer performs no pathname deletion; empty or partial output
can remain. All three key generators share one pair writer: a public-output
failure reports an incomplete pair and the private output's original path,
retaining the completed private file instead of unlinking that name. The name
may have changed, so the diagnostic does not establish its current binding.
Private files keep the requested 0600 mode. Inspect the secured output directory
and choose fresh paths before another generation; a failed command supplies no
completed-pair or retry guarantee. In-memory key/text erasure is unchanged.
See the [output-retention report](../reports/audits/2026-10-21-security-output-retention.md).

The local deployment fixture assembler writes its complete bundle to a new
`bundle.pending` file inside that run's private parts directory. It closes the
staging file before `os.replace` publishes it at the requested output name.
Successful replacement changes the named file; a symlink or hard-link target
stays untouched, and readers holding the old file can finish reading it.
A staging-write or close error occurs before replacement and leaves previous
output intact. Failed staging remains available for inspection; occupied staging
rejects without truncation. The assembled six-record bytes and descriptor
verification are unchanged. Publication uses the same filesystem in the normal
helper, but supplies no fsync/durability, namespace-integrity or automatic recovery
guarantee. New-file modes follow the current umask; prior output metadata is not
preserved. See the [fixture-publication report](../reports/audits/2026-10-21-security-fixture-publication.md).

`deployment-notify` requires the current acknowledgement's single positive
`generation`. `deployment-apply` retains each accepted name/generation pair and
compares complete readiness records against it. Older generations wait, matching
ready generations complete, and newer generations fail as superseded. Invalid
records stop without notification replay; a rejected acknowledgement may follow
an already committed mutation. Observation storage reserves before submission and
borrows names from the retained prepared descriptor set. `deployment-status`
remains an observation of the current named record. `release-apply` still lacks
component-generation binding because its acknowledgement supplies only a release
generation. End-to-end deadlines, connect/DNS/write bounds and status polling
remain separate work. See the [client-generation report](../reports/audits/2026-10-21-security-deployment-generation.md).
