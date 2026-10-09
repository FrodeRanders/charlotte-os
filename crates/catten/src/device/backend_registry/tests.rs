//! Serialized metadata probes; hardware completion stays with typed backends.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    device::*,
    memory,
};

static REJECT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static IRQ: AtomicBool = AtomicBool::new(false);
static PREPARED: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicUsize = AtomicUsize::new(0);
static DROPS: AtomicUsize = AtomicUsize::new(0);

pub(in crate::device) fn reject_next(stage: usize) {
    REJECT.store(stage, Ordering::Release);
}
pub(super) fn reject(stage: usize) -> bool {
    REJECT.compare_exchange(stage, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
}
fn available(mut probe: impl FnMut() -> bool, label: &'static str) {
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while !probe() {
        deadline.assert_pending(label);
        core::hint::spin_loop();
    }
}
pub(in crate::device) fn boundary(dispose: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Relaxed));
    dma::test_assert_backend_available();
    available(
        || memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "backend metadata lifecycle",
    );
    available(|| DEVICES.try_lock().is_some(), "backend metadata devices");
    available(|| memory::ADDRESS_SPACE_TABLE.try_lock().is_some(), "backend metadata root table");
    available(|| memory::KERNEL_AS.try_lock().is_some(), "backend metadata kernel table");
    available(
        || memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "backend metadata physical allocator",
    );
    crate::capability::record_tests::assert_local_available();
    if dispose {
        DISPOSED.fetch_add(1, Ordering::Relaxed);
    } else {
        PREPARED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(in crate::device) fn begin_real() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    PREPARED.store(0, Ordering::Relaxed);
    DISPOSED.store(0, Ordering::Relaxed);
}
pub(in crate::device) fn finish_real() {
    ACTIVE.store(false, Ordering::Release);
    crate::logln!(
        "[backend registry phases] {} node-preparation and {} disposal entries outside local \
         backend/lifecycle/device/capability/table/physical/heap guards; entry IRQ state preserved",
        PREPARED.load(Ordering::Acquire),
        DISPOSED.load(Ordering::Acquire)
    );
}
struct Probe;
impl Drop for Probe {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}

#[allow(clippy::drop_non_drop)] // Exercise complete-grant ManuallyDrop abandonment.
pub(in crate::device) fn run() {
    begin_real();
    let stages = if cfg!(target_arch = "x86_64") {
        3
    } else {
        2
    };
    for stage in 1..=stages {
        let mut nodes = Nodes::<Probe, u16>::new(true, cfg!(target_arch = "x86_64"));
        reject_next(stage);
        assert_eq!(nodes.allocate(), Err(Error::MapFailed));
        assert_eq!(REJECT.load(Ordering::Acquire), 0);
        nodes.finish();
    }
    let mut domains = Map::new();
    let mut sources = Map::new();
    #[cfg(target_arch = "x86_64")]
    let mut contexts = Map::new();
    let mut first = Nodes::<Probe, u16>::new(true, cfg!(target_arch = "x86_64"));
    first.allocate().unwrap();
    {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        first.publish_domain(&mut domains, 1, Probe);
        first.publish_source(&mut sources, 3, 1);
        #[cfg(target_arch = "x86_64")]
        first.publish_context(&mut contexts, 0, PAddr::from(4096u64)); // Metadata ABI fixture, no backing.
        let moved = domains.get_mut(&1).unwrap().take().unwrap();
        assert!(!domains.values().all(Option::is_some));
        *domains.get_mut(&1).unwrap() = Some(moved);
        drop(heap);
    }
    first.finish();
    let retired = {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let retired = domains.take(&1).unwrap();
        *sources.get_mut(&3).unwrap() = 0;
        drop(heap);
        retired
    };
    assert_eq!(DROPS.load(Ordering::Acquire), 0);
    release(retired);
    assert_eq!(DROPS.load(Ordering::Acquire), 1);
    let fence = sources.get(&3).unwrap() as *const u64;
    let mut reuse = Nodes::<Probe, u16>::new(false, false);
    reuse.allocate().unwrap();
    {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        reuse.publish_domain(&mut domains, 2, Probe);
        reuse.publish_source(&mut sources, 3, 2);
        assert_eq!(sources.get(&3).unwrap() as *const u64, fence);
        drop(heap);
    }
    reuse.finish();
    release(domains.take(&2).unwrap());
    release(sources.take(&3).unwrap());
    #[cfg(target_arch = "x86_64")]
    release(contexts.take(&0).unwrap());
    assert_eq!(DROPS.load(Ordering::Acquire), 2);
    finish_real();

    // Complete grant fallback owns partially admitted backend metadata even
    // before reset, ID mutation or physical domain backing exists.
    let root = crate::service::loader::create_user_address_space_handle();
    let mut grant =
        PreparedDmaDomain::new(root.id(), |_| panic!("metadata-only abandonment reached hardware"))
            .unwrap();
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        grant.resources.admission.reserve(&lifecycle).unwrap();
    }
    #[cfg(target_arch = "x86_64")]
    {
        grant.resources.creation.metadata = Some(
            if crate::environment::acpi::sdt::dmar::discover_vtd().is_some() {
                Preparing::Vtd(Nodes::new(true, true))
            } else {
                Preparing::AmdVi(Nodes::new(true, false))
            },
        );
    }
    #[cfg(target_arch = "aarch64")]
    {
        grant.resources.creation.metadata = Some(Preparing::Smmu(Nodes::new(true, false)));
    }
    grant.resources.creation.metadata.as_mut().unwrap().allocate().unwrap();
    assert!(grant.resources.creation.is_armed());
    assert_eq!(
        dma::create_domain_with_reset(u32::MAX, None, &mut grant.resources.creation, |_, _| {
            panic!("metadata-armed grant reached reset")
        }),
        Err(Error::OperationInFlight)
    );
    let before = dma_tables::used();
    dma_tables::test_drop_under_guards(|| {
        let _devices = DEVICES.lock();
        drop(grant);
    });
    assert_eq!(dma_tables::used(), before);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    crate::logln!(
        "[backend registry ownership] node rejection, heap-held publication/claimed-cell \
         restoration/detach and exact zero-fence reuse passed; guarded complete-grant abandonment \
         retains one root/reservation and unused backend nodes without new IOMMU table/data charge"
    );
}
