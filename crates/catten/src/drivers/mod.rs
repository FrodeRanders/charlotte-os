//! # Device Drivers
use alloc::boxed::Box;
use core::fmt::{Debug, Display};

use hashbrown::{HashMap, HashSet};

use crate::cpu::scheduler::sync::mutex::Mutex;
use crate::cpu::scheduler::sync::rwlock::RwLock;
use crate::device_manager::DeviceId;

pub mod busses;
pub mod input_ctlr;
pub mod uart;

pub trait Driver {
    type Error: core::fmt::Debug;
    type Iter: Iterator<Item = DeviceId>;

    fn init(&mut self, device: DeviceId) -> Result<(), Self::Error>;
    fn deinit(&mut self, device: DeviceId) -> Result<(), Self::Error>;
    fn device_iter(&self) -> Self::Iter;
}

pub trait Device: Debug + Display {
    fn id(&self) -> DeviceId;
}

pub struct DeviceInterfaceIndex {
    pub busses: RwLock<HashSet<DeviceId>>,
    pub input_controllers: RwLock<HashSet<DeviceId>>,
    pub uarts: RwLock<HashSet<DeviceId>>,
}

pub struct DeviceInterfaceTable {
    devices: RwLock<HashMap<DeviceId, Box<Mutex<dyn Device>>>>,
    index: DeviceInterfaceIndex,
}
