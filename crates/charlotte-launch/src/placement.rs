//! Cluster placement policy shared by deployment tooling and nodes.
//!
//! This is a policy contract, not a scheduler implementation. It makes the
//! important distinctions explicit before the replicated deployment format
//! grows from its current single-assignment slice: component replicas,
//! co-location affinity between different components, anti-affinity/failure
//! domains, and whether the blessed artifact permits parallel instances.

use crate::signature_note::{
    ArtifactMetadata,
    FLAG_PARALLEL_INSTANCES,
};

pub const COLOCATE_AFFINITY_GROUP: u16 = 1 << 0;
pub const SPREAD_REPLICAS: u16 = 1 << 1;
pub const EVERY_ELIGIBLE_NODE: u16 = 1 << 2;

/// Desired placement for one immutable artifact generation.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementPolicy {
    /// Desired instance count. Zero is valid only with `EVERY_ELIGIBLE_NODE`.
    pub replicas: u16,
    /// Upper bound for instances of this component on one node.
    pub max_instances_per_node: u16,
    /// Minimum distinct nodes represented by the instances.
    pub min_distinct_nodes: u16,
    pub flags: u16,
    /// Components with the same non-zero group are candidates for co-location
    /// when `COLOCATE_AFFINITY_GROUP` is set.
    pub affinity_group: u64,
    /// Components or replicas with the same non-zero group must be separated.
    pub anti_affinity_group: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    EmptyReplicaSet,
    EveryNodeWithFixedReplicaCount,
    MissingPerNodeCapacity,
    TooManyDistinctNodes,
    ImpossibleSpread,
    ParallelInstancesNotBlessed,
    MissingAffinityGroup,
}

impl PlacementPolicy {
    pub const fn singleton() -> Self {
        Self {
            replicas: 1,
            max_instances_per_node: 1,
            min_distinct_nodes: 1,
            flags: 0,
            affinity_group: 0,
            anti_affinity_group: 0,
        }
    }

    /// Validate a placement declaration against the artifact policy that the
    /// cluster signer blessed into the ELF.
    pub fn validate(&self, artifact: &ArtifactMetadata) -> Result<(), PolicyError> {
        let every_node = self.flags & EVERY_ELIGIBLE_NODE != 0;
        if every_node && self.replicas != 0 {
            return Err(PolicyError::EveryNodeWithFixedReplicaCount);
        }
        if !every_node && self.replicas == 0 {
            return Err(PolicyError::EmptyReplicaSet);
        }
        if self.max_instances_per_node == 0 {
            return Err(PolicyError::MissingPerNodeCapacity);
        }
        if !every_node {
            if self.min_distinct_nodes == 0 || self.min_distinct_nodes > self.replicas {
                return Err(PolicyError::TooManyDistinctNodes);
            }
            if u32::from(self.min_distinct_nodes) * u32::from(self.max_instances_per_node)
                < u32::from(self.replicas)
            {
                return Err(PolicyError::ImpossibleSpread);
            }
        }
        let may_run_in_parallel =
            every_node || self.replicas > 1 || self.max_instances_per_node > 1;
        if may_run_in_parallel && artifact.flags & FLAG_PARALLEL_INSTANCES == 0 {
            return Err(PolicyError::ParallelInstancesNotBlessed);
        }
        if self.flags & COLOCATE_AFFINITY_GROUP != 0 && self.affinity_group == 0 {
            return Err(PolicyError::MissingAffinityGroup);
        }
        Ok(())
    }
}
