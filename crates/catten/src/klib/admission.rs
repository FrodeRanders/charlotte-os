//! Common backing charges and a fixed read-only pool inventory. Each family has
//! an independent lock; typed owners still decide when disposal permits refund.
use core::{
    alloc::{
        AllocError,
        Layout,
    },
    marker::PhantomData,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use charlotte_lifecycle::resources::AdmissionPool;

use crate::cpu::multiprocessor::spin::mutex::Mutex;
#[path = "admission/byte_allocator.rs"]
mod byte_allocator;
pub(crate) use byte_allocator::{
    ByteAllocator,
    ByteSponsor,
};

/// State is private: observation does not expose mutation or cleanup authority.
pub(crate) struct Spec<const N: usize> {
    state: Mutex<AdmissionPool<N>>,
}
impl<const N: usize> Spec<N> {
    const fn new(total: [u64; N], ordinary: [u64; N]) -> Self {
        Self {
            state: Mutex::new(AdmissionPool::new(total, ordinary)),
        }
    }
}
pub(crate) trait Family<const N: usize>: core::fmt::Debug + 'static {
    const TOTAL: [u64; N];
    const ORDINARY: [u64; N];
    fn spec() -> &'static Spec<N>;
}
macro_rules! families {
    ($(($tag:ident, $field:ident, $n:literal, $first:literal, $second:literal),)*) => {
        $(#[derive(Debug)] pub(crate) struct $tag;
        impl Family<$n> for $tag {
            const TOTAL: [u64; $n] = crate::resource_policy::POLICY.$field.total;
            const ORDINARY: [u64; $n] = crate::resource_policy::POLICY.$field.ordinary;
            fn spec() -> &'static Spec<$n> {
                static SPEC: Spec<$n> = Spec::new(<$tag as Family<$n>>::TOTAL, <$tag as Family<$n>>::ORDINARY);
                &SPEC
            }
        })*
        /// Each sample is coherent within one pool. No cross-pool atomicity;
        /// callback runs after guard release, even for a recursively busy pool.
        pub(crate) fn visit(mut consume: impl FnMut(catten_syscall::metadata_pools::Sample)) {
            $(consume(sample::<$tag, $n>(catten_syscall::metadata_pools::Id::$tag));)*
        }
    };
}
catten_syscall::metadata_pool_catalog!(families);

fn sample<F: Family<N>, const N: usize>(
    id: catten_syscall::metadata_pools::Id,
) -> catten_syscall::metadata_pools::Sample {
    let mut total = [0; 2];
    let mut ordinary = [0; 2];
    total[..N].copy_from_slice(&F::TOTAL);
    ordinary[..N].copy_from_slice(&F::ORDINARY);
    let used = F::spec().state.try_lock().map(|state| {
        let (a, b) = state.used();
        let mut t = [0; 2];
        let mut o = [0; 2];
        t[..N].copy_from_slice(&a);
        o[..N].copy_from_slice(&b);
        (t, o)
    });
    catten_syscall::metadata_pools::Sample {
        id,
        total_limit: total,
        ordinary_limit: ordinary,
        used,
    }
}
fn reserve<F: Family<N>, const N: usize>(
    platform: bool,
    amount: [u64; N],
) -> Result<(), AllocError> {
    F::spec().state.lock().reserve(platform, amount).map_err(|_| AllocError)
}
fn release<F: Family<N>, const N: usize>(platform: bool, amount: [u64; N]) {
    F::spec().state.lock().release(platform, amount).expect("backing admission underflow");
}

#[derive(Debug)]
#[must_use]
pub(crate) struct NodeCharge<F: Family<2>> {
    bytes: u64,
    platform: bool,
    family: PhantomData<F>,
}
impl<F: Family<2>> NodeCharge<F> {
    pub(crate) fn reserve(platform: bool, bytes: u64) -> Result<Self, AllocError> {
        reserve::<F, 2>(platform, [bytes, 1])?;
        Ok(Self {
            bytes,
            platform,
            family: PhantomData,
        })
    }
}
impl<F: Family<2>> Drop for NodeCharge<F> {
    fn drop(&mut self) {
        release::<F, 2>(self.platform, [self.bytes, 1]);
    }
}
pub(crate) fn prepare_node_with<T, F: Family<2>>(
    platform: bool,
    allocation_bytes: usize,
    allocate: impl FnOnce(NodeCharge<F>) -> Result<T, AllocError>,
) -> Result<T, AllocError> {
    let bytes = u64::try_from(allocation_bytes).map_err(|_| AllocError)?;
    allocate(NodeCharge::reserve(platform, bytes)?)
}

#[derive(Debug)]
#[must_use]
pub(crate) struct BundleCharge<F: Family<1>> {
    platform: bool,
    family: PhantomData<F>,
}
impl<F: Family<1>> BundleCharge<F> {
    pub(crate) fn reserve(platform: bool) -> Result<Self, AllocError> {
        reserve::<F, 1>(platform, [1])?;
        Ok(Self {
            platform,
            family: PhantomData,
        })
    }
}
impl<F: Family<1>> Drop for BundleCharge<F> {
    fn drop(&mut self) {
        release::<F, 1>(self.platform, [1]);
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Reservation<F: Family<N>, const N: usize> {
    amount: [u64; N],
    platform: bool,
    family: PhantomData<F>,
}
impl<F: Family<N>, const N: usize> Reservation<F, N> {
    pub(crate) fn reserve(platform: bool, amount: [u64; N]) -> Result<Self, AllocError> {
        reserve::<F, N>(platform, amount)?;
        Ok(Self {
            amount,
            platform,
            family: PhantomData,
        })
    }
}
impl<F: Family<N>, const N: usize> Drop for Reservation<F, N> {
    fn drop(&mut self) {
        release::<F, N>(self.platform, self.amount);
    }
}

#[derive(Debug)]
pub(crate) struct LayoutCharge<F: Family<2>> {
    bytes: AtomicU64,
    platform: bool,
    family: PhantomData<F>,
}
impl<F: Family<2>> LayoutCharge<F> {
    pub(crate) fn reserve(platform: bool) -> Result<Self, AllocError> {
        reserve::<F, 2>(platform, [0, 1])?;
        Ok(Self {
            bytes: AtomicU64::new(0),
            platform,
            family: PhantomData,
        })
    }

    pub(crate) fn admit_layout(&self, layout: Layout) -> Result<(), AllocError> {
        let bytes = u64::try_from(layout.size()).map_err(|_| AllocError)?;
        reserve::<F, 2>(self.platform, [bytes, 0])?;
        assert_eq!(self.bytes.swap(bytes, Ordering::AcqRel), 0);
        Ok(())
    }

    pub(crate) fn rollback_layout(&self, layout: Layout) {
        let bytes = self.bytes.swap(0, Ordering::AcqRel);
        assert_eq!(bytes, layout.size() as u64);
        release::<F, 2>(self.platform, [bytes, 0]);
    }
}
impl<F: Family<2>> Drop for LayoutCharge<F> {
    fn drop(&mut self) {
        release::<F, 2>(self.platform, [self.bytes.load(Ordering::Acquire), 1]);
    }
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn used<F: Family<N>, const N: usize>() -> ([u64; N], [u64; N]) {
    F::spec().state.lock().used()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked<F: Family<N>, const N: usize, R>(work: impl FnOnce() -> R) -> R {
    let _guard = F::spec().state.lock();
    work()
}

/// Serialized boot fixture: observation takes no allocation and never waits
/// behind the observed pool. Callback entry must follow guard release.
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_observation() {
    use catten_syscall::metadata_pools::{
        ALL,
        Id,
    };
    assert_eq!(core::mem::size_of::<NodeCharge<RootMetadata>>(), 16);
    assert_eq!(core::mem::size_of::<BundleCharge<WaiterAccounts>>(), 1);
    assert_eq!(core::mem::size_of::<Reservation<RootTable, 2>>(), 24);
    assert_eq!(core::mem::size_of::<LayoutCharge<ObserverBacking>>(), 16);
    with_locked::<RootMetadata, 2, _>(|| {
        let mut count = 0;
        visit(|sample| {
            assert_eq!(sample.id, ALL[count]);
            if sample.id == Id::RootMetadata {
                assert!(sample.used.is_none());
            }
            count += 1;
        });
        assert_eq!(count, ALL.len());
    });
}

#[cfg(test)]
#[path = "admission/tests.rs"]
mod tests;
