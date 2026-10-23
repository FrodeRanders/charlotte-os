//! Serialized boot controls for existing root-publication registries.
use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

pub(super) fn run() {
    rejection();
    let mut limits = DomainLimitsTable::new();
    let mut usage = usage::DomainUsageTable::new();
    let mut authority = DomainAuthoritiesTable::new();
    for generation in 1..=64 {
        let prepared = PreparingMetadata::try_new(false).unwrap();
        let handle = AddressSpaceHandle {
            id: 1,
            generation,
        };
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            prepared.publish_into(&mut limits, &mut usage, &mut authority, handle);
            assert_eq!(limits.get(&1).unwrap().0, handle);
            assert_eq!(usage.get(&1).unwrap().0, handle);
            assert_eq!(authority.get(&1).unwrap().handle, handle);
            assert!(authority.get(&1).unwrap().authority.is_none());
        }
        limits.take(&1).unwrap().release();
        usage.take(&1).unwrap().release();
        authority.take(&1).unwrap().release_with(|(_, slot)| slot.history.cancel_unpublished());
    }
    assert!(limits.iter().next().is_none());
    assert!(usage.iter().next().is_none());
    assert!(authority.iter().next().is_none());
    generation_reuse();
    authority_generation_reuse();
    usage::test_principal_history();

    // Partial preparation fallback; complete fallback is tested by the parent
    // registration owner. Retain ten empty nodes across four partial phases,
    // without publishing a root or principal.
    for stage in 1..=4 {
        let mut owner = PreparingMetadata(ManuallyDrop::new(Resources {
            limits: Some(PreparedEntry::try_new().unwrap()),
            usage: None,
            authority: None,
            history: None,
        }));
        if stage >= 2 {
            owner.0.usage = Some(PreparedEntry::try_new().unwrap());
        }
        if stage >= 3 {
            owner.0.authority = Some(PreparedEntry::try_new().unwrap());
        }
        if stage == 4 {
            owner.0.history = Some(usage::PreparingHistory::try_new(false).unwrap());
        }
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _limits = DOMAIN_LIMITS.lock();
        let _usage = usage::DOMAIN_USAGE.lock();
        let _authority = DOMAIN_AUTHORITIES.lock();
        usage::with_principal_history_locked(|| {
            let _heap = PRIMARY_ALLOCATOR.lock();
            drop(owner);
        });
    }
    crate::logln!(
        "[root policy preparation] 320 phase rejections, 64 heap-held publications, exact \
         generation reuse and guarded partial retention passed; ten standalone empty nodes \
         retained"
    );
}

// The root stays inline on rejection; no failure-path boxing.
#[allow(clippy::result_large_err)]
fn rejection() {
    for reject in 1..=5 {
        for _ in 0..64 {
            let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
            let roots = ADDRESS_SPACE_TABLE.lock().iter().filter(|e| e.is_some()).count();
            let capabilities = crate::capability::node_admission_used();
            let limits = DOMAIN_LIMITS.lock().iter().count();
            let usage = usage::DOMAIN_USAGE.lock().iter().count();
            let authorities = DOMAIN_AUTHORITIES.lock().iter().count();
            let root = AddressSpace::try_new_user().unwrap();
            let mut visits = 0;
            assert_eq!(
                super::super::register(
                    root,
                    |_, _| panic!("metadata rejection published root"),
                    |root| {
                        assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                        assert!(DOMAIN_LIMITS.try_lock().is_some());
                        assert!(usage::DOMAIN_USAGE.try_lock().is_some());
                        assert!(DOMAIN_AUTHORITIES.try_lock().is_some());
                        assert!(usage::principal_history_available());
                        assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                        assert!(budget::tests::ledger_available());
                        drop(root);
                    },
                    budget::PreparingAccount::try_new,
                    || PreparingMetadata::prepare(false, |stage| {
                        visits += 1;
                        assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                        assert!(DOMAIN_LIMITS.try_lock().is_some());
                        assert!(usage::DOMAIN_USAGE.try_lock().is_some());
                        assert!(DOMAIN_AUTHORITIES.try_lock().is_some());
                        assert!(usage::principal_history_available());
                        assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                        if stage == reject {
                            Err(AllocError)
                        } else {
                            Ok(())
                        }
                    }),
                ),
                Err(AddressSpaceRegistrationError::DomainMetadataAllocationFailed)
            );
            assert_eq!(visits, reject);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
            assert_eq!(ADDRESS_SPACE_TABLE.lock().iter().filter(|e| e.is_some()).count(), roots);
            assert_eq!(crate::capability::node_admission_used(), capabilities);
            assert_eq!(DOMAIN_LIMITS.lock().iter().count(), limits);
            assert_eq!(usage::DOMAIN_USAGE.lock().iter().count(), usage);
            assert_eq!(DOMAIN_AUTHORITIES.lock().iter().count(), authorities);
        }
    }
}

fn authority_generation_reuse() {
    let first = register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(domain_authority(first.id()), None);
    let principal = charlotte_launch::artifact_principal_id(b"authority-slot-control");
    let roles = catten_syscall::domain_roles::SERVICE_MANAGER;
    let descriptor = b"authority-slot descriptor binding control";
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        register_domain_authority(first, principal, roles).unwrap();
        install_launch_descriptor(first, descriptor);
        let authority = domain_authority(first.id()).unwrap();
        assert_eq!(authority.address_space, first);
        assert_eq!(authority.principal, principal);
        assert_eq!(authority.roles, roles);
        assert_eq!(
            authority.launch_descriptor_digest,
            Some(charlotte_launch::sha256::digest(descriptor))
        );
    }
    {
        let mut table = DOMAIN_AUTHORITIES.lock();
        let slot = table.get_mut(&first.id()).unwrap();
        let original = slot.authority;
        assert_eq!(slot.install(first, principal, 0), Err(DomainAuthorityError::AlreadyInstalled));
        assert_eq!(slot.authority, original);
    }
    close_user_address_space_handle(first).unwrap();
    assert_eq!(domain_authority(first.id()), None);
    let next = register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(next.id(), first.id());
    assert_eq!(next.generation(), first.generation() + 1);
    assert_eq!(domain_authority(next.id()), None);
    {
        let mut table = DOMAIN_AUTHORITIES.lock();
        let slot = table.get_mut(&next.id()).unwrap();
        assert_eq!(slot.install(first, principal, roles), Err(DomainAuthorityError::StaleHandle));
        assert_eq!(slot.install(next, 0, roles), Err(DomainAuthorityError::ZeroPrincipal));
        assert!(slot.authority.is_none());
    }
    close_user_address_space_handle(next).unwrap();
    crate::logln!(
        "[root authority slot] empty grants no authority; heap-held signed-identity/descriptor \
         installation, duplicate/zero/stale rejection and exact generation reuse passed"
    );
}

fn generation_reuse() {
    let first = register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    set_domain_limits(
        first,
        DomainLimits {
            user_stack_pages: 1,
            max_threads: 1,
        },
    )
    .unwrap();
    usage::note_owned_frame(first.id());
    usage::note_thread_created(first.id(), 3);
    assert_eq!(usage::domain_usage(first.id()).unwrap().threads, 1);
    close_user_address_space_handle(first).unwrap();
    assert!(!DOMAIN_LIMITS.lock().contains_key(&first.id()));
    assert!(!usage::DOMAIN_USAGE.lock().contains_key(&first.id()));
    let next = register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(next.id(), first.id());
    assert_eq!(next.generation(), first.generation() + 1);
    assert_eq!(domain_limits(next.id()).user_stack_pages, DomainLimits::default().user_stack_pages);
    assert_eq!(domain_limits(next.id()).max_threads, DomainLimits::default().max_threads);
    assert_eq!(usage::domain_usage(next.id()), Some(usage::DomainUsageSnapshot::default()));
    assert_eq!(
        set_domain_limits(first, DomainLimits::default()),
        Err(DomainLimitError::StaleAddressSpace)
    );
    close_user_address_space_handle(next).unwrap();
}
