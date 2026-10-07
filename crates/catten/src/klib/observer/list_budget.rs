//! Independent admission for observer-list allocations, including empty lists
//! retained by registration tokens and weak-only backing. Entry limits account
//! for a different lifetime. No namespace lookup or allocator under this lock.

use charlotte_lifecycle::resources::CountBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const NODE_LIMIT: usize = 8_192;
pub(crate) const ORDINARY_LIMIT: usize = 6_144;

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}
static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: CountBudget::new(NODE_LIMIT),
        ordinary: CountBudget::new(ORDINARY_LIMIT),
    })
});

#[derive(Debug)]
pub(crate) struct Charge {
    platform: bool,
}

/// Classification must come from captured source admission or the first
/// waiting generation's sponsor, never an application-supplied role or ASID.
pub(crate) fn reserve(platform: bool) -> Result<Charge, super::registration::RegistrationError> {
    use super::registration::RegistrationError;
    let mut node = NODE.lock();
    node.total.reserve().map_err(|_| RegistrationError::ResourceLimit)?;
    if !platform && node.ordinary.reserve().is_err() {
        node.total.release().unwrap();
        return Err(RegistrationError::ResourceLimit);
    }
    Ok(Charge {
        platform,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut node = NODE.lock();
        node.total.release().expect("observer list node budget underflow");
        if !self.platform {
            node.ordinary.release().expect("observer list ordinary budget underflow");
        }
    }
}

pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
