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

- IPC delivery identifies the immediate sender with a kernel-supplied address
  space ID.
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
service restart creates a new execution instance. Policy should use a stable
`PrincipalId` assigned by the trusted launcher or supervisor and resolve the
active `(ASID, generation)` to that identity.

An implementation skeleton now exists in the host-testable
`charlotte-authorization` crate. It implements the modeled policy state
machine independently of transport: exact generation-aware domain identity,
separate administrator and service-manager roles, default-deny exact-match
rules, versioned policy replacement, service-generation fencing, rights
attenuation, and subject-bound single-use decisions. Every collection and
service identifier has an explicit configured bound and fails closed at
capacity.

This is not yet production authorization. `ns` deliberately does not call the
engine because its receive envelope supplies an ASID but not the authenticated
address-space generation needed to construct `DomainIdentity`. Wiring it with
an ASID alone would recreate the identity-reuse flaw that the model excludes.
The next runtime step is therefore a kernel/supervisor identity channel, then a
co-located `ns` policy endpoint and capability-minting adapter.

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

### Co-located first implementation

For an initial implementation, `ns` can own a `PolicyStore` alongside its
catalog. A lookup should perform these logical steps as one serialized
operation:

1. Take the kernel-authenticated sender identity from the IPC envelope.
2. Resolve its current address-space generation to a stable principal.
3. Read the active service binding and its generation.
4. Evaluate the current subject/service rule and requested rights.
5. Delegate an attenuated connection bound to that binding.
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

[`CharlotteAuthorization.tla`](tla/CharlotteAuthorization.tla) models the
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

Cryptographic encoding, principal provisioning, audit retention, policy
language parsing, distributed consistency, availability, and hard revocation
are outside this first model. They remain explicit implementation and modeling
work rather than implicit claims.

## Practical implementation sequence

1. Keep `PrincipalId`, `DomainIdentity`, policy versions, service generations,
   and authorization rights as distinct engine types. Do not expose raw ASIDs
   as durable principals. The initial types and state machine are implemented
   in `charlotte-authorization`.
2. Have the launcher/supervisor install the active domain-to-principal binding
   through authority unavailable to ordinary services.
3. Add a policy store and administrator-only mutation endpoint. Start with an
   exact-match allowlist and default deny.
4. Make lookup request explicit rights and perform policy evaluation plus
   attenuated delegation at one linearization point in `ns`.
5. Add denial/issuance audit records without placing secrets in them.
6. Replace bearer-key lookup users with principal rules, then remove or clearly
   quarantine the keyed protocol.
7. Only split the policy service after the grant format, replay defense,
   revision fencing, and failure behavior have tests corresponding to the TLA+
   actions.
8. Model a revocable proxy or lease separately before promising selective hard
   revocation.

The engine's host tests run through `scripts/run-host-tests.sh`; the repository
test split and the rule for extracting target-independent service logic are
documented in [`testing.md`](testing.md).
