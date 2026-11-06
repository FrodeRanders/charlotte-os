use super::*;

#[derive(Debug)]
struct Nodes;
impl Family<2> for Nodes {
    const ORDINARY: [u64; 2] = [3072, 6];
    const TOTAL: [u64; 2] = [4096, 8];

    fn spec() -> &'static Spec<2> {
        static SPEC: Spec<2> = Spec::new(Nodes::TOTAL, Nodes::ORDINARY);
        &SPEC
    }
}
#[derive(Debug)]
struct Bundles;
impl Family<1> for Bundles {
    const ORDINARY: [u64; 1] = [1];
    const TOTAL: [u64; 1] = [2];

    fn spec() -> &'static Spec<1> {
        static SPEC: Spec<1> = Spec::new(Bundles::TOTAL, Bundles::ORDINARY);
        &SPEC
    }
}

#[test]
fn catalog_observation() {
    test_observation();
    visit(|sample| {
        if sample.id == catten_syscall::metadata_pools::Id::ObserverLists {
            let _guard = ObserverLists::spec().state.try_lock().expect("callback under pool guard");
        }
    });
}

#[test]
fn original_class_and_rejection() {
    let charge = Reservation::<Nodes, 2>::reserve(false, Nodes::ORDINARY).unwrap();
    let full = used::<Nodes, 2>();
    assert!(NodeCharge::<Nodes>::reserve(false, 1).is_err());
    assert!(NodeCharge::<Nodes>::reserve(true, 4097).is_err());
    assert_eq!(used::<Nodes, 2>(), full);
    let platform = NodeCharge::<Nodes>::reserve(true, 128).unwrap();
    assert_eq!(used::<Nodes, 2>(), ([3200, 7], [3072, 6]));
    drop(charge);
    assert_eq!(used::<Nodes, 2>(), ([128, 1], [0, 0]));
    drop(platform);
    assert_eq!(used::<Nodes, 2>(), ([0, 0], [0, 0]));
}

#[test]
fn allocation_failure_and_layout_rollback() {
    assert!(prepare_node_with::<(), Nodes>(false, 128, |_| Err(AllocError)).is_err());
    assert_eq!(used::<Nodes, 2>(), ([0, 0], [0, 0]));
    let charge = LayoutCharge::<Nodes>::reserve(false).unwrap();
    let layout = Layout::new::<[u8; 128]>();
    charge.admit_layout(layout).unwrap();
    assert_eq!(used::<Nodes, 2>(), ([128, 1], [128, 1]));
    charge.rollback_layout(layout);
    assert_eq!(used::<Nodes, 2>(), ([0, 1], [0, 1]));
    assert!(charge.admit_layout(Layout::new::<[u8; 4097]>()).is_err());
    drop(charge);
    assert_eq!(used::<Nodes, 2>(), ([0, 0], [0, 0]));
}

#[test]
fn bundle_headroom_and_concurrent_release() {
    let ordinary = BundleCharge::<Bundles>::reserve(false).unwrap();
    assert!(BundleCharge::<Bundles>::reserve(false).is_err());
    let platform = BundleCharge::<Bundles>::reserve(true).unwrap();
    assert!(BundleCharge::<Bundles>::reserve(true).is_err());
    std::thread::scope(|scope| {
        scope.spawn(move || drop(ordinary));
        scope.spawn(move || drop(platform));
    });
    assert_eq!(used::<Bundles, 1>(), ([0], [0]));
}

#[test]
fn funded_node_rejection_returns_original_sponsor_and_family() {
    use alloc::sync::Arc;
    use core::sync::atomic::{
        AtomicU64,
        Ordering,
    };
    #[derive(Clone, Debug)]
    struct Sponsor(Arc<AtomicU64>);
    impl ByteSponsor for Sponsor {
        fn reserve(&self, bytes: u64) -> Result<(), AllocError> {
            self.0
                .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(bytes).filter(|sum| *sum <= 512)
                })
                .map(|_| ())
                .map_err(|_| AllocError)
        }

        fn release(&self, bytes: u64) {
            assert!(self.0.fetch_sub(bytes, Ordering::AcqRel) >= bytes);
        }
    }
    let used_bytes = Arc::new(AtomicU64::new(0));
    let sponsor = Sponsor(used_bytes.clone());
    type Charge = NodeCharge<Nodes, Sponsor>;
    let charge = Charge::with_sponsor(false, 128, sponsor.clone()).unwrap();
    assert_eq!(used_bytes.load(Ordering::Acquire), 128);
    let before = used::<Nodes, 2>();
    assert!(Charge::with_sponsor(false, 513, sponsor.clone()).is_err());
    assert_eq!(used::<Nodes, 2>(), before);
    let full =
        Reservation::<Nodes, 2>::reserve(false, [Nodes::ORDINARY[0] - 128, Nodes::ORDINARY[1] - 1])
            .unwrap();
    assert!(Charge::with_sponsor(false, 64, sponsor).is_err());
    assert_eq!(used_bytes.load(Ordering::Acquire), 128);
    drop(full);
    std::thread::spawn(move || drop(charge)).join().unwrap();
    assert_eq!(used_bytes.load(Ordering::Acquire), 0);
    assert_eq!(used::<Nodes, 2>(), ([0, 0], [0, 0]));
}
