pub mod ethernet;
pub mod input_ctlr;
pub mod iommu;
pub mod persistent_storage;
pub mod uart;

use crate::device_management::drivers::DeviceControlPlane;
use crate::device_management::topology::DeviceLocation;

pub trait EndpointControlPlane: DeviceControlPlane {
    fn get_location(&self) -> &DeviceLocation;
}
