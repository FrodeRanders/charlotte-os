pub mod apic_timer;
pub mod i8254;
pub mod tsc;

pub use apic_timer::LpTimer;

use crate::cpu::isa::timers::tsc::{IS_TSC_INVARIANT, TSC_CYCLE_PERIOD, TSC_FREQUENCY_HZ};
use crate::logln;

pub fn print_timer_info() {
    if *IS_TSC_INVARIANT {
        logln!("The x86-64 Timestamp Counter IS invariant.");
    } else {
        logln!("The x86-64 Timestamp Counter is NOT invariant.");
    }
    logln!("The x86-64 Timestamp Counter frequency is {:?} MHz.", (*TSC_FREQUENCY_HZ / 1_000_000));
    logln!(
        "The x86-64 Timestamp Counter period is {:?} picoseconds.",
        ((*TSC_CYCLE_PERIOD).as_picos())
    );
}
