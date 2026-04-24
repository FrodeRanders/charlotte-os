//! # Device Drivers
use core::fmt::{Debug, Display};

use crate::device_manager::DeviceId;

pub mod busses;
pub mod display;
pub mod input;
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
