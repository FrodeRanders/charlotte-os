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
//! `OP_REGISTER_KEYED`/`OP_LOOKUP_KEYED` remain a quarantined compatibility
//! gate. Production callers use the explicit-rights authorization operations,
//! whose subject identity and roles come only from the kernel IPC envelope.
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
    ipc_close,
    ipc_recv_block_authenticated,
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

const STATUS_SNAPSHOT_MAX: usize = 4096;

struct Registration {
    connection: u64,
    generation: i64,
    access_key: u64,
}

type Registry = BTreeMap<Vec<u8>, Registration>;

#[derive(Debug)]
enum PendingLookup {
    Legacy {
        reply: u64,
        access_key: u64,
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
            (registration.connection != 0).then_some(catten_services::broker::CatalogTarget {
                generation: registration.generation as u64,
                connection: registration.connection,
            })
        })
    }
}
/// Deferred lookups: name → reply token and the interim bearer key supplied by
/// its caller. Retaining the key is necessary because registration may
/// establish the gate after the lookup has blocked. The waitlist is the
/// service's *event-broker* face; the registry is its *catalog* face (see
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

fn read_generation(message: &IpcMessage) -> Option<u64> {
    if message.memory == 0 {
        return None;
    }
    let (name_scratch_vaddr_1_map_status, name_scratch_vaddr_1) =
        memory_map_any(message.memory, false);
    if unsafe { name_scratch_vaddr_1_map_status } != 0 {
        unsafe { memory_close(message.memory) };
        return None;
    }
    let generation = unsafe { core::ptr::read_volatile(name_scratch_vaddr_1 as *const u64) };
    unsafe {
        memory_unmap(message.memory);
        memory_close(message.memory);
    }
    Some(generation)
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
    if registration.connection == 0
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
            registration.connection,
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
    if registration.connection == 0
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
            registration.connection,
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
        if end > STATUS_SNAPSHOT_MAX || record.service.len() > u16::MAX as usize {
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
    key: Vec<u8>,
    connection: u64,
    access_key: u64,
    ceiling: AuthorizationRights,
) -> i64 {
    let generation = match registry.get(&key) {
        Some(previous) => previous.generation.checked_add(1),
        None => Some(1),
    };
    let Some(generation) = generation else {
        // Fail closed instead of wrapping a live generation and making stale
        // generation-fenced cleanup authoritative again.
        if connection != 0 {
            unsafe {
                ipc_close(connection);
            }
        }
        return ns::ERR_INVALID;
    };
    let binding = match policy.publish_service(publisher, &key, ceiling) {
        Ok(binding)
            if i64::try_from(binding.generation) == Ok(generation) && binding.generation != 0 =>
        {
            binding
        }
        _ => {
            if connection != 0 {
                ipc_close(connection);
            }
            return ns::ERR_ACCESS_DENIED;
        }
    };
    // Publishing the new entry is the replacement linearization point. Retire
    // the old connection only after no subsequent lookup can observe it.
    let previous = registry.insert(
        key.clone(),
        Registration {
            connection,
            generation,
            access_key,
        },
    );
    if let Some(previous) = previous
        && previous.connection != 0
    {
        unsafe {
            ipc_close(previous.connection);
        }
    }

    // Wake all callers only after the new generation is authoritative.
    for waiter in waitlist.fire(&key) {
        match waiter {
            PendingLookup::Legacy {
                reply,
                access_key: caller_key,
            } => {
                if access_key != 0 && access_key != caller_key {
                    unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
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
    binding.generation as i64
}

fn lookup_or_defer(
    registry: &Registry,
    waitlist: &mut Waitlist,
    key: &[u8],
    reply: u64,
    caller_key: u64,
) {
    match registry.get(key) {
        Some(registration) if registration.connection != 0 => {
            if registration.access_key != 0 && registration.access_key != caller_key {
                unsafe { ipc_reply(reply, ns::ERR_ACCESS_DENIED) };
                return;
            }
            reply_connection_or_error(reply, registration.connection, registration.generation);
        }
        _ => {
            // Defer: the event broker retains the reply token until the
            // service registers (fulfillment by the publishing side).
            let _ = waitlist.park(
                key,
                PendingLookup::Legacy {
                    reply,
                    access_key: caller_key,
                },
                &RegistryCatalog(registry),
            );
        }
    }
}

fn try_lookup(registry: &Registry, key: &[u8], reply: u64) {
    match registry.get(key) {
        Some(registration) if registration.connection != 0 && registration.access_key == 0 => {
            reply_connection_or_error(reply, registration.connection, registration.generation);
        }
        Some(registration) if registration.connection != 0 => unsafe {
            ipc_reply(reply, ns::ERR_ACCESS_DENIED);
        },
        _ => unsafe {
            ipc_reply(reply, ns::ERR_NOT_FOUND);
        },
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

    loop {
        let message = ipc_recv_block_authenticated(endpoint);
        if message.status == ipc_status::ENDPOINT_CLOSED {
            unsafe { thread_exit() };
        }
        if !message.is_ok() {
            continue;
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
                        scalar_key(message.arg0),
                        message.connection,
                        0,
                        AuthorizationRights::CLIENT,
                    )
                };
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, result);
                    }
                }
            }
            ns::OP_REGISTER_KEYED => {
                let access_key = unsafe { ns::read_access_key(message.memory) };
                let result = if message.connection == 0 {
                    ns::ERR_INVALID
                } else {
                    register(
                        &mut registry,
                        &mut waitlist,
                        &mut policy,
                        &mut audit,
                        own_identity,
                        scalar_key(message.arg0),
                        message.connection,
                        access_key,
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
                        key,
                        connection,
                        0,
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
                    &registry,
                    &mut waitlist,
                    &scalar_key(message.arg0),
                    message.reply,
                    0,
                );
            }
            ns::OP_UNREGISTER => {
                let key = scalar_key(message.arg0);
                let current = registry
                    .get(&key)
                    .filter(|registration| registration.connection != 0)
                    .map(|registration| registration.generation);
                let result = match current {
                    Some(generation)
                        if policy
                            .unpublish_service(own_identity, &key, generation as u64)
                            .is_ok() =>
                    {
                        let registration = registry.get_mut(&key).expect("registration vanished");
                        ipc_close(registration.connection);
                        registration.connection = 0;
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
                let key = scalar_key(message.arg0);
                let expected_generation = read_generation(&message);
                let current = registry
                    .get(&key)
                    .filter(|registration| registration.connection != 0)
                    .map(|registration| registration.generation);
                let result = match (current, expected_generation) {
                    (Some(generation), Some(expected))
                        if u64::try_from(generation) == Ok(expected)
                            && policy.unpublish_service(own_identity, &key, expected).is_ok() =>
                    {
                        let registration = registry.get_mut(&key).expect("registration vanished");
                        ipc_close(registration.connection);
                        registration.connection = 0;
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
            ns::OP_TRY_LOOKUP => {
                if message.reply != 0 {
                    try_lookup(&registry, &scalar_key(message.arg0), message.reply);
                }
            }
            ns::OP_LOOKUP_KEYED => {
                if message.reply == 0 {
                    continue;
                }
                let caller_key = unsafe { ns::read_access_key(message.memory) };
                lookup_or_defer(
                    &registry,
                    &mut waitlist,
                    &scalar_key(message.arg0),
                    message.reply,
                    caller_key,
                );
            }
            ns::OP_LOOKUP_NAMED => {
                let key = read_named_key(&message);
                if message.reply == 0 {
                    continue;
                }
                match key {
                    Some(key) => lookup_or_defer(&registry, &mut waitlist, &key, message.reply, 0),
                    None => unsafe {
                        ipc_reply(message.reply, ns::ERR_INVALID);
                    },
                }
            }
            ns::OP_TRY_LOOKUP_NAMED => {
                let key = read_named_key(&message);
                if message.reply != 0 {
                    match key {
                        Some(key) => try_lookup(&registry, &key, message.reply),
                        None => unsafe {
                            ipc_reply(message.reply, ns::ERR_INVALID);
                        },
                    }
                }
            }
            ns::OP_REGISTER_AUTHORIZED => {
                let request = read_authorization_request(&message);
                let actor = synchronize_sender(&mut policy, &message);
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
                        service.to_vec(),
                        message.connection,
                        0,
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
            ns::OP_LOOKUP_FOR_GRANT => {
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
                            (Some(target), Some(target_principal)) => {
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
            ns::OP_STATUS => {
                let cap = memory_alloc(1);
                if cap == 0 {
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, ns::ERR_BAD_OPCODE) };
                    }
                    continue;
                }
                let (status_scratch_map_status, status_scratch_vaddr) = memory_map_any(cap, true);
                if status_scratch_map_status != 0 {
                    memory_close(cap);
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, ns::ERR_BAD_OPCODE) };
                    }
                    continue;
                }
                let mut length = 0usize;
                unsafe {
                    core::ptr::write_volatile(
                        (status_scratch_vaddr + ns::STATUS_OFFSET_MAGIC as usize * 4) as *mut u32,
                        ns::STATUS_MAGIC,
                    );
                    core::ptr::write_volatile(
                        (status_scratch_vaddr + ns::STATUS_OFFSET_REGISTERED as usize * 4)
                            as *mut u32,
                        registry.len() as u32,
                    );
                    core::ptr::write_volatile(
                        (status_scratch_vaddr + ns::STATUS_OFFSET_PENDING as usize * 4) as *mut u32,
                        waitlist.len() as u32,
                    );
                }
                length += 12;
                for key in registry.keys() {
                    let name_len = key.len().min(255);
                    if length + 1 + name_len > STATUS_SNAPSHOT_MAX {
                        break;
                    }
                    unsafe {
                        core::ptr::write_volatile(
                            (status_scratch_vaddr + length) as *mut u8,
                            name_len as u8,
                        );
                        core::ptr::copy_nonoverlapping(
                            key.as_ptr(),
                            (status_scratch_vaddr + length + 1) as *mut u8,
                            name_len,
                        );
                    }
                    length += 1 + name_len;
                }
                memory_unmap(cap);
                if message.reply != 0 {
                    unsafe { ipc_reply_move(message.reply, cap, length as i64) };
                } else {
                    memory_close(cap);
                }
            }
            _ => {
                if message.memory != 0 {
                    unsafe {
                        memory_close(message.memory);
                    }
                }
                if message.connection != 0 {
                    unsafe {
                        ipc_close(message.connection);
                    }
                }
                if message.reply != 0 {
                    unsafe {
                        ipc_reply(message.reply, ns::ERR_BAD_OPCODE);
                    }
                }
            }
        }
    }
}

catten_rt::entry!(main);
