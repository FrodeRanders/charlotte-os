# Capability-grant controller

`grantctl` is the node-local mediation service between a deployed application
and CharlotteOS service discovery. It exists so applications can use several
Kafka, S3, or ordinary service capabilities without receiving ambient naming
authority or connector credentials.

## Launch contract

A scoped application receives exactly two relevant launch objects:

- its bootstrap connection, which targets `grantctl` with `CALL` rights;
- a read-only `Profile` capability containing its signed `CDEPLOY5`
  descriptor.

Only `grantctl` receives the separately typed `NameService` initial
capability. An ordinary application should call
`catten_services::grant_client::acquire`, passing its borrowed
`Context::profile_memory()` and the requested service name. The helper owns the
request memory and returned connection, so submission failures and early
returns cannot leak capabilities.

## Checks on every acquisition

The controller fails closed unless all of these hold:

1. The kernel attests that the descriptor digest is exactly the immutable
   policy admitted for this sender's live address-space generation. Admission
   already checked the configured deployment key, artifact key, ELF digest,
   and artifact name before the domain started.
2. The descriptor artifact name derives to the principal in the
   kernel-authenticated IPC sender envelope.
3. The requested generation is still current. A signed descriptor for another
   version of the same logical artifact is not interchangeable with the
   running domain's policy, even if it has a newer sequence or broader grants.
4. An exact service grant contains all requested client (`SEND`/`CALL`) rights,
   or separately authorizes endpoint publication.
5. The service is currently registered and its publication ceiling contains
   those rights.

The name service returns `MINT_CONNECTION` only to the authenticated
policy-administrator controller. `ReplyToken::reply_connection_ref` then mints
an attenuated application connection. The temporary controller connection is
closed by `Drop` after the reply.

`grant_client::publish` is the complementary service-side operation. It moves
no endpoint owner: the controller receives a bounded mintable connection,
checks an exact `publish` grant, and registers only client `SEND`/`CALL` rights
with the name service. A scoped service can therefore become discoverable
without gaining ambient lookup or registration authority.

## Bounded asynchronous admission

The controller keeps each incoming message and name-service `PendingCall` in
one owning operation. It polls outstanding work rather than synchronously
waiting for a missing service. At most 32 operations are outstanding, with at
most four per authenticated sender/generation and 16 new requests admitted per
reactor cycle. A five-second monotonic deadline returns `ERR_UNAVAILABLE` and
drops the operation, cancelling its pending IPC call. The controller uses
`ns::OP_TRY_LOOKUP_FOR_GRANT`, which applies the same authorization as the
deferred operation but never retains a waitlist entry for an absent name.
Cancellation therefore cannot strand grant requests in that shared waitlist.

`grant_client::acquire` retries temporary unavailability at 100 ms intervals
within one five-second total monotonic budget, polling each owned call so
expiry cancels it. Authorization and malformed-request errors are not retried.
Applications may choose a later retry policy after the helper returns.
Shutdown drops every pending controller operation before
acknowledging completion.

`LaunchDescriptorMatches` (syscall 83) returns an attestation only to the exact
kernel-designated grant-controller domain. The controller does not obtain
trust by registering under a particular name, and it does not rebuild policy
from caller-provided principal/revision claims or a hardcoded fixture key.

## Trust boundary

The descriptor selects a logical S3 object key or a logical Kafka capability
name, not an IP address, username, password, certificate, bucket, broker, or
topic credential. Platform connector profiles retain those values. Thus an
application can acquire multiple independently named Kafka endpoints while
remaining unable to inspect or reuse the underlying connector identity.
