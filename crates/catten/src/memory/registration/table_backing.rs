//! ASID-table backing is sponsored before allocation by the unpublished root's
//! trusted translation account. Retained and candidate sets share the limits.
use core::alloc::AllocError;

use crate::{
    klib::{
        admission::{
            self,
            RootTable,
        },
        collections::id_table::{
            GrowthRequest,
            PreparedGrowth,
        },
    },
    memory::{
        AddressSpace,
        AddressSpaceInterface,
        AddressSpaceTable,
        usage::principal_history::RootTableCharge,
    },
};

#[derive(Debug)]
pub struct Charge {
    _node: admission::Reservation<RootTable, 2>,
    _principal: Option<RootTableCharge>,
}

impl Charge {
    fn reserve(
        principal: Option<u64>,
        platform: bool,
        amount: [u64; 2],
    ) -> Result<Self, AllocError> {
        let principal = principal
            .map(|id| RootTableCharge::try_new(id, amount))
            .transpose()
            .map_err(|_| AllocError)?;
        let node = admission::Reservation::reserve(platform, amount).map_err(|_| AllocError)?;
        Ok(Self {
            _node: node,
            _principal: principal,
        })
    }
}

pub(in crate::memory) type Growth = PreparedGrowth<AddressSpace, Charge>;

pub(super) fn prepare(root: &AddressSpace) -> Result<Option<Growth>, AllocError> {
    let request =
        { crate::memory::ADDRESS_SPACE_TABLE.lock().growth_request() }.map_err(|_| AllocError)?;
    request.map(|request| prepare_request(root, request)).transpose()
}

pub(in crate::memory) fn prepare_request(
    root: &AddressSpace,
    request: GrowthRequest<AddressSpace>,
) -> Result<Growth, AllocError> {
    let charge = Charge::reserve(
        root.table_account.principal(),
        root.table_account.is_platform(),
        request.amount().map_err(|_| AllocError)?,
    )?;
    request.prepare(charge).map_err(|_| AllocError)
}

/// Mandatory one-time bootstrap, before this table is globally visible. The
/// kernel slot is trusted captured boot state, independent of root manifests.
pub(in crate::memory) fn kernel_table() -> AddressSpaceTable {
    let mut table = AddressSpaceTable::new();
    let request = table.growth_request().unwrap().unwrap();
    let charge = Charge::reserve(None, true, request.amount().unwrap())
        .expect("kernel ASID-table admission failed");
    let mut growth = request.prepare(charge).expect("kernel ASID-table backing failed");
    let mut root = Some(AddressSpace::get_current());
    let id = table.try_insert_from(&mut root, Some(&mut growth)).unwrap();
    assert_eq!(id, crate::memory::KERNEL_ASID);
    growth.finish();
    table
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<RootTable, 2>()
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<RootTable, 2, _>(work)
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn test_node_pressure() {
    let before = snapshot();
    let ceiling = crate::resource_policy::POLICY.root_table.ordinary;
    let pressure =
        Charge::reserve(None, false, [ceiling[0] - before.1[0], ceiling[1] - before.1[1]]).unwrap();
    let full = snapshot();
    assert!(Charge::reserve(None, false, [1, 0]).is_err());
    assert!(Charge::reserve(None, false, [0, 1]).is_err());
    assert_eq!(snapshot(), full);
    let platform = Charge::reserve(None, true, [1024, 4]).unwrap();
    drop(platform);
    drop(pressure);
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[root table reserve] counter-only ordinary bytes/slots exhausted; platform headroom and \
         exact original refunds passed"
    );
}
