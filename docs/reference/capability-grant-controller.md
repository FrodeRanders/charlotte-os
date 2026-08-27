# Capability-grant controller

`grantctl` is the node-local mediation service between a deployed application
and CharlotteOS service discovery. It exists so applications can use several
Kafka, S3, or ordinary service capabilities without receiving ambient naming
authority or connector credentials.

## Launch contract

A scoped application receives exactly two relevant launch objects:

- its bootstrap connection, which targets `grantctl` with `CALL` rights;
- a read-only `Profile` capability containing its signed `CDEPLOY1`
  descriptor.

Only `grantctl` receives the separately typed `NameService` initial
capability. An ordinary application should call
`catten_services::grant_client::acquire`, passing its borrowed
`Context::profile_memory()` and the requested service name. The helper owns the
request memory and returned connection, so submission failures and early
returns cannot leak capabilities.

## Checks on every acquisition

The controller fails closed unless all of these hold:

1. The descriptor has a valid signature under the cluster public key.
2. The descriptor artifact name derives to the principal in the
   kernel-authenticated IPC sender envelope.
3. The descriptor revision is not older than one already observed for that
   principal; equal revisions must have the same descriptor digest.
4. An exact service grant contains all requested `SEND`/`CALL` rights.
5. The service is currently registered and its publication ceiling contains
   those rights.

The name service returns `MINT_CONNECTION` only to the authenticated
policy-administrator controller. `ReplyToken::reply_connection_ref` then mints
an attenuated application connection. The temporary controller connection is
closed by `Drop` after the reply.

## Trust boundary

The descriptor selects a logical S3 object key or a logical Kafka capability
name, not an IP address, username, password, certificate, bucket, broker, or
topic credential. Platform connector profiles retain those values. Thus an
application can acquire multiple independently named Kafka endpoints while
remaining unable to inspect or reuse the underlying connector identity.
