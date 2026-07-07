#[repr(C, packed)]
pub struct IoApic {
    ioregsel: IoReg32,
    ioregwin: IoReg32,
}

impl From<NonNull<IoApic>> for IoApic {

impl IoApic {

}
