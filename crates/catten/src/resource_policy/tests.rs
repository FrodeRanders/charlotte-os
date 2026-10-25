use super::*;

// Exercise the actual counter implementation, not a duplicate policy model.
#[path = "../../../charlotte-lifecycle/src/resources.rs"]
mod resources;

fn exhaust<const N: usize>(pool: Pool<N>) {
    let mut admission = resources::AdmissionPool::new(pool.total, pool.ordinary);
    admission.reserve(false, pool.ordinary).unwrap();
    assert_eq!(admission.used(), (pool.ordinary, pool.ordinary));
    let before = admission;
    assert!(admission.reserve(false, [1; N]).is_err());
    assert_eq!(admission, before);
    let mut reserve = [0; N];
    for (i, value) in reserve.iter_mut().enumerate() {
        *value = pool.total[i] - pool.ordinary[i];
    }
    admission.reserve(true, reserve).unwrap();
    assert_eq!(admission.used(), (pool.total, pool.ordinary));
    let before = admission;
    assert!(admission.reserve(true, [1; N]).is_err());
    assert_eq!(admission, before);
    admission.release(false, pool.ordinary).unwrap();
    assert_eq!(admission.used(), (reserve, [0; N]));
    admission.release(true, reserve).unwrap();
    assert_eq!(admission.used(), ([0; N], [0; N]));
}
fn exercise(p: Policy) {
    assert_eq!(p.validate(), Ok(()));
    for pool in [
        p.capabilities.pool,
        p.completions.pool,
        p.timers.pool,
        p.watches.pool,
        p.waiters.pool,
        p.observer_lists,
        p.mailbox_records.pool,
        p.memory.objects.pool,
    ] {
        exhaust(pool);
    }
    for pool in [
        p.mailbox_queues.pool,
        p.endpoints.pool,
        p.completion_queues.pool,
        p.root_metadata,
        p.principal_history,
        p.thread_metadata,
        p.scratch_nodes,
        p.snapshot_buffers,
        p.mapping_nodes,
        p.memory_capability_nodes,
    ] {
        exhaust(pool);
    }
    exhaust(p.ipc_records.pool);
}
#[test]
fn default_selected_and_larger_profiles_preserve_counter_admission() {
    exercise(DEFAULT);
    exercise(POLICY);
    exercise(larger::POLICY);
    assert!(larger::POLICY.mapping_nodes.total[1] > DEFAULT.mapping_nodes.total[1]);
}
#[test]
fn invalid_ordering_zero_and_machine_overflow_reject() {
    for pool in [
        Pool {
            total: [10],
            ordinary: [10],
        },
        Pool {
            total: [10],
            ordinary: [0],
        },
        Pool {
            total: [u64::MAX],
            ordinary: [1],
        },
    ] {
        let mut p = DEFAULT;
        p.observer_lists = pool;
        assert_eq!(p.validate(), Err("domain/ordinary/total ordering or machine range"));
    }
    let mut p = DEFAULT;
    p.capabilities.domain[0] = p.capabilities.pool.ordinary[0] + 1;
    assert!(p.validate().is_err());
}
#[test]
fn cross_family_capacity_dependencies_reject() {
    let mut p = DEFAULT;
    p.snapshot_buffers.ordinary[0] = p.memory.object_pages * 8 - 1;
    assert_eq!(p.validate(), Err("object snapshot capacity"));
    let mut p = DEFAULT;
    p.memory.principal_thread_metadata[1] = p.thread_metadata.ordinary[1] + 1;
    assert_eq!(p.validate(), Err("principal thread metadata"));
    let mut p = DEFAULT;
    p.local.endpoint_capacity = p.endpoints.domain[1] as usize + 1;
    assert_eq!(p.validate(), Err("local source/queue ceilings"));
    let mut p = DEFAULT;
    p.local.watches_per_endpoint = p.watches.domain[0] as usize + 1;
    assert_eq!(p.validate(), Err("local source/queue ceilings"));
    let mut p = DEFAULT;
    p.memory.principal_pages = p.memory.domain_object_pages - 1;
    assert_eq!(p.validate(), Err("backing ceilings"));
}
#[test]
fn ratios_and_heap_layouts_reject_invalid_or_overflowing_policy() {
    for ratio in [
        Ratio {
            numerator: 1,
            denominator: 0,
        },
        Ratio {
            numerator: 0,
            denominator: 4,
        },
        Ratio {
            numerator: 4,
            denominator: 4,
        },
    ] {
        let mut p = DEFAULT;
        p.memory.ordinary_share = ratio;
        assert_eq!(p.validate(), Err("RAM ratio"));
    }
    for h in [
        KernelHeap {
            ram_divisor: 0,
            ..DEFAULT.kernel_heap
        },
        KernelHeap {
            growth_min_bytes: DEFAULT.kernel_heap.growth_max_bytes + 2 * 1024 * 1024,
            ..DEFAULT.kernel_heap
        },
        KernelHeap {
            initial_bytes: usize::MAX,
            ..DEFAULT.kernel_heap
        },
        KernelHeap {
            growth_max_bytes: 1,
            ..DEFAULT.kernel_heap
        },
    ] {
        let mut p = DEFAULT;
        p.kernel_heap = h;
        assert_eq!(p.validate(), Err("kernel heap layout"));
    }
}
#[test]
fn discovered_ram_rounding_and_reserve_match_previous_defaults() {
    for value in [0, 1, 3, 7, 8, 9, 63, 64, 65, 1_000_003, u64::MAX] {
        assert_eq!(DEFAULT.memory.object_ram.of(value), value / 4);
        assert_eq!(DEFAULT.memory.stack_ram.of(value), value / 8);
        assert_eq!(DEFAULT.memory.dma_ram.of(value), value / 32);
        assert_eq!(DEFAULT.memory.ordinary_share.of(value), (value as u128 * 3 / 4) as u64);
    }
    for free in 0..=80 {
        // Same default physical floor as the lifecycle counter model.
        assert_eq!(
            DEFAULT.memory.frames_available(free, 80, 1),
            resources::frames_available(free, 80, 1)
        );
    }
    assert!(!frames_available(0, 0, 1));
}

#[test]
fn heap_reserve_clamps_before_rounding_at_extreme_ram() {
    for divisor in [1, 2, 64, u64::MAX] {
        let heap = KernelHeap {
            ram_divisor: divisor,
            ..DEFAULT.kernel_heap
        };
        for ram in [0, 1, 64 * 1024 * 1024, u64::MAX] {
            let bytes = heap.growth_reserve_bytes(ram);
            assert!((heap.growth_min_bytes..=heap.growth_max_bytes).contains(&bytes));
            assert!(bytes.is_multiple_of(2 * 1024 * 1024));
        }
    }
    assert_eq!(
        KernelHeap {
            ram_divisor: 1,
            ..DEFAULT.kernel_heap
        }
        .growth_reserve_bytes(u64::MAX),
        DEFAULT.kernel_heap.growth_max_bytes
    );
}
