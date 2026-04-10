use alloc::sync::Arc;

use spin::Mutex;

pub use crate::klib::time::duration::ExtDuration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LpTimerError {
    DeadlinePassed,
    DivisorNotSupported,
    DurationNotSet,
    DurationOutOfRange,
    TimerAlreadyStarted,
    TimerNotPresent,
    TimerNotStarted,
    TimerStartsAutomatically,
}

pub trait LpTimerIfce {
    //! # Local Interrupt Controller Timer Interface

    const NAME: &'static str;

    type Divisor;
    type TickCount;
    type IntDispatchNum;
    type Timestamp;

    fn get() -> Arc<Mutex<Self>>;
    // Timestamp functions
    fn now() -> Self::Timestamp;
    fn get_ts_cycle_period() -> ExtDuration;
    // Timer Interrupt Source functions
    fn get_int_resolution(&self) -> Result<ExtDuration, LpTimerError>;
    fn set_divisor(&mut self, divisor: Self::Divisor) -> Result<(), LpTimerError>;
    fn set_duration(&mut self, duration: ExtDuration) -> Result<(), LpTimerError>;
    fn set_deadline(&mut self, deadline: Self::Timestamp) -> Result<(), LpTimerError>;
    fn get_duration(&self) -> Result<ExtDuration, LpTimerError>;
    fn start(&mut self) -> Result<(), LpTimerError>;
    fn stop(&mut self) -> Result<(), LpTimerError>;
    fn reset(&mut self) -> Result<(), LpTimerError>;
    fn get_interrupt_mask(&mut self) -> Result<bool, LpTimerError>;
    fn set_interrupt_mask(&mut self, mask: bool) -> Result<(), LpTimerError>;
    fn set_isr_dispatch_number(&mut self, num: Self::IntDispatchNum) -> Result<(), LpTimerError>;
}
