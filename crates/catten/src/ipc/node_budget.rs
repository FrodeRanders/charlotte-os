//! IPC payload-node allocation admission is separate from logical
//! record counts. The same node retains both until actual deallocation.
use super::*;
use crate::klib::{
    admission::{
        self,
        IpcNodes,
    },
    collections::retirement_list::BindCharge,
};

pub(super) struct Charge<R = record_budget::Charge> {
    _backing: admission::NodeCharge<IpcNodes>,
    resource: Option<R>,
}
impl<R> BindCharge<R> for Charge<R> {
    fn bind(&mut self, charge: R) {
        assert!(self.resource.is_none(), "IPC record admission rebound");
        self.resource = Some(charge);
    }
}
pub(super) type Storage<T, R = record_budget::Charge> = PreparedEntry<T, Charge<R>>;
pub(super) type Retired<T, R = record_budget::Charge> = RetiredEntry<T, Charge<R>>;
pub(super) type Map<K, V, R = record_budget::Charge> = AdmittedMap<K, V, Charge<R>>;
pub(super) type List<T, R = record_budget::Charge> = RetirementList<T, Charge<R>>;

pub(super) fn prepare<T>(platform: bool) -> Result<Storage<T>, IpcError> {
    prepare_with(platform, Storage::try_new_charged)
}
pub(super) fn prepare_endpoint<T>(platform: bool) -> Result<Storage<T, budget::Charge>, IpcError> {
    prepare_with(platform, Storage::try_new_charged)
}
fn prepare_with<T, R>(
    platform: bool,
    allocate: impl FnOnce(Charge<R>) -> Result<Storage<T, R>, core::alloc::AllocError>,
) -> Result<Storage<T, R>, IpcError> {
    admission::prepare_node_with::<_, IpcNodes>(
        platform,
        Storage::<T, R>::allocation_bytes(),
        |backing| {
            allocate(Charge {
                _backing: backing,
                resource: None,
            })
        },
    )
    .map_err(|_| IpcError::ResourceLimit)
}
#[cfg(feature = "kernel_tests")]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<IpcNodes, 2>()
}
#[cfg(feature = "kernel_tests")]
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    admission::with_locked::<IpcNodes, 2, _>(work)
}
#[cfg(feature = "kernel_tests")]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(super) use tests::run as test_admission;
