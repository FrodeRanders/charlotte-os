//! Pending-call/reply-token allocation admission is separate from logical
//! record counts. The same node retains both until actual deallocation.
use super::*;
use crate::klib::{
    admission::{
        self,
        IpcNodes,
    },
    collections::retirement_list::BindCharge,
};

pub(super) struct Charge {
    _backing: admission::NodeCharge<IpcNodes>,
    resource: Option<record_budget::Charge>,
}
impl BindCharge<record_budget::Charge> for Charge {
    fn bind(&mut self, charge: record_budget::Charge) {
        assert!(self.resource.is_none(), "IPC record admission rebound");
        self.resource = Some(charge);
    }
}
pub(super) type Storage<T> = PreparedEntry<T, Charge>;
pub(super) type Retired<T> = RetiredEntry<T, Charge>;
pub(super) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type List<T> = RetirementList<T, Charge>;

pub(super) fn prepare<T>(platform: bool) -> Result<Storage<T>, IpcError> {
    prepare_with(platform, Storage::try_new_charged)
}
fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Storage<T>, core::alloc::AllocError>,
) -> Result<Storage<T>, IpcError> {
    admission::prepare_node_with::<_, IpcNodes>(
        platform,
        Storage::<T>::allocation_bytes(),
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
