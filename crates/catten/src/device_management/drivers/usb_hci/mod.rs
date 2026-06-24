pub mod ehci;
pub mod xhci;

pub trait UsbHostController {
    fn initialize(&self) -> Result<(), ()>;
    fn deinitialize(&self) -> Result<(), ()>;
}
