#![allow(unused_unsafe)]
//! The CharlotteOS userspace name service.
//!
//! Runs in its own EL0 protection domain. Maps service names to
//! `(re-delegable connection, instance generation)`.
//!
//! **Deferred lookups:** if OP_LOOKUP arrives before the service registers,
//! the name service retains the reply token. When the service later calls
//! OP_REGISTER, all waiting callers receive their connections. No polling,
//! no retry loops — the caller's future resolves when the service appears.
//!
//! Public bootstrap naming and explicit-rights authorization are separate
//! operations. Subject identity and roles come only from the kernel envelope;
//! retired bearer-key opcodes are unsupported.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    MAX_NAME_LEN,
    broker::{
        Catalog,
        EventBroker,
    },
    ns,
};
use catten_syscall::{
    IpcMessage,
    IpcRights,
    completion_status,
    cq_read,
    cq_wait_checked,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_recv,
    ipc_reply,
    ipc_reply_connection,
    ipc_reply_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use charlotte_authorization::{
    AuditLog,
    AuditOutcome,
    AuthorizationError,
    AuthorizationRights,
    DomainIdentity,
    PolicyStore,
    PrincipalId,
    Roles,
    wire,
};
use charlotte_launch::ns_status as status;

#[path = "ns/status_snapshot.rs"]
mod status_snapshot;

const AUDIT_SNAPSHOT_MAX: usize = 4096;

struct Registration {
    publication: Option<catten_services::ns_publication::Publication>,
    generation: i64,
    /// Stable principal of the domain that created the registration, when the
    /// kernel-authenticated envelope supplied one. Only this principal (or a
    /// service manager) may request retirement. NS's owned endpoint-death
    /// producer independently retires its exact current publication.
    registrar: Option<PrincipalId>,
    /// Whether this entry requires policy-mediated lookup and rejects public
    /// replacement. Both publication classes have a `PolicyStore` binding;
    /// authorized lookup still checks its caller and requested rights.
    authorized: bool,
}

impl Registration {
    fn connection(&self) -> u64 {
        self.publication.as_ref().map_or(0, |owner| owner.connection().as_raw())
    }
}

type Registry = BTreeMap<Vec<u8>, Registration>;

fn publication_generation(
    registry: &Registry,
    query: &mut catten_rt::owned::IncomingMessage,
) -> i64 {
    if query.connection.is_some() {
        return ns::ERR_INVALID;
    }
    let Ok((key, _)) = generation_request(query, catten_services::ns_generation::Action::Observe)
    else {
        return ns::ERR_INVALID;
    };
    let Some(registration) = registry.get(&key).filter(|entry| entry.connection() != 0) else {
        return ns::ERR_NOT_FOUND;
    };
    // The reactor drains this publication's persistent exact-endpoint watch
    // before intake. This read borrows registry metadata and mints no resource.
    registration.generation
}

#[derive(Debug)]
enum PendingLookup {
    Public {
        reply: u64,
        /// Kernel-authenticated caller identity when available, retained so a
        /// deferred denial of a policy-published registration can be audited.
        actor: Option<DomainIdentity>,
    },
    Authorized {
        reply: u64,
        caller: DomainIdentity,
        requested: AuthorizationRights,
    },
    Grant {
        reply: u64,
        actor: DomainIdentity,
        target: DomainIdentity,
        target_principal: PrincipalId,
        requested: AuthorizationRights,
    },
}

/// The registry viewed as an immediate catalog (the broker's lookups).
struct RegistryCatalog<'a>(&'a Registry);

impl catten_services::broker::Catalog for RegistryCatalog<'_> {
    fn resolve(&self, name: &[u8]) -> Option<catten_services::broker::CatalogTarget> {
        // The unregister tombstone (connection == 0) is not a live
        // registration: resolving it would make KeyedWaitlist::park return
        // the waiter instead of parking it, and the lookup path would then
        // discard the reply token (a lost reply and a forever-stalled
        // caller).
        self.0.get(name).and_then(|registration| {
            (registration.connection() != 0).then_some(catten_services::broker::CatalogTarget {
                generation: registration.generation as u64,
                connection: registration.connection(),
            })
        })
    }
}
/// Deferred lookups retain the reply and captured caller identity. The waitlist
/// is the service's *event-broker* face; the registry is its *catalog* face (see
/// `catten_services::broker`).
type Waitlist = catten_services::broker::KeyedWaitlist<PendingLookup>;

fn scalar_key(packed: u64) -> Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}

fn read_named_key(message: &IpcMessage) -> Option<Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let len = message.arg0 as usize;
    if len == 0 || len > MAX_NAME_LEN {
        unsafe {
            memory_close(message.memory);
        }
        return None;
    }
    let (name_scratch_vaddr_0_map_status, name_scratch_vaddr_0) =
        memory_map_any(message.memory, false);
    if unsafe { name_scratch_vaddr_0_map_status } != 0 {
        unsafe {
            memory_close(message.memory);
        }
        return None;
    }
    let mut key = Vec::with_capacity(len);
    unsafe {
        let src = name_scratch_vaddr_0 as *const u8;
        for i in 0..len {
            key.push(core::ptr::read_volatile(src.add(i)));
        }
        memory_unmap(message.memory);
        memory_close(message.memory);
    }
    Some(key)
}

fn read_authorization_request(message: &IpcMessage) -> Option<Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let len = match usize::try_from(message.arg0) {
        Ok(len) => len,
        Err(_) => {
            memory_close(message.memory);
            return None;
        }
    };
    if !(wire::HEADER_LEN..=wire::MAX_REQUEST_LEN).contains(&len) {
        memory_close(message.memory);
        return None;
    }
    let (status, base) = memory_map_any(message.memory, false);
    if status != 0 {
        memory_close(message.memory);
        return None;
    }
    let mut bytes = Vec::with_capacity(len);
    unsafe {
        let source = base as *const u8;
        for offset in 0..len {
            bytes.push(core::ptr::read_volatile(source.add(offset)));
        }
    }
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(bytes)
}

fn synchronize_sender(
    policy: &mut PolicyStore,
    message: &IpcMessage,
) -> Result<DomainIdentity, AuthorizationError> {
    let identity = DomainIdentity::new(message.sender, message.sender_generation)
        .ok_or(AuthorizationError::UnknownIdentity)?;
    let principal =
        PrincipalId::new(message.sender_principal).ok_or(AuthorizationError::UnknownIdentity)?;
    let roles =
        Roles::from_bits(message.sender_roles).ok_or(AuthorizationError::UnknownIdentity)?;
    policy.provision_identity_from_supervisor(identity, principal, roles)?;
    Ok(identity)
}

/// Decode the complete owned receive transaction before registry/policy mutation.
fn generation_request(
    query: &mut catten_rt::owned::IncomingMessage,
    action: catten_services::ns_generation::Action,
) -> Result<(Vec<u8>, u64), i64> {
    use catten_services::ns_generation as protocol;
    if query.connection.is_some() {
        return Err(ns::ERR_INVALID);
    }
    let length = usize::try_from(query.arg0).map_err(|_| ns::ERR_INVALID)?;
    if !(protocol::HEADER_LEN..=protocol::MAX_FRAME_LEN).contains(&length) {
        return Err(ns::ERR_INVALID);
    }
    let memory = query.memory.take().ok_or(ns::ERR_INVALID)?;
    if memory.len() < length {
        return Err(ns::ERR_INVALID);
    }
    let mapping = memory.map_read_only().map_err(|_| ns::ERR_INVALID)?;
    let request = protocol::decode(action, &mapping.as_slice()[..length]).ok_or(ns::ERR_INVALID)?;
    let mut name = Vec::new();
    name.try_reserve_exact(request.name.len()).map_err(|_| ns::ERR_INVALID)?;
    name.extend_from_slice(request.name);
    let generation = request.generation;
    drop(mapping.unmap().map_err(|_| ns::ERR_INVALID)?);
    Ok((name, generation))
}

fn unregister_generation(
    registry: &mut Registry,
    policy: &mut PolicyStore,
    own_identity: DomainIdentity,
    query: &mut catten_rt::owned::IncomingMessage,
) -> i64 {
    let Ok((key, expected)) =
        generation_request(query, catten_services::ns_generation::Action::Retire)
    else {
        return ns::ERR_INVALID;
    };
    let Some(identity) = DomainIdentity::new(query.sender, query.sender_generation) else {
        return ns::ERR_NOT_FOUND;
    };
    let Some(principal) = PrincipalId::new(query.sender_principal) else {
        return ns::ERR_NOT_FOUND;
    };
    let Some(roles) = Roles::from_bits(query.sender_roles) else {
        return ns::ERR_NOT_FOUND;
    };
    if policy.provision_identity_from_supervisor(identity, principal, roles).is_err() {
        return ns::ERR_NOT_FOUND;
    }
    let Some(registration) = registry.get(&key).filter(|entry| entry.connection() != 0) else {
        return ns::ERR_NOT_FOUND;
    };
    let Ok(current) = u64::try_from(registration.generation) else {
        return ns::ERR_NOT_FOUND;
    };
    if !catten_services::publication_generation::retirement_allowed(
        current,
        expected,
        registration.registrar.map(PrincipalId::get),
        Some(principal.get()),
        roles.contains(Roles::SERVICE_MANAGER),
    ) || policy.unpublish_service(own_identity, &key, expected).is_err()
    {
        return ns::ERR_NOT_FOUND;
    }
    let registration = registry.get_mut(&key).expect("validated registration vanished");
    // Logical unpublication precedes disposal of the complete source/watch.
    // Best-effort connection close does not prove physical quiescence.
    drop(registration.publication.take());
    current as i64
}

fn reply_connection_or_error(reply: u64, connection: u64, generation: i64) {
    let status = unsafe {
        ipc_reply_connection(reply, connection, IpcRights::SEND | IpcRights::CALL, generation)
    };
    if status != 0 {
        // A registration without MINT_CONNECTION cannot be handed to a
        // lookup caller. Do not strand the retained reply token: report a
        // protocol error so the caller can discard/retry it.
        unsafe {
            ipc_reply(reply, ns::ERR_INVALID);
        }
    }
}

fn record_denial(
    policy: &PolicyStore,
    audit: &mut AuditLog,
    caller: DomainIdentity,
    service: &[u8],
    requested: AuthorizationRights,
    error: AuthorizationError,
) {
    let _ = audit.record(
        caller,
        policy.principal_for(caller),
        service,
        requested,
        AuthorizationRights::NONE,
        policy.service(service).map_or(0, |binding| binding.generation),
        policy
            .principal_for(caller)
            .and_then(|principal| policy.policy(principal, service))
            .map_or(0, |rule| rule.version),
        AuditOutcome::Denied(error),
    );
}

fn authorize_and_reply(
    policy: &mut PolicyStore,
    audit: &mut AuditLog,
    registry: &Registry,
    service: &[u8],
    caller: DomainIdentity,
    requested: AuthorizationRights,
    reply: u64,
) {
    if !audit.can_record() {
        unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
        return;
    }
    let principal = policy.principal_for(caller);
    let grant = match policy.authorize_now(caller, service, requested) {
        Ok(grant) => grant,
        Err(error) => {
            record_denial(policy, audit, caller, service, requested, error);
            unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
            return;
        }
    };
    let Some(registration) = registry.get(service) else {
        record_denial(
            policy,
            audit,
            caller,
            service,
            requested,
            AuthorizationError::ServiceMissing,
        );
        unsafe { ipc_reply(reply, ns::ERR_NOT_FOUND) };
        return;
    };
    if registration.connection() == 0
        || u64::try_from(registration.generation) != Ok(grant.service_generation)
    {
        record_denial(
            policy,
            audit,
            caller,
            service,
            requested,
            AuthorizationError::StaleServiceGeneration,
        );
        unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
        return;
    }

    let status = unsafe {
        ipc_reply_connection(
            reply,
            registration.connection(),
            IpcRights::from_bits(grant.rights.bits()),
            registration.generation,
        )
    };
    let outcome = if status == 0 {
        AuditOutcome::Issued
    } else {
        unsafe { ipc_reply(reply, ns::ERR_INVALID) };
        AuditOutcome::DelegationFailed
    };
    let _ = audit.record(
        caller,
        principal,
        service,
        requested,
        if status == 0 {
            grant.rights
        } else {
            AuthorizationRights::NONE
        },
        grant.service_generation,
        grant.policy_version,
        outcome,
    );
}

#[allow(clippy::too_many_arguments)]
fn authorize_grant_and_reply(
    policy: &mut PolicyStore,
    audit: &mut AuditLog,
    registry: &Registry,
    service: &[u8],
    actor: DomainIdentity,
    target: DomainIdentity,
    target_principal: PrincipalId,
    requested: AuthorizationRights,
    reply: u64,
) {
    if !audit.can_record() {
        unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
        return;
    }
    let grant =
        match policy.authorize_for_admin(actor, target, target_principal, service, requested) {
            Ok(grant) => grant,
            Err(error) => {
                record_denial(policy, audit, target, service, requested, error);
                unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
                return;
            }
        };
    let Some(registration) = registry.get(service) else {
        record_denial(
            policy,
            audit,
            target,
            service,
            requested,
            AuthorizationError::ServiceMissing,
        );
        unsafe { ipc_reply(reply, ns::ERR_NOT_FOUND) };
        return;
    };
    if registration.connection() == 0
        || u64::try_from(registration.generation) != Ok(grant.service_generation)
    {
        record_denial(
            policy,
            audit,
            target,
            service,
            requested,
            AuthorizationError::StaleServiceGeneration,
        );
        unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
        return;
    }
    // The grant controller is the only recipient of this re-delegable
    // connection. It attenuates the application reply back to SEND/CALL; the
    // application never receives MINT_CONNECTION or name-service authority.
    let delegated_rights = IpcRights::from_bits(grant.rights.bits()) | IpcRights::MINT_CONNECTION;
    let status = unsafe {
        ipc_reply_connection(
            reply,
            registration.connection(),
            delegated_rights,
            registration.generation,
        )
    };
    let outcome = if status == 0 {
        AuditOutcome::Issued
    } else {
        unsafe { ipc_reply(reply, ns::ERR_INVALID) };
        AuditOutcome::DelegationFailed
    };
    let _ = audit.record(
        target,
        Some(target_principal),
        service,
        requested,
        if status == 0 {
            grant.rights
        } else {
            AuthorizationRights::NONE
        },
        grant.service_generation,
        grant.policy_version,
        outcome,
    );
}

#[allow(clippy::too_many_arguments)]
fn authorized_lookup_or_defer(
    policy: &mut PolicyStore,
    audit: &mut AuditLog,
    registry: &Registry,
    waitlist: &mut Waitlist,
    service: &[u8],
    caller: DomainIdentity,
    requested: AuthorizationRights,
    reply: u64,
) {
    if RegistryCatalog(registry).resolve(service).is_some() {
        authorize_and_reply(policy, audit, registry, service, caller, requested, reply);
    } else if waitlist.waiter_count() >= catten_services::broker::MAX_WAITERS {
        unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
    } else {
        let _ = waitlist.park(
            service,
            PendingLookup::Authorized {
                reply,
                caller,
                requested,
            },
            &RegistryCatalog(registry),
        );
    }
}

fn write_audit_snapshot(audit: &AuditLog, base: usize) -> usize {
    const HEADER_LEN: usize = 8;
    const RECORD_HEADER_LEN: usize = 64;
    unsafe {
        core::ptr::write_unaligned(base as *mut u32, ns::AUDIT_MAGIC);
        core::ptr::write_unaligned((base + 4) as *mut u32, 0);
    }
    let mut offset = HEADER_LEN;
    let mut count = 0u32;
    for record in audit.iter() {
        let Some(end) = offset
            .checked_add(RECORD_HEADER_LEN)
            .and_then(|header_end| header_end.checked_add(record.service.len()))
        else {
            break;
        };
        if end > AUDIT_SNAPSHOT_MAX || record.service.len() > u16::MAX as usize {
            break;
        }
        let principal = record.principal.map_or(0, PrincipalId::get);
        let outcome = match record.outcome {
            AuditOutcome::Issued => 0i32,
            AuditOutcome::Denied(_) => -1,
            AuditOutcome::DelegationFailed => -2,
        };
        unsafe {
            core::ptr::write_unaligned((base + offset) as *mut u64, record.sequence);
            core::ptr::write_unaligned((base + offset + 8) as *mut u64, record.caller.asid());
            core::ptr::write_unaligned(
                (base + offset + 16) as *mut u64,
                record.caller.generation(),
            );
            core::ptr::write_unaligned((base + offset + 24) as *mut u64, principal);
            core::ptr::write_unaligned((base + offset + 32) as *mut u64, record.service_generation);
            core::ptr::write_unaligned((base + offset + 40) as *mut u64, record.policy_version);
            core::ptr::write_unaligned((base + offset + 48) as *mut u32, record.requested.bits());
            core::ptr::write_unaligned((base + offset + 52) as *mut u32, record.granted.bits());
            core::ptr::write_unaligned((base + offset + 56) as *mut i32, outcome);
            core::ptr::write_unaligned(
                (base + offset + 60) as *mut u16,
                record.service.len() as u16,
            );
            core::ptr::write_unaligned((base + offset + 62) as *mut u16, 0);
            core::ptr::copy_nonoverlapping(
                record.service.as_ptr(),
                (base + offset + RECORD_HEADER_LEN) as *mut u8,
                record.service.len(),
            );
        }
        offset = end;
        count += 1;
    }
    unsafe { core::ptr::write_unaligned((base + 4) as *mut u32, count) };
    offset
}

#[allow(clippy::too_many_arguments)]
fn register(
    registry: &mut Registry,
    waitlist: &mut Waitlist,
    policy: &mut PolicyStore,
    audit: &mut AuditLog,
    publisher: DomainIdentity,
    authorized: bool,
    registrar: Option<PrincipalId>,
    key: Vec<u8>,
    connection: u64,
    ceiling: AuthorizationRights,
) -> i64 {
    // Unique receive ABI boundary: this branch transfers the incoming
    // connection once; no handler or raw cleanup uses it after adoption.
    let Ok(connection) = (unsafe { catten_rt::owned::Connection::from_raw(connection) }) else {
        return ns::ERR_INVALID;
    };
    let replacing_live = registry.get(&key).is_some_and(|entry| entry.publication.is_some());
    let live = registry.values().filter(|entry| entry.publication.is_some()).count();
    if !catten_services::ns_publication::publication_admitted(live, replacing_live) {
        return ns::ERR_INVALID;
    }
    // Public replacement is registrant-bound: it may only replace an active
    // public entry created by the same stable principal. An entry published
    // through the authorized opcode can never be displaced or downgraded by
    // a public opcode; a service manager must unregister it first.
    if !authorized
        && let Some(previous) = registry.get(&key).filter(|previous| previous.connection() != 0)
        && (previous.authorized || previous.registrar.is_none() || previous.registrar != registrar)
    {
        return ns::ERR_ACCESS_DENIED;
    }
    // Watch admission precedes every policy/registry mutation. Failure closes
    // only this unused candidate and cannot displace the existing generation.
    let publication = match catten_services::ns_publication::Publication::prepare(connection) {
        Ok(publication) => publication,
        Err((_connection, _error)) => return ns::ERR_INVALID,
    };
    let binding = match policy.publish_service(publisher, &key, ceiling) {
        Ok(binding) if binding.generation != 0 => binding,
        _ => {
            return ns::ERR_ACCESS_DENIED;
        }
    };
    let Ok(generation) = i64::try_from(binding.generation) else {
        return ns::ERR_INVALID;
    };
    // Publishing the new entry is the replacement linearization point. Retire
    // the old connection only after no subsequent lookup can observe it.
    let previous = registry.insert(
        key.clone(),
        Registration {
            publication: Some(publication),
            generation,
            registrar,
            authorized,
        },
    );
    drop(previous); // Cancel the replaced watch before its source closes.
    let connection = registry.get(&key).expect("published registration missing").connection();

    // Wake all callers only after the new generation is authoritative.
    for waiter in waitlist.fire(&key) {
        match waiter {
            PendingLookup::Public {
                reply,
                actor,
            } => {
                if authorized {
                    deny_policy_published(policy, audit, &key, actor, reply);
                } else {
                    reply_connection_or_error(reply, connection, generation);
                }
            }
            PendingLookup::Authorized {
                reply,
                caller,
                requested,
            } => authorize_and_reply(policy, audit, registry, &key, caller, requested, reply),
            PendingLookup::Grant {
                reply,
                actor,
                target,
                target_principal,
                requested,
            } => authorize_grant_and_reply(
                policy,
                audit,
                registry,
                &key,
                actor,
                target,
                target_principal,
                requested,
                reply,
            ),
        }
    }
    generation
}

/// The kernel-authenticated caller identity for audit, when the envelope
/// supplies a valid asid/generation pair.
fn public_actor(message: &IpcMessage) -> Option<DomainIdentity> {
    DomainIdentity::new(message.sender, message.sender_generation)
}

/// A public opcode may not resolve a registration published through the
/// policy-mediated opcode. The denial is audited when the caller identity is
/// available; public lookups remain outside the authorization boundary.
fn deny_policy_published(
    policy: &PolicyStore,
    audit: &mut AuditLog,
    service: &[u8],
    actor: Option<DomainIdentity>,
    reply: u64,
) {
    if let Some(caller) = actor {
        record_denial(
            policy,
            audit,
            caller,
            service,
            AuthorizationRights::SEND | AuthorizationRights::CALL,
            AuthorizationError::RightsDenied,
        );
    }
    unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
}

#[allow(clippy::too_many_arguments)]
fn lookup_or_defer(
    policy: &PolicyStore,
    audit: &mut AuditLog,
    registry: &Registry,
    waitlist: &mut Waitlist,
    key: &[u8],
    reply: u64,
    actor: Option<DomainIdentity>,
) {
    match registry.get(key) {
        Some(registration) if registration.connection() != 0 => {
            if registration.authorized {
                deny_policy_published(policy, audit, key, actor, reply);
                return;
            }
            reply_connection_or_error(reply, registration.connection(), registration.generation);
        }
        _ => {
            // Defer: the event broker retains the reply token until the
            // service registers (fulfillment by the publishing side). Fail
            // closed if the broker is saturated.
            if waitlist.waiter_count() >= catten_services::broker::MAX_WAITERS {
                unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
            } else {
                let _ = waitlist.park(
                    key,
                    PendingLookup::Public {
                        reply,
                        actor,
                    },
                    &RegistryCatalog(registry),
                );
            }
        }
    }
}

fn try_lookup(
    policy: &PolicyStore,
    audit: &mut AuditLog,
    registry: &Registry,
    key: &[u8],
    reply: u64,
    actor: Option<DomainIdentity>,
) {
    match registry.get(key) {
        Some(registration) if registration.connection() != 0 && registration.authorized => {
            deny_policy_published(policy, audit, key, actor, reply);
        }
        Some(registration) if registration.connection() != 0 => {
            reply_connection_or_error(reply, registration.connection(), registration.generation);
        }
        _ => unsafe {
            ipc_reply(reply, ns::ERR_NOT_FOUND);
        },
    }
}

/// NS observes endpoint death internally; callers gain no retirement authority from it.
/// Each hint is resolved through the watch retained with the current binding;
/// removed/replaced watches cannot retire another generation under the name.
fn retire_closed(
    registry: &mut Registry,
    policy: &mut PolicyStore,
    own_identity: DomainIdentity,
    entry: &catten_syscall::CqEntry,
) {
    use catten_services::ns_publication::Observation;
    for (name, registration) in registry.iter_mut() {
        let Some(publication) = registration.publication.as_mut() else {
            continue;
        };
        match publication.observe(entry) {
            Ok(Observation::Unrelated) => continue,
            Ok(Observation::Pending) => return,
            Ok(Observation::Closed) => {
                let Ok(generation) = u64::try_from(registration.generation) else {
                    catten_rt::domain_abort();
                };
                if policy.unpublish_service(own_identity, name, generation).is_err() {
                    // Keep the complete registry transaction on uncertainty;
                    // do not clear the connection or fabricate policy success.
                    catten_rt::domain_abort();
                }
                drop(registration.publication.take());
                catten_rt::logln!(
                    "[ns] endpoint-death retired {:?} generation={generation}",
                    core::str::from_utf8(name).ok()
                );
                return;
            }
            Err(_) => catten_rt::domain_abort(),
        }
    }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let endpoint = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2);

    let mut registry: Registry = BTreeMap::new();
    let mut waitlist: Waitlist = catten_services::broker::KeyedWaitlist::new();
    let own = catten_syscall::get_domain_identity();
    let own_identity = DomainIdentity::new(own.asid, own.generation)
        .expect("name service received an invalid kernel identity");
    let own_principal =
        PrincipalId::new(own.principal).expect("name service received an invalid kernel principal");
    let mut policy = PolicyStore::new();
    policy
        .provision_identity_from_supervisor(
            own_identity,
            own_principal,
            Roles::POLICY_ADMIN | Roles::SERVICE_MANAGER,
        )
        .expect("name-service policy bootstrap failed");
    let mut audit = AuditLog::new(256).expect("invalid authorization audit capacity");
    let mut handled: u32 = 0;

    // Context retains the launch-owned endpoint. Binding is readiness/config
    // only; no borrowed Endpoint view exists yet, so do not adopt this handle.
    if ipc_endpoint_bind_cq(endpoint, 0) != ipc_status::OK {
        catten_rt::domain_abort();
    }
    let cq = ctx.completion_queue_layout();
    loop {
        if ctx.lifecycle().shutdown_requested().is_some() {
            // Readiness is not teardown completion. Preserve the registry and
            // outstanding reply owners through terminal domain abort rather
            // than acknowledge cooperative/physical cleanup or spin on this wake.
            catten_rt::domain_abort();
        }
        while let Some(entry) = unsafe { cq_read(cq.base, cq.entries) } {
            retire_closed(&mut registry, &mut policy, own_identity, &entry);
        }
        let message = ipc_recv(endpoint);
        if message.status == ipc_status::NO_MESSAGE {
            let (_, status) = cq_wait_checked(None, 0);
            if status != completion_status::READY {
                // Wait admission rejection is neither timeout nor cleanup
                // completion. Keep the registry's owners through fail-closed abort.
                catten_rt::domain_abort();
            }
            continue;
        }
        if message.status == ipc_status::ENDPOINT_CLOSED {
            unsafe { thread_exit() };
        }
        if !message.is_ok() {
            catten_rt::domain_abort();
        }
        handled += 1;
        config::write::<u32>(status::HANDLED, handled);
        config::write::<u32>(status::LAST_OPCODE, message.opcode);
        config::write::<u32>(status::WAITERS, waitlist.len() as u32);

        match message.opcode {
            ns::OP_REGISTER => {
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    register(
                        &mut registry,
                        &mut waitlist,
                        &mut policy,
                        &mut audit,
                        own_identity,
                        false,
                        PrincipalId::new(message.sender_principal),
                        scalar_key(message.arg0),
                        message.connection,
                        AuthorizationRights::CLIENT,
                    )
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_REGISTER_NAMED => {
                let key = read_named_key(&message);
                let result = match (key, message.connection) {
                    (Some(key), connection) if connection != 0 => register(
                        &mut registry,
                        &mut waitlist,
                        &mut policy,
                        &mut audit,
                        own_identity,
                        false,
                        PrincipalId::new(message.sender_principal),
                        key,
                        connection,
                        AuthorizationRights::CLIENT,
                    ),
                    (_, connection) => {
                        if connection != 0 {
                            unsafe {
                                ipc_close(connection);
                            }
                        }
                        ns::ERR_INVALID
                    }
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_LOOKUP => {
                if message.reply == 0 {
                    continue;
                }
                lookup_or_defer(
                    &policy,
                    &mut audit,
                    &registry,
                    &mut waitlist,
                    &scalar_key(message.arg0),
                    message.reply,
                    public_actor(&message),
                );
            }
            ns::OP_UNREGISTER => {
                let key = scalar_key(message.arg0);
                let current = registry
                    .get(&key)
                    .filter(|registration| registration.connection() != 0)
                    .map(|registration| registration.generation);
                let actor = synchronize_sender(&mut policy, &message);
                let is_manager = match &actor {
                    Ok(identity) => policy
                        .roles_for(*identity)
                        .is_some_and(|roles| roles.contains(Roles::SERVICE_MANAGER)),
                    Err(_) => false,
                };
                let result = match (current, actor) {
                    (Some(generation), Ok(actor))
                        if is_manager
                            && policy.unpublish_service(actor, &key, generation as u64).is_ok() =>
                    {
                        let registration = registry.get_mut(&key).expect("registration vanished");
                        drop(registration.publication.take());
                        generation
                    }
                    _ => ns::ERR_NOT_FOUND,
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_UNREGISTER_GENERATION => {
                // This branch consumes the successful receive exactly once.
                let Ok(Some(mut query)) =
                    (unsafe { catten_rt::owned::IncomingMessage::from_raw(message) })
                else {
                    continue;
                };
                let result =
                    unregister_generation(&mut registry, &mut policy, own_identity, &mut query);
                if let Some(reply) = query.reply.take() {
                    let _ = reply.reply(result);
                }
            }
            ns::OP_TRY_LOOKUP => {
                if message.reply != 0 {
                    try_lookup(
                        &policy,
                        &mut audit,
                        &registry,
                        &scalar_key(message.arg0),
                        message.reply,
                        public_actor(&message),
                    );
                }
            }
            ns::OP_LOOKUP_NAMED => {
                let key = read_named_key(&message);
                if message.reply == 0 {
                    continue;
                }
                match key {
                    Some(key) => lookup_or_defer(
                        &policy,
                        &mut audit,
                        &registry,
                        &mut waitlist,
                        &key,
                        message.reply,
                        public_actor(&message),
                    ),
                    None => unsafe {
                        ipc_reply(message.reply, ns::ERR_INVALID);
                    },
                }
            }
            ns::OP_TRY_LOOKUP_NAMED => {
                let key = read_named_key(&message);
                if message.reply != 0 {
                    match key {
                        Some(key) => try_lookup(
                            &policy,
                            &mut audit,
                            &registry,
                            &key,
                            message.reply,
                            public_actor(&message),
                        ),
                        None => unsafe {
                            ipc_reply(message.reply, ns::ERR_INVALID);
                        },
                    }
                }
            }
            ns::OP_REGISTER_AUTHORIZED => {
                let request = read_authorization_request(&message);
                let actor = synchronize_sender(&mut policy, &message);
                let registrar = match &actor {
                    Ok(identity) => policy.principal_for(*identity),
                    Err(_) => None,
                };
                let result = match (request.as_deref().and_then(wire::decode), actor) {
                    (
                        Some(wire::Request::Publish {
                            service,
                            ceiling,
                        }),
                        Ok(actor),
                    ) if message.connection != 0 => register(
                        &mut registry,
                        &mut waitlist,
                        &mut policy,
                        &mut audit,
                        actor,
                        true,
                        registrar,
                        service.to_vec(),
                        message.connection,
                        ceiling,
                    ),
                    _ => {
                        if message.connection != 0 {
                            ipc_close(message.connection);
                        }
                        ns::ERR_ACCESS_DENIED
                    }
                };
                if message.reply != 0 {
                    unsafe { ipc_reply(message.reply, result) };
                }
            }
            ns::OP_LOOKUP_AUTHORIZED => {
                let request = read_authorization_request(&message);
                if message.connection != 0 {
                    ipc_close(message.connection);
                }
                let caller = synchronize_sender(&mut policy, &message);
                if message.reply == 0 {
                    continue;
                }
                match (request.as_deref().and_then(wire::decode), caller) {
                    (
                        Some(wire::Request::Lookup {
                            service,
                            requested,
                        }),
                        Ok(caller),
                    ) => authorized_lookup_or_defer(
                        &mut policy,
                        &mut audit,
                        &registry,
                        &mut waitlist,
                        service,
                        caller,
                        requested,
                        message.reply,
                    ),
                    _ => unsafe {
                        ipc_reply(message.reply, ns::ERR_ACCESS_DENIED);
                    },
                }
            }
            ns::OP_SET_POLICY => {
                let request = read_authorization_request(&message);
                if message.connection != 0 {
                    ipc_close(message.connection);
                }
                let actor = synchronize_sender(&mut policy, &message);
                let result = match (request.as_deref().and_then(wire::decode), actor) {
                    (
                        Some(wire::Request::SetPolicy {
                            service,
                            subject,
                            allowed,
                            expected_version,
                        }),
                        Ok(actor),
                    ) => PrincipalId::new(subject)
                        .ok_or(AuthorizationError::UnknownIdentity)
                        .and_then(|subject| {
                            policy.set_policy(actor, subject, service, allowed, expected_version)
                        })
                        .and_then(|rule| {
                            i64::try_from(rule.version)
                                .map_err(|_| AuthorizationError::PolicyVersionExhausted)
                        })
                        .unwrap_or(ns::ERR_ACCESS_DENIED),
                    _ => ns::ERR_ACCESS_DENIED,
                };
                if message.reply != 0 {
                    unsafe { ipc_reply(message.reply, result) };
                }
            }
            ns::OP_AUTH_AUDIT => {
                if message.memory != 0 {
                    memory_close(message.memory);
                }
                if message.connection != 0 {
                    ipc_close(message.connection);
                }
                let actor = synchronize_sender(&mut policy, &message);
                let permitted = actor
                    .ok()
                    .and_then(|actor| policy.roles_for(actor))
                    .is_some_and(|roles| roles.contains(Roles::POLICY_ADMIN));
                if message.reply == 0 {
                    continue;
                }
                if !permitted {
                    unsafe { ipc_reply(message.reply, ns::ERR_ACCESS_DENIED) };
                    continue;
                }
                let cap = memory_alloc(1);
                if cap == 0 {
                    unsafe { ipc_reply(message.reply, ns::ERR_INVALID) };
                    continue;
                }
                let (status, base) = memory_map_any(cap, true);
                if status != 0 {
                    memory_close(cap);
                    unsafe { ipc_reply(message.reply, ns::ERR_INVALID) };
                    continue;
                }
                let length = write_audit_snapshot(&audit, base);
                memory_unmap(cap);
                if ipc_reply_move(message.reply, cap, length as i64) != 0 {
                    memory_close(cap);
                    ipc_reply(message.reply, ns::ERR_INVALID);
                }
            }
            ns::OP_LOOKUP_FOR_GRANT | ns::OP_TRY_LOOKUP_FOR_GRANT => {
                let request = read_authorization_request(&message);
                if message.connection != 0 {
                    ipc_close(message.connection);
                }
                let actor = synchronize_sender(&mut policy, &message);
                if message.reply == 0 {
                    continue;
                }
                match (request.as_deref().and_then(wire::decode), actor) {
                    (
                        Some(wire::Request::GrantLookup {
                            service,
                            requested,
                            target_asid,
                            target_generation,
                            target_principal,
                        }),
                        Ok(actor),
                    ) if policy
                        .roles_for(actor)
                        .is_some_and(|roles| roles.contains(Roles::POLICY_ADMIN)) =>
                    {
                        let target = DomainIdentity::new(target_asid, target_generation);
                        let target_principal = PrincipalId::new(target_principal);
                        match (target, target_principal) {
                            (Some(target), Some(target_principal))
                                if RegistryCatalog(&registry).resolve(service).is_some() =>
                            {
                                authorize_grant_and_reply(
                                    &mut policy,
                                    &mut audit,
                                    &registry,
                                    service,
                                    actor,
                                    target,
                                    target_principal,
                                    requested,
                                    message.reply,
                                );
                            }
                            (Some(_), Some(_)) if message.opcode == ns::OP_TRY_LOOKUP_FOR_GRANT => {
                                // Ownership transfers exactly once from the
                                // authenticated receive ABI for this reply.
                                if let Ok(reply) =
                                    unsafe { catten_rt::owned::ReplyToken::from_raw(message.reply) }
                                {
                                    let _ = reply.reply(ns::ERR_NOT_FOUND);
                                }
                            }
                            (Some(target), Some(target_principal)) => {
                                if waitlist.waiter_count() >= catten_services::broker::MAX_WAITERS {
                                    unsafe {
                                        ipc_reply(message.reply, ns::ERR_ACCESS_DENIED);
                                    }
                                } else {
                                    let _ = waitlist.park(
                                        service,
                                        PendingLookup::Grant {
                                            reply: message.reply,
                                            actor,
                                            target,
                                            target_principal,
                                            requested,
                                        },
                                        &RegistryCatalog(&registry),
                                    );
                                }
                            }
                            _ => unsafe {
                                ipc_reply(message.reply, ns::ERR_ACCESS_DENIED);
                            },
                        }
                    }
                    _ => unsafe {
                        ipc_reply(message.reply, ns::ERR_ACCESS_DENIED);
                    },
                }
            }
            ns::OP_PUBLICATION_GENERATION => {
                // This branch adopts each successful receive exactly once at
                // the raw service ABI boundary; no scalar cleanup follows it.
                let Ok(Some(mut query)) =
                    (unsafe { catten_rt::owned::IncomingMessage::from_raw(message) })
                else {
                    continue;
                };
                let result = publication_generation(&registry, &mut query);
                if let Some(reply) = query.reply.take() {
                    let _ = reply.reply(result);
                }
            }
            ns::OP_STATUS => {
                // Unique receive boundary: this branch owns all attachments
                // and reply authority before validating the observation.
                if let Ok(Some(request)) =
                    unsafe { catten_rt::owned::IncomingMessage::from_raw(message) }
                {
                    status_snapshot::reply_status(request, &registry, waitlist.waiter_count());
                }
            }
            _ => {
                // Unique receive ABI boundary, including retired opcodes: own
                // every attachment before rejecting without decoding its body.
                if let Ok(Some(mut request)) =
                    unsafe { catten_rt::owned::IncomingMessage::from_raw(message) }
                    && let Some(reply) = request.reply.take()
                {
                    let _ = reply.reply(ns::ERR_BAD_OPCODE);
                }
            }
        }
    }
}

catten_rt::entry!(main);
