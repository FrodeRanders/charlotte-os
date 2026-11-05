use alloc::{
    sync::Arc,
    vec::Vec,
};

use spin::{
    lazylock::LazyLock,
    once::Once,
};
mod calibration;

use super::tsc::TSC_CYCLE_PERIOD;
//use crate::cpu::isa::interrupts::x2apic::X2Apic;
use crate::cpu::isa::timers::tsc::rdtsc;
use crate::{
    cpu::{
        isa::{
            constants::interrupt_vectors::LAPIC_TIMER_VECTOR,
            interface::timers::{
                LpTimerError,
                LpTimerIfce,
                bootstrap::BootTimer,
            },
            lp::ops::get_lp_id,
            x86_64::constants::msrs,
        },
        multiprocessor::{
            get_lp_count,
            spin::mutex::Mutex,
        },
    },
    klib::time::duration::ExtDuration,
};

// All backing is prepared by the BSP. Only the owning LP calibrates its slot;
// runtime get never allocates, calibrates or waits for an initializer.
static APIC_TIMERS: Once<Vec<BootTimer<ApicTimer>>> = Once::new();

pub type LpTimer = ApicTimer;

/// # Timer Divisors for the Local APIC Timer
#[repr(u64)]
pub enum ApicTimerDivisors {
    DivBy2 = 0b0000,
    DivBy4 = 0b0001,
    DivBy8 = 0b0010,
    DivBy16 = 0b0011,
    DivBy32 = 0b1000,
    DivBy64 = 0b1001,
    DivBy128 = 0b1010,
    DivBy1 = 0b1011,
}

#[derive(Clone, Debug)]
pub struct ApicTimer {
    resolution: ExtDuration,
    reset_value: <Self as LpTimerIfce>::TickCount,
}

impl ApicTimer {
    pub fn set_timer_initial_count(count: u32) {
        unsafe {
            msrs::write(msrs::x2apic::TIMER_INITIAL_COUNT, count as u64);
        }
    }

    pub fn read_timer_current_count() -> u32 {
        unsafe { msrs::read(msrs::x2apic::TIMER_CURRENT_COUNT) as u32 }
    }

    fn determine_timer_resolution(&mut self) {
        const SAMPLE_TICKS: u32 = 10_000;

        let _ = self.set_interrupt_mask(true);
        let _ = Self::set_divisor(self, ApicTimerDivisors::DivBy1);
        let resolution = calibration::resolution(|| {
            Self::set_timer_initial_count(SAMPLE_TICKS);
            let tsc_start = rdtsc();
            if !crate::klib::spin::bounded_spin(100_000_000, || {
                Self::read_timer_current_count() == 0
            }) {
                return None;
            }
            let tsc_end = rdtsc();
            let duration = (tsc_end - tsc_start) as u128 * (*TSC_CYCLE_PERIOD).as_picos();
            let apic_timer_duration = duration / SAMPLE_TICKS as u128;
            Some(apic_timer_duration)
        });
        Self::set_timer_initial_count(0);
        self.resolution = ExtDuration::from_picos(
            resolution.expect("incomplete or invalid APIC timer calibration"),
        );
    }

    fn prepared() -> Self {
        Self {
            resolution: ExtDuration::from_secs(0),
            reset_value: 0,
        }
    }

    fn initialize_local(&mut self) {
        self.determine_timer_resolution();
        self.set_divisor(ApicTimerDivisors::DivBy1)
            .expect("Setting the x2APIC timer divisor failed");
        self.set_isr_dispatch_number(LAPIC_TIMER_VECTOR)
            .expect("Setting the interrupt vector number for the x2APIC timer failed");
        self.set_interrupt_mask(false).expect("Error unmasking timer interrupt in the x2APIC.");
    }
}

impl LpTimerIfce for ApicTimer {
    type Divisor = ApicTimerDivisors;
    type IntDispatchNum = u8;
    type TickCount = u32;
    type Timestamp = u64;

    const NAME: &'static str = "x86-64 x2APIC Timer";

    fn prepare_boot_storage() {
        LazyLock::force(&TSC_CYCLE_PERIOD);
        APIC_TIMERS.call_once(|| {
            (0..get_lp_count()).map(|_| BootTimer::prepare(Self::prepared())).collect()
        });
    }

    fn initialize_local_boot() {
        APIC_TIMERS.get().expect("timer backing missing at boot")[get_lp_id() as usize]
            .initialize(Self::initialize_local);
    }

    fn boot_ready() -> bool {
        APIC_TIMERS.get().is_some_and(|timers| {
            timers.len() == get_lp_count() as usize && timers.iter().all(BootTimer::ready)
        })
    }

    fn get() -> Arc<Mutex<Self>> {
        APIC_TIMERS.get().expect("timer backing missing at boot")[get_lp_id() as usize].get()
    }

    fn now() -> Self::Timestamp {
        rdtsc()
    }

    fn get_ts_cycle_period() -> ExtDuration {
        *TSC_CYCLE_PERIOD
    }

    fn get_int_resolution(&self) -> Result<ExtDuration, LpTimerError> {
        Ok(self.resolution)
    }

    fn set_divisor(&mut self, divisor: Self::Divisor) -> Result<(), LpTimerError> {
        unsafe {
            msrs::write(msrs::x2apic::TIMER_DIVIDE_CONFIGURATION, divisor as u64);
        }
        Ok(())
    }

    fn set_duration(&mut self, duration: ExtDuration) -> Result<(), LpTimerError> {
        // Clamp to at least one timer tick: a deadline equal to `now` produces
        // a zero duration, and `start` rejects a zero initial count as
        // `DurationNotSet`. One tick is ~one TSC period, well below any
        // observable timer resolution.
        let ticks = duration.as_picos() / (self.resolution.as_picos())
            + if !duration.as_picos().is_multiple_of(self.resolution.as_picos()) {
                1
            } else {
                0
            };
        self.reset_value = ticks.max(1).try_into().map_err(|_| LpTimerError::DurationOutOfRange)?;
        Ok(())
    }

    fn set_deadline(&mut self, deadline: Self::Timestamp) -> Result<(), LpTimerError> {
        let current_time = Self::now();
        if deadline < current_time {
            return Err(LpTimerError::DeadlinePassed);
        }
        let duration_until_deadline = ExtDuration::from_picos(
            (deadline - current_time) as u128 * TSC_CYCLE_PERIOD.as_picos(),
        );
        let period_ps = self.resolution.as_picos();
        let ticks = duration_until_deadline.as_picos() / period_ps
            + u128::from(!duration_until_deadline.as_picos().is_multiple_of(period_ps));
        // APIC initial counts are 32-bit. Use multiple hardware checkpoints
        // to reach a distant logical deadline within that range. TimerQueue
        // compares `now` with the unchanged logical deadline on every IRQ.
        self.reset_value = charlotte_lifecycle::timer_checkpoint_count(ticks);
        Ok(())
    }

    fn get_duration(&self) -> Result<ExtDuration, LpTimerError> {
        let current_count = Self::read_timer_current_count();
        let duration = ExtDuration::from_picos(current_count as u128 * self.resolution.as_picos());
        Ok(duration)
    }

    fn start(&mut self) -> Result<(), LpTimerError> {
        if self.reset_value == 0 {
            Err(LpTimerError::DurationNotSet)
        } else {
            self.reset()
        }
    }

    fn stop(&mut self) -> Result<(), LpTimerError> {
        if Self::read_timer_current_count() == 0 {
            Err(LpTimerError::TimerNotStarted)
        } else {
            Self::set_timer_initial_count(0);
            Ok(())
        }
    }

    fn reset(&mut self) -> Result<(), LpTimerError> {
        Self::set_timer_initial_count(self.reset_value);
        Ok(())
    }

    fn get_interrupt_mask(&mut self) -> Result<bool, LpTimerError> {
        const MASK_BIT_SHIFT: u64 = 16;
        let apic_timer_lvt_entry = unsafe { msrs::read(msrs::x2apic::TIMER_LVTR) };
        let is_masked = (apic_timer_lvt_entry >> MASK_BIT_SHIFT) & 0b1 == 1;
        Ok(is_masked)
    }

    fn set_interrupt_mask(&mut self, mask: bool) -> Result<(), LpTimerError> {
        const MASK_BIT_SHIFT: u64 = 16;
        let mut apic_timer_lvt_entry = unsafe { msrs::read(msrs::x2apic::TIMER_LVTR) };
        if mask {
            apic_timer_lvt_entry |= 1u64 << MASK_BIT_SHIFT;
        } else {
            apic_timer_lvt_entry &= !(1u64 << MASK_BIT_SHIFT);
        }
        unsafe { msrs::write(msrs::x2apic::TIMER_LVTR, apic_timer_lvt_entry) };
        Ok(())
    }

    fn set_isr_dispatch_number(&mut self, num: Self::IntDispatchNum) -> Result<(), LpTimerError> {
        let mut apic_timer_lvt_entry = unsafe { msrs::read(msrs::x2apic::TIMER_LVTR) };
        apic_timer_lvt_entry &= !0xffu64;
        apic_timer_lvt_entry |= num as u64;
        unsafe { msrs::write(msrs::x2apic::TIMER_LVTR, apic_timer_lvt_entry) };
        Ok(())
    }
}
