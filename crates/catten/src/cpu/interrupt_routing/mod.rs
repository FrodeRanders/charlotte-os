#![allow(dead_code)]
//! # The Interrupt Routing Manager
//!
//! The interrupt routing manager is responsible for the following:
//! - Allocating and deallocating interrupt vectors to devices
//! - Routing interrupts from devices to the appropriate interrupt vector
//! - Managing interrupt redirection tables for both platform level interrupt controllers and
//!   IOMMUs.
//! - Providing a unified interface for devices to register and unregister interrupt handlers.
//! - Ensuring that the interrupt service load is roughly balanced across all logical processors in
//!   the system.

use alloc::collections::btree_map::BTreeMap;

use hashbrown::HashMap;

use crate::{
    cpu::isa::lp::{
        EicId,
        EicPinNum,
        InterruptVectorNum,
        LpId,
    },
    device_management::drivers::busses::{
        pci_express,
        pci_express::topology::PcieLocation,
    },
};

pub type InterruptHandler = extern "C" fn();

pub enum Error {
    InterruptVectorsExhausted,
    InterruptRedirectionTableFull,
    PcieError(pci_express::Error),
}

/// External Interrupt Controller input source
#[derive(Default, Clone, Debug)]
pub struct EicSource {
    pub pic_id: EicId,
    pub pin_num: EicPinNum,
}
#[derive(Default, Clone, Debug)]
pub struct PcieMsiSource {
    pub location: PcieLocation,
    pub msi_num: u32,
}
#[derive(Default, Clone, Debug)]
pub struct PcieMsiXSource {
    pub location: PcieLocation,
    pub table_index: u32,
}
#[derive(Clone, Debug)]
pub enum InterruptRouter {
    ExternalInterruptController(EicSource),
    PcieMsi(PcieMsiSource),
    PcieMsiX(PcieMsiXSource),
}
#[derive(Default, Clone, Debug)]
pub struct InterruptSignalType {
    pub level_triggered: bool,
    pub active_level: bool,
}
#[derive(Clone, Debug)]
pub struct InterruptInput {
    pub router: InterruptRouter,
    pub signal_type: InterruptSignalType,
}

#[derive(Default, Clone, Debug)]
pub struct InterruptRoutingManager {
    routes: HashMap<LpId, BTreeMap<InterruptVectorNum, InterruptInput>>,
}

pub struct InterruptTarget {
    lp_id: LpId,
    vector_num: InterruptVectorNum,
}

impl InterruptRoutingManager {
    pub fn try_register_interrupt(
        &mut self,
        _input: InterruptInput,
        _handler: InterruptHandler,
    ) -> Result<InterruptTarget, Error> {
        todo!(
            "Set the appropriate routing entries in the external interrupt controller, \
             redirection entries in the IOMMU, and local interrupt controller and register the \
             interrupt handler."
        )
    }
}
