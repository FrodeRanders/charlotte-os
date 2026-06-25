//! # Device Drivers

use alloc::boxed::Box;
use core::fmt::Debug;

pub mod busses;
pub mod endpoints;

pub enum Error {
    DeviceNotRecognized,
    InitializationFailed,
    DeinitializationFailed,
    DeviceAlreadyBoundToDriver,
}

pub trait DeviceControlPlane {
    type Status: Debug;

    fn get_status(&self) -> Box<Self::Status>;
}
