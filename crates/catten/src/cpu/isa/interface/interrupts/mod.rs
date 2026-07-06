use crate::cpu::{
    interrupt_routing::InterruptHandler,
    isa::lp::{
        InterruptVectorNum,
        LpId,
    },
};

/// Dynamic Interrupt Dispatcher Interface
pub trait DynInterruptDispatcherIfce {
    /// Set the interrupt handler for a given logical processor and vector
    /// Note: must be #[unsafe(no_mangle)] and extern "C" to be callable from assembly code
    extern "C" fn set_dyn_ih(
        &self,
        lp: LpId,
        vector: InterruptVectorNum,
        handler: InterruptHandler,
    );

    /// Get the interrupt handler for a given vector
    /// Note: must be #[unsafe(no_mangle)] and extern "C" to be callable from assembly code
    extern "C" fn get_dyn_ih(&self, vector: InterruptVectorNum) -> *const InterruptHandler;

    /// Check if a given vector is available for a logical processor
    fn is_vector_available(&self, lp: LpId, vector: InterruptVectorNum) -> bool;
}

/// Local Interrupt Controller Interface
pub trait LocalIntCtlrIfce {
    type Error;

    /// Initialize the local interrupt controller for the current logical processor
    fn init_lp();
    /// Send an inter-processor interrupt to the specified logical processor
    fn send_unicast_ipi(
        target_lp: LpId,
        target_vector: InterruptVectorNum,
    ) -> Result<(), Self::Error>;
    /// Signal End of Interrupt
    fn signal_eoi();
}
