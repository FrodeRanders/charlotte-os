// Qualification profile: co-scale requested bytes/counts for the three recent
// metadata pools, preserving a quarter of each pool for trusted preparation.
use super::{
    DEFAULT,
    Policy,
    Pool,
};

const fn larger(pool: Pool<2>) -> Pool<2> {
    Pool {
        total: [pool.total[0] * 5 / 4, pool.total[1] * 5 / 4],
        ordinary: [pool.ordinary[0] * 5 / 4, pool.ordinary[1] * 5 / 4],
    }
}
pub(super) const POLICY: Policy = Policy {
    name: "larger-metadata-test",
    scratch_nodes: larger(DEFAULT.scratch_nodes),
    snapshot_buffers: larger(DEFAULT.snapshot_buffers),
    mapping_nodes: larger(DEFAULT.mapping_nodes),
    ..DEFAULT
};
