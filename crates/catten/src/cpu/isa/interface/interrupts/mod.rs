use crate::cpu::isa::lp::{
    InterruptVectorNum,
    LpId,
};

/// # Local Interrupt Controller Interface
pub trait LocalIntCtlrIfce {
    type Error;

    /// # Initialize the local interrupt controller for the current logical processor
    fn init_lp();
    /// Send an inter-processor interrupt to the specified logical processor
    fn send_unicast_ipi(
        target_lp: LpId,
        target_vector: InterruptVectorNum,
    ) -> Result<(), Self::Error>;
    /// Signal End of Interrupt
    fn signal_eoi();
}
