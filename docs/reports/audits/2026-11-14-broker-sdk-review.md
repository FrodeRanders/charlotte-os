# Broker as an application SDK consumer

14 November 2026. Reviewed `charlotte-kafka-broker` revision
`09798d276cc2ce8617cbc716aac487e9e56ec866`, with a clean working tree. Its
platform pins are CharlotteOS `9f3d5203e4604c3113afb07e25b28ef6cd0db48c` and
Sitas `d27f7e7b7a6e3657c9b39e53da02cf0cd8f91a78`; Rust is
`nightly-2026-07-27`. The OS application foundation being evaluated is based on
`327a50a74127229ec9d1dc6e41ac6c5aaebd7de7` plus uncommitted SDK changes.

The broker is an appropriate application acceptance case. Its pure core, wire,
engine and shard-runtime layers already support an independent host loop.
Build/sign tooling consumes platform inputs without staging the broker into an
OS bundle. The EL0 adapter still uses the older `catten-services` client surface
and local platform scripts; it has not adopted `charlotte-app` or the new CLI.
The foundation needs the concrete extensions below before this concurrent TCP
server becomes a complete example of the new SDK.

## Findings and implementation boundaries

### Accepted socket handoff requires an additional owner

`broker-el0/src/main.rs:275` deliberately leaks its TCP/IP `Connection` to obtain
a process-lifetime borrow. The listening socket later becomes the accepted
session and is moved into a `'static` Sitas closure at lines 398–403. The current
SDK's `OwnedSocket<'connection>` stores a `ConnectionRef<'connection>` inside
its private `SocketLease`, retaining the correct service incarnation.

Capturing `Arc<Connection>` does not by itself make that borrowed socket
transferable. This compile-only check against the new application library fails:

```rust
use std::sync::Arc;
use charlotte_app::{owned::Connection, socket::{self, OwnedSocket}};
pub fn handoff(tcp: Arc<Connection>) {
    let socket = OwnedSocket::open(tcp.as_ref().as_ref(), socket::DOMAIN_TCP).unwrap();
    let _worker: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        drop(socket);
        drop(tcp);
    });
}
```

The diagnostics are `E0597: tcp does not live long enough` at socket creation
and `E0505: cannot move out of tcp because it is borrowed` at closure creation.
The check invokes no native syscall. It qualifies an API limitation, not a
runtime fault. The earlier architecture suggestion to capture an `Arc` needs
this qualification.

The SDK should provide a socket owner that carries its shared connection
through opening, accepted-session handoff, use and consuming close, or sound
scoped Sitas workers that retain the borrow until confirmed exit. Keep the exact
service incarnation and remote selector together; do not move a bare socket ID,
reconstruct cleanup, extend a lifetime unsafely or adopt a capability twice.

### Worker custody and shutdown are incomplete

Both `broker-el0/src/main.rs:398` and
`broker-runtime/src/partition.rs:54` discard the returned Sitas join handle.
Partition shutdown (`broker-runtime/src/broker.rs:514`) waits for messages, not
native thread exit. It also marks `stopped` before fallible channel creation,
submission and acknowledgement; a failed shutdown is subsequently treated as
already stopped. The entry thread completes its shutdown request after its
accept scope returns, without joining session or partition workers.

At the pinned Sitas revision, `CharlotteJoinHandle` contains a raw exit-observer
completion and has no `Drop` implementation; only consuming `join` closes it.
Discarding the handle therefore leaves that observer unclosed until domain
teardown. This is source evidence, not a measured exhaustion campaign. The
pinned `spawn_shard` interface also returns a handle directly, preventing normal
fallible worker-admission handling at the application seam.

The new native `Workers` group cannot adopt Sitas handles. A Sitas owner must
retain its actual join/result state and original generation; a TID cannot
reconstruct it. Add bounded/fallible spawning and owner-preserving joins at the
runtime boundary, then make shutdown stop admission, signal workers, drain
accepted work and confirm joins before acknowledgement. The application owns
the Kafka-specific stop order; the SDK/Sitas layer should own generic custody.

### Publication and waits need application-level ordering

Readiness publication occurs at `broker-el0/src/main.rs:251`, before socket
creation and the first successful listen in `accept_loop`. Deployment can
observe readiness while listen is still failing. Publish after initial listener
setup, retaining the endpoint and its single registration attempt. `Application`
reduces grant/descriptor plumbing but cannot infer when the TCP service is ready.

The adapter also repeats setup and polling mechanics: a private `listen` helper
allocates/maps/writes/unmaps a port page even though `OwnedSocket::listen` and
`listen_call` already exist in its pinned platform. Accept polls every 25 ms,
and socket setup/reception/sending use separate waits and retry counts. Portable
mailbox submission and reply observation each receive a fresh retry-count budget
(`broker-runtime/src/broker.rs:603–659`); unrelated wakes can consume iterations
without the intended wall-clock interval elapsing.

Use the existing typed listen operation and define the accept/read/write
operation's owner, original monotonic deadline and lifecycle notification
contract. The existing deferred `accept_call` provides a producer notification
path to integrate with the reactor. Preserve accepted work on local expiry and
keep remote mutation uncertainty explicit. Kernel IPC admission helpers do not
automatically supply a deadline contract for Sitas mailbox operations.

### Existing-workspace SDK adoption needs tooling support

The broker has six crates, binary `broker-el0`, artifact name `broker`, and a
multi-package host suite. `scripts/app-sdk.py` currently hardcodes the generated
starter's `app/Cargo.toml`, a binary equal to the artifact name and a
`<name>-logic` test package. It cannot directly represent this existing layout.
The lower-level exported build wrapper does support its manifest and binary.

Add explicit manifest/binary/test-package selection and an existing-project
adoption path while retaining source/fingerprint checks. Exported platform
crates and Git dependencies must resolve to one selected platform/Sitas source
graph; mixing identical crate names from Git and path sources can create
incompatible types. The host conformance suite uses `charlotte-kafka` and
`sitas-unix`, which are not currently exported. Their deliberate Git pins may
remain, or an optional test source profile can include them with the same graph.

The broker's older `charlotte.lock` and Cargo dependencies intentionally agree.
A pin bump alone is insufficient: its `charlotte-sdk.sh fetch` sparse-checks out
only service tooling, launch, signer and scripts, while the new exporter also
needs runtime/application source and a pinned Sitas source checkout. Consume a
prebuilt SDK or update that fetch closure. Commit the platform foundation before
adopting it by an immutable revision; change consumer pins deliberately in their
own change, as the broker's contributor contract requires.

## Qualification

All checks used an isolated archive of the broker's committed source; the
broker checkout and its pins remain unchanged. `cargo test --locked --offline`
passed 56 Rust tests, including all 11 real-client codec conformance tests.
`python3 -m unittest discover -s tools/tests` passed three platform tests.

The first sandboxed Cargo run failed all 11 conformance cases at TCP listener
creation with `PermissionDenied: Operation not permitted`, before protocol
execution. The separately authorized repeat permitted local TCP binds and
passed. This environment failure is separate from the passing protocol result.

A newly exported SDK's lower-level builder compiled the unchanged broker with
its original Git-pinned crates for both architectures. Both stripped ELFs
passed layout validation: three LOAD segments, entry `0x20000`, AArch64 283080
bytes and x86-64 274888 bytes. These builds exercise the SDK's build tools;
they do not establish migration to the new application library or CLI.

Relevant SHA-256 identities:

| Input or artifact | Digest |
| --- | --- |
| Broker Cargo.lock | `08549636d966ceb971a0ac47153a7c16d2b3c43cde331acb52915e119a2673c1` |
| SDK sdk.json fingerprint, dirty OS snapshot | `5b0ee4cc0d3a43db5399fb606230f158aebb6f2b9da4a2672dc87c20eef19650` |
| Unsigned AArch64 broker ELF | `0b4eadd852b85953930c3fa7b1419e1d25529c16da1fbd910e0237d2413ed5f8` |
| Unsigned x86-64 broker ELF | `97c78e5a172b2f702d7393a10937a2c3a163128a6e9ac4c56116d3c06e29dc72` |

No broker artifact was signed, uploaded or deployed during this review. Native
shutdown, worker exhaustion, wake delivery and network operation still require
their own deployment qualification after migration. Temporary logs are not
durable evidence dependencies; the relevant source identities, outcomes and
compiler diagnostics are recorded here.

The next finite batch is shared socket custody, Sitas worker admission/joins and
existing-workspace tooling, followed by a broker-only adoption change and native
startup/shutdown checks. Kafka feature expansion is outside this ergonomics work.
