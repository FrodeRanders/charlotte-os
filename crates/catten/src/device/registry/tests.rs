//! Serialized metadata rejection/relink/disposal probes, never caller policy.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
};

use super::*;
static REJECT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static IRQ: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicUsize = AtomicUsize::new(0);

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
pub(super) fn boundary(dispose: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Relaxed));
    dma::test_assert_backend_available();
    available(|| DEVICES.try_lock().is_some(), "device metadata registry");
    available(
        || crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "device metadata lifecycle",
    );
    available(
        || crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some(),
        "device metadata root table",
    );
    available(|| crate::memory::KERNEL_AS.try_lock().is_some(), "device metadata kernel table");
    available(
        || crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "device metadata physical allocator",
    );
    available(
        || crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some(),
        "device metadata heap",
    );
    if dispose {
        DISPOSED.fetch_add(1, Ordering::Relaxed);
    } else {
        ALLOCATED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(in crate::device) fn begin_real() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    ALLOCATED.store(0, Ordering::Relaxed);
    DISPOSED.store(0, Ordering::Relaxed);
}
pub(in crate::device) fn finish_real() {
    ACTIVE.store(false, Ordering::Release);
    let allocated = ALLOCATED.load(Ordering::Acquire);
    let disposed = DISPOSED.load(Ordering::Acquire);
    assert!(allocated > 0 && disposed > 0);
    crate::logln!(
        "[device registry phases] {} allocated-node and {} disposal boundaries outside local \
         lifecycle/device/backend/table/physical/heap guards; entry IRQ state preserved",
        allocated,
        disposed
    );
}

pub(in crate::device) fn run() {
    begin_real();
    let root = crate::service::loader::create_user_address_space_handle();
    for kind in 0..3 {
        for stage in [1, 2] {
            REJECT.store(stage, Ordering::Release);
            let result = match kind {
                0 => grant_mmio(root.id(), 0x0900_0000, 1),
                1 => grant_interrupt(root.id(), 225),
                _ => grant_dma_domain_with_backend(
                    root.id(),
                    |_| panic!("metadata rejection reached hardware"),
                    |_| panic!("unstarted metadata rejection destroyed hardware"),
                ),
            };
            assert_eq!(result, Err(DeviceError::ResourceLimit));
            assert_eq!(REJECT.load(Ordering::Acquire), 0);
            assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 0);
            assert!(!DEVICES.lock().contains_key(&root.id()));
        }
    }
    for kind in 0..3 {
        let mut admission = GrantAdmission::new(root.id()).unwrap();
        let lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        admission.reserve(&lifecycle).unwrap();
        let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let mut devices = DEVICES.lock();
        let object = match kind {
            0 => DeviceObject::Mmio(MmioRegion {
                phys_base: 0x0900_0000,
                pages: 1,
                mapped: None,
                scratch_mapped: false,
                operation_in_flight: false,
            }),
            1 => DeviceObject::Interrupt(InterruptObject {
                intid: 225,
                cq: None,
                target_lp: 0,
            }),
            _ => DeviceObject::DmaDomain {
                id: u64::MAX,
                operation_in_flight: false,
            }, // ABI fixture, no hardware backing.
        };
        let cap = admission.publish(&mut devices, object).unwrap();
        // The original namespace node is reused after the first publication.
        assert!(devices.get(&root.id()).unwrap().caps.contains_key(&cap));
        drop(devices);
        drop(heap);
        drop(lifecycle);
        admission.finish().unwrap();
        close_cap(root.id(), cap).unwrap();
        assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 0);
    }
    crate::memory::close_user_address_space_handle(root).unwrap();
    finish_real();
    crate::logln!(
        "[device registry ownership] first/second node rejection precedes all MMIO/IRQ/DMA \
         authority/hardware; heap-held namespace creation/reuse and capability publication; \
         explicit post-guard node disposal and exact-root completion passed"
    );
}
