use charlotte_authorization::{
    AuditLog,
    AuditOutcome,
    AuthorizationError,
    AuthorizationRights as Rights,
    DomainIdentity,
    PolicyLimits,
    PolicyStore,
    PrincipalId,
    Roles,
};

const SERVICE: &[u8] = b"storage";

fn identity(asid: u64, generation: u64) -> DomainIdentity {
    DomainIdentity::new(asid, generation).unwrap()
}

fn principal(raw: u64) -> PrincipalId {
    PrincipalId::new(raw).unwrap()
}

struct Fixture {
    store: PolicyStore,
    admin: DomainIdentity,
    client: DomainIdentity,
    other: DomainIdentity,
    client_principal: PrincipalId,
}

fn fixture() -> Fixture {
    let admin = identity(1, 1);
    let client = identity(2, 1);
    let other = identity(3, 1);
    let client_principal = principal(20);
    let mut store = PolicyStore::new();
    store
        .provision_identity_from_supervisor(
            admin,
            principal(10),
            Roles::POLICY_ADMIN | Roles::SERVICE_MANAGER,
        )
        .unwrap();
    store.provision_identity_from_supervisor(client, client_principal, Roles::NONE).unwrap();
    store.provision_identity_from_supervisor(other, principal(30), Roles::NONE).unwrap();
    store.publish_service(admin, SERVICE, Rights::CLIENT).unwrap();
    Fixture {
        store,
        admin,
        client,
        other,
        client_principal,
    }
}

#[test]
fn default_deny_requires_an_explicit_rule() {
    let mut fixture = fixture();
    assert_eq!(
        fixture.store.issue_ticket(fixture.client, SERVICE, Rights::CALL),
        Err(AuthorizationError::PolicyMissing)
    );
}

#[test]
fn control_plane_roles_are_separate() {
    let mut fixture = fixture();
    assert_eq!(
        fixture.store.set_policy(
            fixture.client,
            fixture.client_principal,
            SERVICE,
            Rights::CALL,
            0,
        ),
        Err(AuthorizationError::PolicyAdministratorRequired)
    );
    assert_eq!(
        fixture.store.publish_service(fixture.client, b"other", Rights::CALL),
        Err(AuthorizationError::ServiceManagerRequired)
    );
}

#[test]
fn issuance_is_bounded_by_policy_and_service_ceiling() {
    let mut fixture = fixture();
    fixture
        .store
        .set_policy(fixture.admin, fixture.client_principal, SERVICE, Rights::CLIENT, 0)
        .unwrap();
    fixture.store.publish_service(fixture.admin, SERVICE, Rights::CALL).unwrap();

    assert_eq!(
        fixture.store.issue_ticket(fixture.client, SERVICE, Rights::SEND),
        Err(AuthorizationError::RightsDenied)
    );
    let grant = fixture.store.authorize_now(fixture.client, SERVICE, Rights::CALL).unwrap();
    assert_eq!(grant.rights, Rights::CALL);
    assert_eq!(grant.service_generation, 2);
}

#[test]
fn policy_change_invalidates_an_unredeemed_ticket() {
    let mut fixture = fixture();
    fixture
        .store
        .set_policy(fixture.admin, fixture.client_principal, SERVICE, Rights::CALL, 0)
        .unwrap();
    let ticket = fixture.store.issue_ticket(fixture.client, SERVICE, Rights::CALL).unwrap();
    fixture
        .store
        .set_policy(fixture.admin, fixture.client_principal, SERVICE, Rights::NONE, 1)
        .unwrap();

    assert_eq!(
        fixture.store.redeem_ticket(fixture.client, ticket),
        Err(AuthorizationError::StalePolicy)
    );
    assert_eq!(fixture.store.outstanding_tickets(), 0);
}

#[test]
fn service_replacement_invalidates_an_unredeemed_ticket() {
    let mut fixture = fixture();
    fixture
        .store
        .set_policy(fixture.admin, fixture.client_principal, SERVICE, Rights::CALL, 0)
        .unwrap();
    let ticket = fixture.store.issue_ticket(fixture.client, SERVICE, Rights::CALL).unwrap();
    fixture.store.publish_service(fixture.admin, SERVICE, Rights::CLIENT).unwrap();

    assert_eq!(
        fixture.store.redeem_ticket(fixture.client, ticket),
        Err(AuthorizationError::StaleServiceGeneration)
    );
}

#[test]
fn a_ticket_is_subject_bound_and_single_use() {
    let mut fixture = fixture();
    fixture
        .store
        .set_policy(fixture.admin, fixture.client_principal, SERVICE, Rights::CALL, 0)
        .unwrap();
    let ticket = fixture.store.issue_ticket(fixture.client, SERVICE, Rights::CALL).unwrap();

    assert_eq!(
        fixture.store.redeem_ticket(fixture.other, ticket),
        Err(AuthorizationError::PrincipalMismatch)
    );
    assert_eq!(
        fixture.store.redeem_ticket(fixture.client, ticket),
        Err(AuthorizationError::TicketMissing)
    );
}

#[test]
fn asid_reuse_does_not_inherit_the_old_principal() {
    let mut fixture = fixture();
    fixture
        .store
        .set_policy(fixture.admin, fixture.client_principal, SERVICE, Rights::CALL, 0)
        .unwrap();
    let ticket = fixture.store.issue_ticket(fixture.client, SERVICE, Rights::CALL).unwrap();
    let replacement = identity(fixture.client.asid(), 2);
    fixture
        .store
        .provision_identity_from_supervisor(replacement, principal(40), Roles::NONE)
        .unwrap();

    assert_eq!(fixture.store.principal_for(fixture.client), None);
    assert_eq!(
        fixture.store.redeem_ticket(fixture.client, ticket),
        Err(AuthorizationError::UnknownIdentity)
    );
}

#[test]
fn generation_fenced_unpublish_preserves_a_replacement() {
    let mut fixture = fixture();
    fixture.store.publish_service(fixture.admin, SERVICE, Rights::CLIENT).unwrap();

    assert_eq!(
        fixture.store.unpublish_service(fixture.admin, SERVICE, 1),
        Err(AuthorizationError::StaleServiceGeneration)
    );
    assert_eq!(fixture.store.service(SERVICE).unwrap().generation, 2);
    assert!(fixture.store.service(SERVICE).unwrap().active);
}

#[test]
fn configured_capacity_limits_fail_closed_without_losing_replacements() {
    let limits = PolicyLimits {
        identities: 1,
        policies: 1,
        services: 1,
        tickets: 1,
        service_id_bytes: 4,
    };
    let mut store = PolicyStore::with_limits(limits).unwrap();
    let admin = identity(1, 1);
    store
        .provision_identity_from_supervisor(
            admin,
            principal(10),
            Roles::POLICY_ADMIN | Roles::SERVICE_MANAGER,
        )
        .unwrap();

    assert_eq!(
        store.provision_identity_from_supervisor(identity(2, 1), principal(20), Roles::NONE),
        Err(AuthorizationError::IdentityCapacity)
    );
    let replacement = identity(1, 2);
    let replacement_principal = principal(30);
    store
        .provision_identity_from_supervisor(
            replacement,
            replacement_principal,
            Roles::POLICY_ADMIN | Roles::SERVICE_MANAGER,
        )
        .unwrap();
    assert_eq!(store.principal_for(admin), None);
    assert_eq!(store.principal_for(replacement), Some(replacement_principal));
    assert_eq!(
        store.publish_service(replacement, b"longer", Rights::CALL),
        Err(AuthorizationError::InvalidService)
    );
    store.publish_service(replacement, b"svc", Rights::CALL).unwrap();
    assert_eq!(
        store.publish_service(replacement, b"next", Rights::CALL),
        Err(AuthorizationError::ServiceCapacity)
    );
    store.set_policy(replacement, replacement_principal, b"svc", Rights::CALL, 0).unwrap();
    assert_eq!(
        store.set_policy(replacement, replacement_principal, b"next", Rights::CALL, 0),
        Err(AuthorizationError::PolicyCapacity)
    );
    store.issue_ticket(replacement, b"svc", Rights::CALL).unwrap();
    assert_eq!(
        store.issue_ticket(replacement, b"svc", Rights::CALL),
        Err(AuthorizationError::TicketCapacity)
    );
}

#[test]
fn delayed_identity_provisioning_cannot_replace_a_newer_generation() {
    let mut store = PolicyStore::new();
    let newest = identity(7, 3);
    let newest_principal = principal(70);
    store.provision_identity_from_supervisor(newest, newest_principal, Roles::NONE).unwrap();

    assert_eq!(
        store.provision_identity_from_supervisor(identity(7, 2), principal(60), Roles::NONE),
        Err(AuthorizationError::StaleIdentity)
    );
    assert_eq!(store.principal_for(newest), Some(newest_principal));
}

#[test]
fn exact_identity_reprovisioning_is_idempotent_but_cannot_change_authority() {
    let mut store = PolicyStore::new();
    let domain = identity(7, 3);
    let assigned_principal = principal(70);
    store.provision_identity_from_supervisor(domain, assigned_principal, Roles::NONE).unwrap();

    store.provision_identity_from_supervisor(domain, assigned_principal, Roles::NONE).unwrap();
    assert_eq!(
        store.provision_identity_from_supervisor(domain, principal(71), Roles::POLICY_ADMIN),
        Err(AuthorizationError::IdentityConflict)
    );
    assert_eq!(store.principal_for(domain), Some(assigned_principal));
    assert_eq!(store.roles_for(domain), Some(Roles::NONE));
}

#[test]
fn administrator_can_resolve_for_an_exact_unprivileged_target() {
    let mut fixture = fixture();
    let target = identity(9, 4);
    let target_principal = principal(90);
    fixture.store.set_policy(fixture.admin, target_principal, SERVICE, Rights::CALL, 0).unwrap();

    let grant = fixture
        .store
        .authorize_for_admin(fixture.admin, target, target_principal, SERVICE, Rights::CALL)
        .unwrap();
    assert_eq!(grant.principal, target_principal);
    assert_eq!(fixture.store.roles_for(target), Some(Roles::NONE));
    assert_eq!(
        fixture.store.authorize_for_admin(
            fixture.client,
            identity(10, 1),
            principal(100),
            SERVICE,
            Rights::CALL,
        ),
        Err(AuthorizationError::PolicyAdministratorRequired)
    );
}

#[test]
fn audit_log_is_bounded_and_preserves_monotonic_sequence() {
    let mut log = AuditLog::new(2).unwrap();
    let caller = identity(2, 1);
    for requested in [Rights::SEND, Rights::CALL, Rights::CLIENT] {
        log.record(
            caller,
            Some(principal(20)),
            SERVICE,
            requested,
            requested,
            4,
            8,
            AuditOutcome::Issued,
        )
        .unwrap();
    }
    let records = log.iter().collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].sequence, 2);
    assert_eq!(records[1].sequence, 3);
}

#[test]
fn policy_wire_requests_round_trip_and_exclude_caller_identity() {
    use charlotte_authorization::wire::{
        Request,
        decode,
        encode_grant_lookup,
        encode_lookup,
        encode_publish,
        encode_set_policy,
    };

    let mut bytes = [0u8; 512];
    let len = encode_lookup(SERVICE, Rights::CALL, &mut bytes).unwrap();
    assert_eq!(
        decode(&bytes[..len]),
        Some(Request::Lookup {
            service: SERVICE,
            requested: Rights::CALL,
        })
    );

    let len = encode_set_policy(SERVICE, 20, Rights::CLIENT, 7, &mut bytes).unwrap();
    assert_eq!(
        decode(&bytes[..len]),
        Some(Request::SetPolicy {
            service: SERVICE,
            subject: 20,
            allowed: Rights::CLIENT,
            expected_version: 7,
        })
    );

    let len = encode_publish(SERVICE, Rights::SEND, &mut bytes).unwrap();
    assert_eq!(
        decode(&bytes[..len]),
        Some(Request::Publish {
            service: SERVICE,
            ceiling: Rights::SEND,
        })
    );
    bytes[12] = 1;
    assert_eq!(decode(&bytes[..len]), None);

    let len = encode_grant_lookup(SERVICE, Rights::CLIENT, 9, 4, 90, &mut bytes).unwrap();
    assert_eq!(
        decode(&bytes[..len]),
        Some(Request::GrantLookup {
            service: SERVICE,
            requested: Rights::CLIENT,
            target_asid: 9,
            target_generation: 4,
            target_principal: 90,
        })
    );
}
