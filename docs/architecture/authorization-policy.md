# Authorization policy direction

CharlotteOS should treat authorization as **controlled capability issuance**.
The name catalog answers where a service is and which generation is current.
The policy engine answers whether an authenticated principal may receive a
capability, and with which rights. The kernel then enforces the issued
capability without consulting policy on every IPC operation.

The two roles may initially live in the same `ns` process, but their state and
interfaces should remain distinct. This keeps a later replicated or standalone
policy service possible without making names themselves authoritative.

## What exists today

The kernel already provides useful enforcement foundations:

- The explicit authenticated IPC receive ABI identifies the immediate sender
  with a kernel-supplied exact address-space generation, stable principal, and
  role bits captured when the message is queued. Legacy receive syscalls keep
  their original register contract and omit this metadata.
- Connections carry `SEND`, `CALL`, and `MINT_CONNECTION` rights.
- Delegation can attenuate a connection to `SEND | CALL`.
- The local and distributed name catalogs retain service generations and fence
  stale lifecycle operations.

The local name service also has `OP_REGISTER_KEYED` and `OP_LOOKUP_KEYED`.
That mechanism is an interim bearer-key gate, not a realistic authorization
policy:

- the service publisher chooses one reusable 64-bit secret;
- possession of the value, rather than a principal identity, grants access;
- rules cannot express different rights for different callers;
- policy administration is not a separately authorized operation;
- the rule has no revision, durable history, or useful audit identity;
- key replacement and service replacement are not one explicitly fenced
  authorization transaction; and
- the distributed catalog does not replicate a production authorization rule.

The raw sender ASID is suitable for authenticating the immediate IPC hop, but
not as a durable policy identity. Numeric address spaces may be recycled and a
service restart creates a new execution instance. The signed loader therefore
derives a stable `PrincipalId` from the authenticated artifact name and binds
it to the active `(ASID, generation)` in a kernel-only authority table. Only
artifacts classified as administration workloads receive policy-administrator
and service-manager roles.

An implementation skeleton now exists in the host-testable
`charlotte-authorization` crate. It implements the modeled policy state
machine independently of transport: exact generation-aware domain identity,
separate administrator and service-manager roles, default-deny exact-match
rules, versioned policy replacement, service-generation fencing, rights
attenuation, and subject-bound single-use decisions. Every collection and
service identifier has an explicit configured bound and fails closed at
capacity.

The local `ns` service now hosts this engine. `OP_REGISTER_AUTHORIZED` requires
the kernel-authenticated service-manager role, `OP_SET_POLICY` requires the
policy-administrator role, and `OP_LOOKUP_AUTHORIZED` performs a default-deny
decision followed by an attenuated connection delegation. The caller cannot
supply or override its identity in request bytes. Deferred lookups retain the
same exact `DomainIdentity`, and stale ASID generations are rejected rather
than being rebound.

Authorization decisions and delegation failures enter a bounded FIFO audit
stream containing sequence, exact caller identity, stable principal, service,
requested/granted rights, service generation, and policy version. Audit
sequence exhaustion fails closed before issuance; `OP_AUTH_AUDIT` is restricted
to policy administrators. Policy and audit state remain volatile, and the
distributed catalog does not yet replicate policy. The legacy public and
bearer-key opcodes remain available for compatibility and are explicitly not
an authorization boundary; security-sensitive clients must use the authorized
opcodes.

## Proposed contract

The smallest useful rule is:

```text
allow PrincipalId ServiceId {SEND, CALL}
```

Its authoritative state should contain:

```text
PolicyRule {
    subject: PrincipalId,
    service: ServiceId,
    allowed_rights: Rights,
    version: u64,
}

ServiceBinding {
    service: ServiceId,
    generation: u64,
    maximum_delegable_rights: Rights,
}
```

A lookup requests an explicit rights set. The result is bounded by both the
policy rule and the service binding:

```text
issued_rights <= requested_rights
issued_rights <= policy.allowed_rights
issued_rights <= binding.maximum_delegable_rights
```

Policy mutation requires a separately delegated policy-administrator
capability. Publication and replacement require service-manager authority.
Ordinary discovery clients receive neither capability.

### Co-located implementation

`ns` owns a `PolicyStore` alongside its catalog. An authorized lookup performs
these logical steps as one serialized operation:

1. Take the kernel-authenticated sender generation, principal, and roles from
   the IPC envelope.
2. Synchronize the exact address-space lifetime into the bounded identity map.
3. Read the active service binding and its generation.
4. Evaluate the current subject/service rule and requested rights.
5. Delegate a connection attenuated to the approved rights and bound to that
   service generation.
6. Record the subject, service generation, policy version, rights, and result
   in a bounded audit stream.

This path does not need a transferable token. The TLA+ model separates policy
decision from redemption so that it also covers interleaving and the possible
future split-service design. In a co-located implementation those two actions
are one linearization point.

### Split service later

If policy evaluation later moves into a separate process or replicated state
machine, its decision must be an unforgeable, single-use grant bound to:

- the authenticated subject;
- the service identity and current generation;
- the exact rights approved;
- the policy version;
- a unique nonce or grant ID; and
- an expiry or bounded redemption lifetime if grants can leave one node.

The resolver must revalidate the subject, service generation, and policy
version when redeeming the grant. A policy update or service replacement then
invalidates every unredeemed grant from the older version. Raft agreement can
make policy state consistent, but agreement alone does not authorize the
caller that proposed a mutation.

## Revocation semantics

The first contract should promise **prospective revocation**:

- a changed rule prevents new decisions;
- a changed rule invalidates old unredeemed decisions; and
- a replaced service generation cannot be reached with a decision for the old
  generation.

It must not claim that changing a rule retracts connections already delegated.
Those capabilities remain valid according to the endpoint's normal lifetime.
This is inherent in the current direct capability design, which has no general
derivation tree or policy check on use.

Hard revocation needs a different authority shape. Practical choices are:

- close or replace the endpoint, revoking all of its connections;
- delegate a connection to a revocable proxy/session gate rather than directly
  to the service; or
- add a kernel-supported lease or revocation object checked on use.

The proxy approach is the least invasive place to experiment with selective
revocation, but it adds a data-path dependency and should receive its own TLA+
model before being treated as a safety mechanism.

## TLA+ model

[`CharlotteAuthorization.tla`](../tla/CharlotteAuthorization.tla) models the
target issuance protocol. It includes authenticated policy and publication
roles, versioned subject/service rules, generation-fenced service bindings,
rights attenuation, single-use decisions, capability close, and the fact that
already issued capabilities survive later policy changes.

The safe model checks that:

- only policy administrators mutate rules;
- only service managers publish or replace bindings;
- every decision was within the rule and service ceiling at issue time;
- redemption uses the same principal, current policy version, and current
  service generation;
- issued rights do not exceed the decision or the rights allowed at redemption;
  and
- one decision cannot create two capabilities.

Five negative configurations deliberately omit one fence each. TLC must find
counterexamples for unauthorized policy mutation, cross-principal redemption,
stale policy redemption, stale service-generation redemption, and rights
amplification.

Cryptographic artifact verification and principal provisioning happen below
the model boundary. Durable audit retention, policy-language parsing,
distributed policy consistency, availability, and hard revocation remain
outside this first model and are not implicit claims.

## Remaining implementation sequence

1. Migrate security-sensitive clients from the public/bearer-key compatibility
   opcodes to explicit principal rules and authorized lookup.
2. Persist or replicate policy and audit state with generation/version fencing;
   Raft agreement must not replace caller authentication.
3. Separate administration and service-management assignments when deployment
   policy needs finer privilege than the current administration artifact class.
4. Only split the policy service after the grant format, replay defense,
   revision fencing, and failure behavior have tests corresponding to the TLA+
   actions.
5. Model a revocable proxy or lease separately before promising selective hard
   revocation.

The engine's host tests run through `scripts/run-host-tests.sh`; the repository
test split and the rule for extracting target-independent service logic are
documented in [the testing guide](../guides/testing.md).
