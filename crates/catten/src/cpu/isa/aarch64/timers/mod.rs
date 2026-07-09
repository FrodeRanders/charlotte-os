pub mod generic_timer;

pub use generic_timer::{ArmGenericTimer, LpTimer};

use crate::logln;
use generic_timer::{TIMER_CYCLE_PERIOD, TIMER_FREQUENCY_HZ};

pub fn print_timer_info() {
    logln!("The ARM Generic Timer frequency is {:?} Hz.", (*TIMER_FREQUENCY_HZ));
    logln!(
        "The ARM Generic Timer period is {:?} picoseconds.",
        ((*TIMER_CYCLE_PERIOD).as_picos())
    );
}
