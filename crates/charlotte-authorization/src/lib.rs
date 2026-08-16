#![no_std]

//! Authorization policy state machine for controlled capability issuance.
//!
//! This module contains no transport or capability-minting code. The node name
//! service can host it directly, or a future policy service can expose the same
//! transitions over IPC. Keeping the decision state independent makes the
//! safety boundary testable before it is connected to a privileged endpoint.
//!
//! A transport integrating this store must obtain [`DomainIdentity`] from the
//! kernel, not from request bytes, and must allow
//! [`PolicyStore::provision_identity_from_supervisor`] only on an authenticated
//! supervisor control path.

extern crate alloc;

use alloc::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    vec::Vec,
};
use core::ops::BitOr;

/// Stable workload identity assigned by trusted launch policy.
///
/// This is deliberately distinct from a numeric address-space ID. Two domain
/// instances may share a principal, while reuse of an ASID must not transfer
/// the former occupant's identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PrincipalId(u64);

impl PrincipalId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Exact identity of one occupancy of an address-space slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DomainIdentity {
    asid: u64,
    generation: u64,
}

impl DomainIdentity {
    pub const fn new(asid: u64, generation: u64) -> Option<Self> {
        if asid == 0 || generation == 0 {
            None
        } else {
            Some(Self {
                asid,
                generation,
            })
        }
    }

    pub const fn asid(self) -> u64 {
        self.asid
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Roles assigned only through the supervisor provisioning boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Roles(u8);

impl Roles {
    pub const NONE: Self = Self(0);
    pub const POLICY_ADMIN: Self = Self(1 << 0);
    pub const SERVICE_MANAGER: Self = Self(1 << 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !((Self::POLICY_ADMIN.0 | Self::SERVICE_MANAGER.0) as u32) == 0 {
            Some(Self(bits as u8))
        } else {
            None
        }
    }
}

impl BitOr for Roles {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Rights the policy service may approve for a client connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationRights(u32);

impl AuthorizationRights {
    pub const CALL: Self = Self(1 << 1);
    pub const CLIENT: Self = Self(Self::SEND.0 | Self::CALL.0);
    pub const NONE: Self = Self(0);
    pub const SEND: Self = Self(1 << 0);

    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::CLIENT.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl BitOr for AuthorizationRights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IdentityRecord {
    principal: PrincipalId,
    roles: Roles,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyRule {
    pub allowed: AuthorizationRights,
    pub version: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceBinding {
    pub generation: u64,
    pub ceiling: AuthorizationRights,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TicketId(u64);

impl TicketId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Decision {
    principal: PrincipalId,
    service: Vec<u8>,
    service_generation: u64,
    policy_version: u64,
    rights: AuthorizationRights,
}

/// The authority snapshot a resolver may turn into an attenuated connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizedGrant {
    pub principal: PrincipalId,
    pub service_generation: u64,
    pub policy_version: u64,
    pub rights: AuthorizationRights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationError {
    AuditSequenceExhausted,
    InvalidService,
    InvalidLimits,
    InvalidAuditCapacity,
    IdentityCapacity,
    IdentityConflict,
    PolicyCapacity,
    ServiceCapacity,
    TicketCapacity,
    UnknownIdentity,
    PolicyAdministratorRequired,
    ServiceManagerRequired,
    PolicyMissing,
    ServiceMissing,
    ServiceInactive,
    EmptyRights,
    RightsDenied,
    PolicyVersionConflict,
    PolicyVersionExhausted,
    ServiceGenerationExhausted,
    TicketIdExhausted,
    TicketMissing,
    PrincipalMismatch,
    StalePolicy,
    StaleIdentity,
    StaleServiceGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    Issued,
    Denied(AuthorizationError),
    DelegationFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    pub sequence: u64,
    pub caller: DomainIdentity,
    pub principal: Option<PrincipalId>,
    pub service: Vec<u8>,
    pub requested: AuthorizationRights,
    pub granted: AuthorizationRights,
    pub service_generation: u64,
    pub policy_version: u64,
    pub outcome: AuditOutcome,
}

/// Bounded authorization audit stream. Old records are evicted in FIFO order;
/// sequence exhaustion fails closed before a new capability is issued.
pub struct AuditLog {
    capacity: usize,
    next_sequence: u64,
    records: VecDeque<AuditRecord>,
}

impl AuditLog {
    pub fn new(capacity: usize) -> Result<Self, AuthorizationError> {
        if capacity == 0 {
            return Err(AuthorizationError::InvalidAuditCapacity);
        }
        Ok(Self {
            capacity,
            next_sequence: 1,
            records: VecDeque::new(),
        })
    }

    pub fn can_record(&self) -> bool {
        self.next_sequence != u64::MAX
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        caller: DomainIdentity,
        principal: Option<PrincipalId>,
        service: &[u8],
        requested: AuthorizationRights,
        granted: AuthorizationRights,
        service_generation: u64,
        policy_version: u64,
        outcome: AuditOutcome,
    ) -> Result<u64, AuthorizationError> {
        if !self.can_record() {
            return Err(AuthorizationError::AuditSequenceExhausted);
        }
        let sequence = self.next_sequence;
        self.next_sequence = sequence + 1;
        if self.records.len() == self.capacity {
            let _ = self.records.pop_front();
        }
        self.records.push_back(AuditRecord {
            sequence,
            caller,
            principal,
            service: service.to_vec(),
            requested,
            granted,
            service_generation,
            policy_version,
            outcome,
        });
        Ok(sequence)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &AuditRecord> {
        self.records.iter()
    }
}

/// Explicit memory bounds for a policy store hosted by an EL0 service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyLimits {
    pub identities: usize,
    pub policies: usize,
    pub services: usize,
    pub tickets: usize,
    pub service_id_bytes: usize,
}

impl PolicyLimits {
    pub const DEFAULT: Self = Self {
        identities: 1_024,
        policies: 4_096,
        services: 1_024,
        tickets: 1_024,
        service_id_bytes: 256,
    };

    const fn valid(self) -> bool {
        self.identities > 0
            && self.policies > 0
            && self.services > 0
            && self.tickets > 0
            && self.service_id_bytes > 0
    }
}

/// In-memory authorization state.
///
/// Policy and service records retain their monotonically increasing versions
/// across deny rules and unpublication. They fail closed on integer exhaustion.
pub struct PolicyStore {
    limits: PolicyLimits,
    identities: BTreeMap<DomainIdentity, IdentityRecord>,
    policies: BTreeMap<(PrincipalId, Vec<u8>), PolicyRule>,
    services: BTreeMap<Vec<u8>, ServiceBinding>,
    tickets: BTreeMap<TicketId, Decision>,
    next_ticket: u64,
}

impl PolicyStore {
    pub const fn new() -> Self {
        Self {
            limits: PolicyLimits::DEFAULT,
            identities: BTreeMap::new(),
            policies: BTreeMap::new(),
            services: BTreeMap::new(),
            tickets: BTreeMap::new(),
            next_ticket: 1,
        }
    }

    pub const fn with_limits(limits: PolicyLimits) -> Result<Self, AuthorizationError> {
        if !limits.valid() {
            return Err(AuthorizationError::InvalidLimits);
        }
        Ok(Self {
            limits,
            identities: BTreeMap::new(),
            policies: BTreeMap::new(),
            services: BTreeMap::new(),
            tickets: BTreeMap::new(),
            next_ticket: 1,
        })
    }

    /// Install one exact domain-to-principal binding.
    ///
    /// The caller must authenticate the supervisor before invoking this
    /// transition. Installing a new generation for an ASID removes every old
    /// occupancy of that slot, so delayed messages cannot inherit identity.
    pub fn provision_identity_from_supervisor(
        &mut self,
        identity: DomainIdentity,
        principal: PrincipalId,
        roles: Roles,
    ) -> Result<(), AuthorizationError> {
        if let Some(existing) = self.identities.get(&identity) {
            if existing.principal != principal || existing.roles != roles {
                return Err(AuthorizationError::IdentityConflict);
            }
            return Ok(());
        }
        if self
            .identities
            .keys()
            .any(|known| known.asid == identity.asid && known.generation > identity.generation)
        {
            return Err(AuthorizationError::StaleIdentity);
        }
        let replaces_asid = self.identities.keys().any(|known| known.asid == identity.asid);
        if !replaces_asid && self.identities.len() >= self.limits.identities {
            return Err(AuthorizationError::IdentityCapacity);
        }
        self.identities.retain(|known, _| known.asid != identity.asid);
        self.identities.insert(
            identity,
            IdentityRecord {
                principal,
                roles,
            },
        );
        Ok(())
    }

    /// Remove a domain binding only when both ASID and generation match.
    pub fn remove_identity_from_supervisor(&mut self, identity: DomainIdentity) -> bool {
        self.identities.remove(&identity).is_some()
    }

    pub fn principal_for(&self, identity: DomainIdentity) -> Option<PrincipalId> {
        self.identities.get(&identity).map(|record| record.principal)
    }

    pub fn roles_for(&self, identity: DomainIdentity) -> Option<Roles> {
        self.identities.get(&identity).map(|record| record.roles)
    }

    /// Publish or replace a service binding and allocate a new generation.
    pub fn publish_service(
        &mut self,
        actor: DomainIdentity,
        service: &[u8],
        ceiling: AuthorizationRights,
    ) -> Result<ServiceBinding, AuthorizationError> {
        self.require_role(actor, Roles::SERVICE_MANAGER)?;
        self.validate_service(service)?;
        if ceiling.is_empty() {
            return Err(AuthorizationError::EmptyRights);
        }
        if !self.services.contains_key(service) && self.services.len() >= self.limits.services {
            return Err(AuthorizationError::ServiceCapacity);
        }
        let generation = match self.services.get(service) {
            Some(binding) => binding
                .generation
                .checked_add(1)
                .ok_or(AuthorizationError::ServiceGenerationExhausted)?,
            None => 1,
        };
        let binding = ServiceBinding {
            generation,
            ceiling,
            active: true,
        };
        self.services.insert(service.to_vec(), binding);
        Ok(binding)
    }

    /// Retain the generation tombstone while making a service unresolvable.
    pub fn unpublish_service(
        &mut self,
        actor: DomainIdentity,
        service: &[u8],
        expected_generation: u64,
    ) -> Result<(), AuthorizationError> {
        self.require_role(actor, Roles::SERVICE_MANAGER)?;
        let binding = self.services.get_mut(service).ok_or(AuthorizationError::ServiceMissing)?;
        if !binding.active {
            return Err(AuthorizationError::ServiceInactive);
        }
        if binding.generation != expected_generation {
            return Err(AuthorizationError::StaleServiceGeneration);
        }
        binding.active = false;
        binding.ceiling = AuthorizationRights::NONE;
        Ok(())
    }

    /// Replace one complete subject/service rule using optimistic versioning.
    ///
    /// `expected_version` is zero when creating a rule. An empty allowed-rights
    /// set is an explicit versioned deny, not deletion of the tombstone.
    pub fn set_policy(
        &mut self,
        actor: DomainIdentity,
        subject: PrincipalId,
        service: &[u8],
        allowed: AuthorizationRights,
        expected_version: u64,
    ) -> Result<PolicyRule, AuthorizationError> {
        self.require_role(actor, Roles::POLICY_ADMIN)?;
        self.validate_service(service)?;
        let key = (subject, service.to_vec());
        if !self.policies.contains_key(&key) && self.policies.len() >= self.limits.policies {
            return Err(AuthorizationError::PolicyCapacity);
        }
        let current = self.policies.get(&key).map_or(0, |rule| rule.version);
        if current != expected_version {
            return Err(AuthorizationError::PolicyVersionConflict);
        }
        let version = current.checked_add(1).ok_or(AuthorizationError::PolicyVersionExhausted)?;
        let rule = PolicyRule {
            allowed,
            version,
        };
        self.policies.insert(key, rule);
        Ok(rule)
    }

    /// Create a decision bound to the current subject, policy, and service.
    pub fn issue_ticket(
        &mut self,
        caller: DomainIdentity,
        service: &[u8],
        requested: AuthorizationRights,
    ) -> Result<TicketId, AuthorizationError> {
        if requested.is_empty() {
            return Err(AuthorizationError::EmptyRights);
        }
        self.validate_service(service)?;
        let principal = self.principal_for(caller).ok_or(AuthorizationError::UnknownIdentity)?;
        let binding = self.active_binding(service)?;
        let rule = self
            .policies
            .get(&(principal, service.to_vec()))
            .copied()
            .ok_or(AuthorizationError::PolicyMissing)?;
        let allowed = rule.allowed.intersection(binding.ceiling);
        if !allowed.contains(requested) {
            return Err(AuthorizationError::RightsDenied);
        }
        if self.tickets.len() >= self.limits.tickets {
            return Err(AuthorizationError::TicketCapacity);
        }
        let raw_ticket = self.next_ticket;
        self.next_ticket =
            raw_ticket.checked_add(1).ok_or(AuthorizationError::TicketIdExhausted)?;
        let ticket = TicketId(raw_ticket);
        self.tickets.insert(
            ticket,
            Decision {
                principal,
                service: service.to_vec(),
                service_generation: binding.generation,
                policy_version: rule.version,
                rights: requested,
            },
        );
        Ok(ticket)
    }

    /// Redeem a ticket exactly once, revalidating every authority binding.
    ///
    /// Removal precedes validation, so even a stale or misdirected ticket is
    /// consumed and cannot be retried after state changes.
    pub fn redeem_ticket(
        &mut self,
        caller: DomainIdentity,
        ticket: TicketId,
    ) -> Result<AuthorizedGrant, AuthorizationError> {
        let decision = self.tickets.remove(&ticket).ok_or(AuthorizationError::TicketMissing)?;
        let principal = self.principal_for(caller).ok_or(AuthorizationError::UnknownIdentity)?;
        if principal != decision.principal {
            return Err(AuthorizationError::PrincipalMismatch);
        }
        let binding = self.active_binding(&decision.service)?;
        if binding.generation != decision.service_generation {
            return Err(AuthorizationError::StaleServiceGeneration);
        }
        let rule = self
            .policies
            .get(&(principal, decision.service.clone()))
            .copied()
            .ok_or(AuthorizationError::PolicyMissing)?;
        if rule.version != decision.policy_version {
            return Err(AuthorizationError::StalePolicy);
        }
        if !rule.allowed.intersection(binding.ceiling).contains(decision.rights) {
            return Err(AuthorizationError::RightsDenied);
        }
        Ok(AuthorizedGrant {
            principal,
            service_generation: binding.generation,
            policy_version: rule.version,
            rights: decision.rights,
        })
    }

    pub fn cancel_ticket(
        &mut self,
        caller: DomainIdentity,
        ticket: TicketId,
    ) -> Result<(), AuthorizationError> {
        let principal = self.principal_for(caller).ok_or(AuthorizationError::UnknownIdentity)?;
        let decision = self.tickets.get(&ticket).ok_or(AuthorizationError::TicketMissing)?;
        if decision.principal != principal {
            return Err(AuthorizationError::PrincipalMismatch);
        }
        self.tickets.remove(&ticket);
        Ok(())
    }

    /// Co-located name/policy-service path. No ticket becomes externally
    /// visible, but the same issue and redemption checks are exercised.
    pub fn authorize_now(
        &mut self,
        caller: DomainIdentity,
        service: &[u8],
        requested: AuthorizationRights,
    ) -> Result<AuthorizedGrant, AuthorizationError> {
        let ticket = self.issue_ticket(caller, service, requested)?;
        self.redeem_ticket(caller, ticket)
    }

    pub fn policy(&self, subject: PrincipalId, service: &[u8]) -> Option<PolicyRule> {
        self.policies.get(&(subject, service.to_vec())).copied()
    }

    pub fn service(&self, service: &[u8]) -> Option<ServiceBinding> {
        self.services.get(service).copied()
    }

    pub fn outstanding_tickets(&self) -> usize {
        self.tickets.len()
    }

    fn require_role(
        &self,
        actor: DomainIdentity,
        role: Roles,
    ) -> Result<PrincipalId, AuthorizationError> {
        let record = self.identities.get(&actor).ok_or(AuthorizationError::UnknownIdentity)?;
        if !record.roles.contains(role) {
            return Err(if role == Roles::POLICY_ADMIN {
                AuthorizationError::PolicyAdministratorRequired
            } else {
                AuthorizationError::ServiceManagerRequired
            });
        }
        Ok(record.principal)
    }

    fn active_binding(&self, service: &[u8]) -> Result<ServiceBinding, AuthorizationError> {
        let binding =
            self.services.get(service).copied().ok_or(AuthorizationError::ServiceMissing)?;
        if !binding.active {
            return Err(AuthorizationError::ServiceInactive);
        }
        Ok(binding)
    }

    fn validate_service(&self, service: &[u8]) -> Result<(), AuthorizationError> {
        if service.is_empty() || service.len() > self.limits.service_id_bytes {
            Err(AuthorizationError::InvalidService)
        } else {
            Ok(())
        }
    }
}

impl Default for PolicyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded wire format for the co-located name-service policy endpoint.
/// Request bytes are carried in a copied memory object; caller identity is
/// deliberately absent because it comes from the kernel IPC envelope.
pub mod wire {
    use super::AuthorizationRights;

    pub const MAGIC: u32 = 0x3154_5541; // "AUT1" little-endian
    pub const HEADER_LEN: usize = 32;
    pub const MAX_SERVICE_LEN: usize = 256;
    pub const MAX_REQUEST_LEN: usize = HEADER_LEN + MAX_SERVICE_LEN;

    const KIND_LOOKUP: u16 = 1;
    const KIND_SET_POLICY: u16 = 2;
    const KIND_PUBLISH: u16 = 3;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Request<'a> {
        Lookup {
            service: &'a [u8],
            requested: AuthorizationRights,
        },
        SetPolicy {
            service: &'a [u8],
            subject: u64,
            allowed: AuthorizationRights,
            expected_version: u64,
        },
        Publish {
            service: &'a [u8],
            ceiling: AuthorizationRights,
        },
    }

    fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
    }

    fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
        Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
    }

    pub fn decode(bytes: &[u8]) -> Option<Request<'_>> {
        if bytes.len() < HEADER_LEN || read_u32(bytes, 0)? != MAGIC || read_u32(bytes, 12)? != 0 {
            return None;
        }
        let kind = read_u16(bytes, 4)?;
        let name_len = usize::from(read_u16(bytes, 6)?);
        if name_len == 0 || name_len > MAX_SERVICE_LEN || bytes.len() != HEADER_LEN + name_len {
            return None;
        }
        let rights = AuthorizationRights::from_bits(read_u32(bytes, 8)?)?;
        let subject = read_u64(bytes, 16)?;
        let version = read_u64(bytes, 24)?;
        let service = &bytes[HEADER_LEN..];
        match kind {
            KIND_LOOKUP if subject == 0 && version == 0 && !rights.is_empty() => {
                Some(Request::Lookup {
                    service,
                    requested: rights,
                })
            }
            KIND_SET_POLICY if subject != 0 => Some(Request::SetPolicy {
                service,
                subject,
                allowed: rights,
                expected_version: version,
            }),
            KIND_PUBLISH if subject == 0 && version == 0 && !rights.is_empty() => {
                Some(Request::Publish {
                    service,
                    ceiling: rights,
                })
            }
            _ => None,
        }
    }

    fn encode(
        kind: u16,
        service: &[u8],
        rights: AuthorizationRights,
        subject: u64,
        version: u64,
        output: &mut [u8],
    ) -> Option<usize> {
        if service.is_empty() || service.len() > MAX_SERVICE_LEN {
            return None;
        }
        let len = HEADER_LEN.checked_add(service.len())?;
        let bytes = output.get_mut(..len)?;
        bytes.fill(0);
        bytes[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&kind.to_le_bytes());
        bytes[6..8].copy_from_slice(&(service.len() as u16).to_le_bytes());
        bytes[8..12].copy_from_slice(&rights.bits().to_le_bytes());
        bytes[16..24].copy_from_slice(&subject.to_le_bytes());
        bytes[24..32].copy_from_slice(&version.to_le_bytes());
        bytes[HEADER_LEN..len].copy_from_slice(service);
        Some(len)
    }

    pub fn encode_lookup(
        service: &[u8],
        requested: AuthorizationRights,
        output: &mut [u8],
    ) -> Option<usize> {
        if requested.is_empty() {
            return None;
        }
        encode(KIND_LOOKUP, service, requested, 0, 0, output)
    }

    pub fn encode_set_policy(
        service: &[u8],
        subject: u64,
        allowed: AuthorizationRights,
        expected_version: u64,
        output: &mut [u8],
    ) -> Option<usize> {
        if subject == 0 {
            return None;
        }
        encode(KIND_SET_POLICY, service, allowed, subject, expected_version, output)
    }

    pub fn encode_publish(
        service: &[u8],
        ceiling: AuthorizationRights,
        output: &mut [u8],
    ) -> Option<usize> {
        if ceiling.is_empty() {
            return None;
        }
        encode(KIND_PUBLISH, service, ceiling, 0, 0, output)
    }
}
