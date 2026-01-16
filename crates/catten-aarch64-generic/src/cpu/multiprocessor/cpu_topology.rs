//! # Physical Processor Cores
//!
//! This module defines the `CpuTopology` struct, which represents the hierarchical
//! organization of physical processor cores in a multi-processor system. The topology
//! includes sockets, clusters, and cores, allowing for efficient management and
//! scheduling of threads on different logical processors.
use alloc::vec::Vec;

use crate::cpu::isa::cpu::LpId;

pub struct CpuTopology {
    pub sockets: Vec<Socket>,
}

impl CpuTopology {}

pub struct Socket {
    pub clusters: Vec<Cluster>,
}

pub struct Cluster {
    pub cores: Vec<Core>,
}

pub struct Core {
    pub lp_ids: Vec<LpId>,
}
