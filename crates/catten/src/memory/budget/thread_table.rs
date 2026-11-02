//! Shared table backing retains its triggering principal and trusted class
//! through actual deallocation, including concurrent replacement candidates.
use crate::{
    klib::admission::{
        self,
        ThreadTable,
    },
    memory::usage::principal_history::ThreadTableCharge,
};

#[derive(Debug)]
pub struct Charge {
    _node: admission::Reservation<ThreadTable, 2>,
    _principal: Option<ThreadTableCharge>,
}

impl Charge {
    pub(super) fn reserve(
        principal: Option<u64>,
        platform: bool,
        amount: [u64; 2],
    ) -> Result<Self, super::Error> {
        let principal = principal
            .map(|id| ThreadTableCharge::try_new(id, amount))
            .transpose()
            .map_err(|_| super::Error::Limit)?;
        let node =
            admission::Reservation::reserve(platform, amount).map_err(|_| super::Error::Limit)?;
        Ok(Self {
            _node: node,
            _principal: principal,
        })
    }
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<ThreadTable, 2>()
}

/// Counter-only saturation is separate from the real-buffer controls.
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn test_node_pressure() {
    let before = snapshot();
    let ceiling = crate::resource_policy::POLICY.thread_table.ordinary;
    let amount = [ceiling[0] - before.1[0], ceiling[1] - before.1[1]];
    let pressure = Charge::reserve(None, false, amount).unwrap();
    let full = snapshot();
    assert!(Charge::reserve(None, false, [1, 0]).is_err());
    assert!(Charge::reserve(None, false, [0, 1]).is_err());
    assert_eq!(snapshot(), full);
    let platform = Charge::reserve(None, true, [1024, 4]).unwrap();
    drop(platform);
    drop(pressure);
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[thread table reserve] counter-only ordinary byte/slot saturation rejects; platform \
         headroom and exact refunds passed"
    );
}
