//! Serialized boot controls for existing root-publication registries.
use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

pub(super) fn run() {
    rejection();
    let mut limits = DomainLimitsTable::new();
    let mut usage = usage::DomainUsageTable::new();
    for generation in 1..=64 {
        let prepared = PreparingMetadata::try_new().unwrap();
        let handle = AddressSpaceHandle {
            id: 1,
            generation,
        };
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            prepared.publish_into(&mut limits, &mut usage, handle);
            assert_eq!(limits.get(&1).unwrap().0, handle);
            assert_eq!(usage.get(&1).unwrap().0, handle);
        }
        limits.take(&1).unwrap().release();
        usage.take(&1).unwrap().release();
    }
    assert!(limits.iter().next().is_none());
    assert!(usage.iter().next().is_none());
    generation_reuse();

    // Partial preparation fallback; complete fallback is tested by the parent
    // registration owner. Retain exactly one empty node, with no root/charge.
    let owner = PreparingMetadata(ManuallyDrop::new(Resources {
        limits: Some(PreparedEntry::try_new().unwrap()),
        usage: None,
    }));
    {
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _limits = DOMAIN_LIMITS.lock();
        let _usage = usage::DOMAIN_USAGE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(owner);
    }
    crate::logln!(
        "[root policy preparation] 192 phase rejections, 64 heap-held publications, exact \
         generation reuse and guarded partial retention passed; one standalone empty node retained"
    );
}

// The root stays inline on rejection; no failure-path boxing.
#[allow(clippy::result_large_err)]
fn rejection() {
    for reject in 1..=3 {
        for _ in 0..64 {
            let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
            let roots = ADDRESS_SPACE_TABLE.lock().iter().filter(|e| e.is_some()).count();
            let capabilities = crate::capability::node_admission_used();
            let limits = DOMAIN_LIMITS.lock().iter().count();
            let usage = usage::DOMAIN_USAGE.lock().iter().count();
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
                        assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                        assert!(budget::tests::ledger_available());
                        drop(root);
                    },
                    budget::PreparingAccount::try_new,
                    || PreparingMetadata::prepare(|stage| {
                        visits += 1;
                        assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                        assert!(DOMAIN_LIMITS.try_lock().is_some());
                        assert!(usage::DOMAIN_USAGE.try_lock().is_some());
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
        }
    }
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
