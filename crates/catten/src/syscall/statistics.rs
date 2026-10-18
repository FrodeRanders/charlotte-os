//! Bounded thread/domain telemetry. Capture plain values under registry
//! guards; all fallible vector preparation and disposal leaves those guards.

use super::{
    TrapFrame,
    caller_asid,
    push_u64,
};
use crate::{
    klib::collections::snapshot::{
        Budget,
        Error,
        MAX_WORKING_BYTES,
    },
    memory::object,
};

pub(super) fn capture(frame: &mut TrapFrame) {
    capture_inner(frame, 0);
}

// Private boot-fixture adapter; no caller-controlled fault selector or ABI.
pub(crate) fn test_rejection(frame: &mut TrapFrame, stage: usize) {
    assert!((1..=4).contains(&stage));
    capture_inner(frame, stage);
}

fn encoded_len(threads: usize, domains: usize) -> Result<usize, Error> {
    use catten_syscall::{
        THREAD_STATISTICS_DOMAIN_RECORD_U64S,
        THREAD_STATISTICS_HEADER_U64S,
        THREAD_STATISTICS_RECORD_U64S,
    };
    threads
        .checked_mul(THREAD_STATISTICS_RECORD_U64S)
        .and_then(|words| {
            domains
                .checked_mul(THREAD_STATISTICS_DOMAIN_RECORD_U64S)
                .and_then(|domain_words| words.checked_add(domain_words))
        })
        .and_then(|words| words.checked_add(THREAD_STATISTICS_HEADER_U64S))
        .and_then(|words| words.checked_mul(core::mem::size_of::<u64>()))
        .ok_or(Error::Capacity)
}

fn capture_inner(frame: &mut TrapFrame, reject: usize) {
    use catten_syscall::{
        OBSERVABILITY_NONE,
        THREAD_STATISTICS_DOMAIN_RECORD_U64S,
        THREAD_STATISTICS_HEADER_U64S,
        THREAD_STATISTICS_MAGIC,
        THREAD_STATISTICS_RECORD_U64S,
        THREAD_STATISTICS_VERSION,
        thread_domain_record as domain_record,
        thread_statistics_header as header,
        thread_statistics_record as record,
    };

    let asid = caller_asid(frame);
    let observer_cap = frame.regs[1];
    let observer = observer_cap != 0
        && crate::capability::contains(
            asid,
            observer_cap,
            crate::capability::ObjectKind::SystemObserver,
        );
    if observer_cap != 0 && !observer {
        frame.regs[0] = 0;
        frame.regs[1] = 0;
        return;
    }
    let scope = (!observer).then_some(asid);
    let prepared = (|| {
        let mut budget = Budget::new(MAX_WORKING_BYTES);
        if reject == 1 {
            return Err(Error::Allocation);
        }
        let snapshots = crate::cpu::scheduler::threads::try_statistics(scope, &mut budget)?;
        if reject == 2 {
            return Err(Error::Allocation);
        }
        let domains = crate::memory::usage::try_domain_usage(scope, &mut budget)?;
        let exact_len = encoded_len(snapshots.len(), domains.len())?;
        if reject == 3 {
            return Err(Error::Allocation);
        }
        let bytes = budget.prepare::<u8>(exact_len)?.into_vec();
        Ok((snapshots, domains, bytes, exact_len))
    })();
    let Ok((snapshots, domains, mut bytes, exact_len)) = prepared else {
        frame.regs[0] = 0;
        frame.regs[1] = 0;
        return;
    };
    let cpu_busy_ticks = if observer {
        crate::cpu::scheduler::threads::cpu_busy_ticks()
    } else {
        snapshots
            .iter()
            .fold(0u128, |sum, snapshot| sum.saturating_add(snapshot.runtime_ticks.total))
    };
    let mut header_words = [0; THREAD_STATISTICS_HEADER_U64S];
    header_words[header::MAGIC] = THREAD_STATISTICS_MAGIC;
    header_words[header::VERSION] = THREAD_STATISTICS_VERSION;
    header_words[header::LOGICAL_PROCESSORS] = crate::cpu::multiprocessor::get_lp_count() as u64;
    header_words[header::CPU_BUSY_TICKS] = u64::try_from(cpu_busy_ticks).unwrap_or(u64::MAX);
    header_words[header::RECORD_BYTES] =
        (THREAD_STATISTICS_RECORD_U64S * core::mem::size_of::<u64>()) as u64;
    header_words[header::RECORD_COUNT] = snapshots.len() as u64;
    header_words[header::COUNTER_FREQUENCY_HZ] = crate::cpu::scheduler::counter_frequency_hz();
    header_words[header::MONOTONIC_TICKS] = crate::cpu::scheduler::monotonic_ticks();
    {
        let allocator = crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock();
        header_words[header::FREE_FRAMES] = allocator.free_frames() as u64;
        header_words[header::USABLE_FRAMES] =
            allocator.usable_bytes() / crate::cpu::isa::memory::paging::PAGE_SIZE as u64;
    }
    header_words[header::DOMAIN_RECORD_BYTES] =
        (THREAD_STATISTICS_DOMAIN_RECORD_U64S * core::mem::size_of::<u64>()) as u64;
    header_words[header::DOMAIN_RECORD_COUNT] = domains.len() as u64;
    let recovery = crate::memory::retirement::recovery::status_words(observer);
    header_words[header::ROOT_RECOVERY_AWAITING..=header::ROOT_RECOVERY_CAPACITY]
        .copy_from_slice(&recovery);
    for value in header_words {
        push_u64(&mut bytes, value);
    }
    for snapshot in snapshots {
        let statistics = snapshot.runtime_ticks;
        let mut record_words = [0; THREAD_STATISTICS_RECORD_U64S];
        record_words[record::TID] = snapshot.tid as u64;
        record_words[record::GENERATION] = snapshot.generation;
        record_words[record::ASID] = snapshot.asid as u64;
        record_words[record::STATE] = snapshot.state as u64;
        record_words[record::AFFINITY_LP] =
            snapshot.affinity_lp.map_or(OBSERVABILITY_NONE, u64::from);
        record_words[record::PINNED_LP] = snapshot.pinned_lp.map_or(OBSERVABILITY_NONE, u64::from);
        record_words[record::DISPATCH_COUNT] = snapshot.dispatch_count;
        record_words[record::SAMPLE_COUNT] = statistics.count;
        record_words[record::MIN_TICKS] = statistics.min.unwrap_or(OBSERVABILITY_NONE);
        record_words[record::MAX_TICKS] = statistics.max.unwrap_or(OBSERVABILITY_NONE);
        record_words[record::TOTAL_TICKS_LOW] = statistics.total as u64;
        record_words[record::TOTAL_TICKS_HIGH] = (statistics.total >> 64) as u64;
        record_words[record::SUM_OF_SQUARES_LOW] = statistics.sum_of_squares as u64;
        record_words[record::SUM_OF_SQUARES_HIGH] = (statistics.sum_of_squares >> 64) as u64;
        record_words[record::SATURATED] = u64::from(statistics.saturated);
        record_words[record::CURRENT_SLICE_STARTED_AT] =
            snapshot.current_slice_started_at.unwrap_or(OBSERVABILITY_NONE);
        record_words[record::STACK_RESERVED_PAGES] = snapshot.stack_reserved_pages;
        record_words[record::STACK_COMMITTED_PAGES] = snapshot.stack_committed_pages;
        record_words[record::STACK_USED_PAGES] = snapshot.stack_used_pages;
        for value in record_words {
            push_u64(&mut bytes, value);
        }
    }
    for (domain, usage) in domains {
        let domain_asid = domain.id();
        let mut record_words = [0; THREAD_STATISTICS_DOMAIN_RECORD_U64S];
        record_words[domain_record::ASID] = domain_asid as u64;
        record_words[domain_record::OWNED_FRAMES] = usage.owned_frames;
        record_words[domain_record::USER_STACK_PAGES] = usage.user_stack_pages;
        record_words[domain_record::USER_STACK_PAGES_HIGH_WATER] =
            usage.user_stack_pages_high_water;
        record_words[domain_record::STACK_PAGES_USED_HIGH_WATER] =
            usage.stack_pages_used_high_water;
        record_words[domain_record::THREADS] = usage.threads;
        record_words[domain_record::THREADS_HIGH_WATER] = usage.threads_high_water;
        if let Some(heap) = crate::memory::usage::domain_heap_status(domain) {
            record_words[domain_record::HEAP_STATUS_VALID] = 1;
            record_words[domain_record::HEAP_CAPACITY_BYTES] = heap.capacity_bytes;
            record_words[domain_record::HEAP_ALLOCATED_BYTES] = heap.allocated_bytes;
            record_words[domain_record::HEAP_PEAK_BYTES] = heap.peak_bytes;
            record_words[domain_record::HEAP_ALLOCATIONS] = heap.allocations;
            record_words[domain_record::HEAP_TOTAL_ALLOCATED_BYTES] = heap.total_allocated_bytes;
            record_words[domain_record::HEAP_LOCK_SPINS] = heap.lock_spins;
        }
        for value in record_words {
            push_u64(&mut bytes, value);
        }
    }
    debug_assert_eq!(bytes.len(), exact_len);
    // ABI publication: no output capability exists during fallible scratch
    // preparation or heap observation. The syscall transfers it once on success.
    let Ok(cap) = object::allocate(asid, exact_len.div_ceil(4096)) else {
        frame.regs[0] = 0;
        frame.regs[1] = 0;
        return;
    };
    if reject == 4 || object::write_bytes(asid, cap, &bytes).is_err() {
        let _ = object::close_cap(asid, cap);
        frame.regs[0] = 0;
        frame.regs[1] = 0;
        return;
    }
    frame.regs[0] = cap;
    frame.regs[1] = exact_len as u64;
}

pub(crate) fn test_capture(asid: crate::memory::AddressSpaceId) {
    use crate::{
        capability::admission_tests::test_namespace_used,
        memory::{
            self,
            allocators::global_allocator::PRIMARY_ALLOCATOR,
        },
    };
    let caps = test_namespace_used(asid);
    let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        for stage in 1..=4 {
            let mut frame = TrapFrame {
                regs: [0; 19],
                elr_el1: 0,
                spsr_el1: 0,
                sp_el0: 0,
                lp_id: 0,
                asid,
            };
            frame.regs[0] = u64::MAX;
            test_rejection(&mut frame, stage);
            assert_eq!((frame.regs[0], frame.regs[1]), (0, 0));
            assert_eq!(test_namespace_used(asid), caps);
            assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        }
    }
    assert_eq!(encoded_len(usize::MAX, 1), Err(Error::Capacity));
    assert_eq!(encoded_len(1, usize::MAX), Err(Error::Capacity));
    let mut budget = Budget::new(MAX_WORKING_BYTES);
    let threads = crate::cpu::scheduler::threads::try_statistics(None, &mut budget).unwrap();
    let domains = memory::usage::try_domain_usage(None, &mut budget).unwrap();
    let mut thread_output = budget.prepare(threads.len()).unwrap();
    let mut domain_output = budget.prepare(domains.len()).unwrap();
    let mut empty_domain = budget.prepare(0).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        crate::cpu::scheduler::threads::capture_statistics(None, &mut thread_output).unwrap();
        memory::usage::capture_domain_usage(None, &mut domain_output).unwrap();
        assert_eq!(
            memory::usage::capture_domain_usage(Some(asid), &mut empty_domain),
            Err(Error::SourceChanged)
        );
    }
    assert_eq!(thread_output.into_vec(), threads);
    assert_eq!(domain_output.into_vec(), domains);
    memory::usage::test_snapshot_heap();
    crate::logln!(
        "[telemetry snapshot] 256 preparation/publication rejections, heap-held captures, checked \
         sizing and generation-qualified heap reads passed"
    );
}
