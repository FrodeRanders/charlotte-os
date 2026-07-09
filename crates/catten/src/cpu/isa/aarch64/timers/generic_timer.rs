//! # ARM Generic Timer
//!
//! The ARM Generic Timer provides a per-core, always-on system counter plus a
//! set of per-core timers. We use:
//! - `CNTFRQ_EL0`: the frequency of the system counter in Hz (timestamp source).
//! - `CNTPCT_EL0`: the current system counter value (our monotonic timestamp).
//! - The EL1 physical timer (`CNTP_CTL_EL0`, `CNTP_CVAL_EL0`, `CNTP_TVAL_EL0`)
//!   as the per-LP interrupt source. It raises its PPI (INTID 30 on the GIC of
//!   the QEMU `virt` machine) when `CNTPCT_EL0 >= CNTP_CVAL_EL0`.
//!
//! See the ARM Architecture Reference Manual (ARM ARM), chapter D12 "The
//! Generic Timer in AArch64 state".

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;

use spin::lazylock::LazyLock;

use crate::cpu::isa::constants::interrupt_vectors::LAPIC_TIMER_VECTOR;
use crate::cpu::isa::interface::timers::{LpTimerError, LpTimerIfce};
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::multiprocessor::get_lp_count;
use crate::cpu::multiprocessor::spin::mutex::Mutex;
use crate::klib::time::duration::ExtDuration;

pub type LpTimer = ArmGenericTimer;

/// `CNTP_CTL_EL0` ENABLE bit: enables the timer.
const CNTP_CTL_ENABLE: u64 = 1 << 0;
/// `CNTP_CTL_EL0` IMASK bit: when set, the timer interrupt is masked.
const CNTP_CTL_IMASK: u64 = 1 << 1;

/// The frequency of the system counter in Hz, read from `CNTFRQ_EL0`. This is a
/// fixed, firmware-programmed value that is identical on every core.
pub static TIMER_FREQUENCY_HZ: LazyLock<u64> = LazyLock::new(read_cntfrq);

/// The period of a single system counter tick.
pub static TIMER_CYCLE_PERIOD: LazyLock<ExtDuration> = LazyLock::new(|| {
    let ps = 1_000_000_000_000u128 / *TIMER_FREQUENCY_HZ as u128;
    ExtDuration::from_picos(ps)
});

pub static GENERIC_TIMERS: LazyLock<Vec<Arc<Mutex<ArmGenericTimer>>>> = LazyLock::new(|| {
    (0..get_lp_count())
        .map(|_| Arc::new(Mutex::new(ArmGenericTimer::new(LAPIC_TIMER_VECTOR))))
        .collect()
});

/// Read the system counter frequency (`CNTFRQ_EL0`) in Hz.
fn read_cntfrq() -> u64 {
    let freq: u64;
    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack, preserves_flags));
    }
    freq
}

/// Read the system counter (`CNTPCT_EL0`). An `isb` is issued first because the
/// architecture permits the counter read to be reordered; the barrier ensures we
/// observe an up-to-date value.
#[inline]
fn read_cntpct() -> u64 {
    let count: u64;
    unsafe {
        asm!(
            "isb",
            "mrs {}, cntpct_el0",
            out(reg) count,
            options(nomem, nostack)
        );
    }
    count
}

#[inline]
fn read_cntp_ctl() -> u64 {
    let ctl: u64;
    unsafe {
        asm!("mrs {}, cntp_ctl_el0", out(reg) ctl, options(nomem, nostack, preserves_flags));
    }
    ctl
}

#[inline]
fn write_cntp_ctl(ctl: u64) {
    unsafe {
        asm!("msr cntp_ctl_el0, {}", in(reg) ctl, options(nomem, nostack, preserves_flags));
    }
}

#[inline]
fn write_cntp_cval(cval: u64) {
    unsafe {
        asm!("msr cntp_cval_el0, {}", in(reg) cval, options(nomem, nostack, preserves_flags));
    }
}

#[derive(Clone, Debug)]
pub struct ArmGenericTimer {
    /// The absolute system-counter value at which the timer should next fire.
    compare_value: <Self as LpTimerIfce>::Timestamp,
    /// The GIC INTID on which this timer's interrupt is delivered. On the ARM
    /// Generic Timer this is fixed by the hardware wiring and cannot be
    /// reprogrammed, so it is retained purely for informational purposes.
    dispatch_num: <Self as LpTimerIfce>::IntDispatchNum,
}

impl ArmGenericTimer {
    pub fn new(dispatch_num: <Self as LpTimerIfce>::IntDispatchNum) -> Self {
        // Ensure the timer starts disabled but with its interrupt unmasked, so
        // that arming it later (by programming CNTP_CVAL and setting ENABLE) is
        // all that is required to make it fire.
        write_cntp_ctl(0);
        ArmGenericTimer {
            compare_value: 0,
            dispatch_num,
        }
    }
}

impl Default for ArmGenericTimer {
    fn default() -> Self {
        ArmGenericTimer::new(LAPIC_TIMER_VECTOR)
    }
}

impl LpTimerIfce for ArmGenericTimer {
    type Divisor = ();
    type IntDispatchNum = u32;
    type TickCount = u64;
    type Timestamp = u64;

    const NAME: &'static str = "ARM Generic Timer (EL1 Physical)";

    fn get() -> Arc<Mutex<Self>> {
        GENERIC_TIMERS[get_lp_id() as usize].clone()
    }

    fn now() -> Self::Timestamp {
        read_cntpct()
    }

    fn get_ts_cycle_period() -> ExtDuration {
        *TIMER_CYCLE_PERIOD
    }

    fn get_int_resolution(&self) -> Result<ExtDuration, LpTimerError> {
        Ok(*TIMER_CYCLE_PERIOD)
    }

    /// The ARM Generic Timer has no programmable prescaler/divisor; the counter
    /// runs at the fixed `CNTFRQ_EL0` rate.
    fn set_divisor(&mut self, _divisor: Self::Divisor) -> Result<(), LpTimerError> {
        Err(LpTimerError::DivisorNotSupported)
    }

    fn set_duration(&mut self, duration: ExtDuration) -> Result<(), LpTimerError> {
        let period_ps = TIMER_CYCLE_PERIOD.as_picos();
        let ticks = duration.as_picos() / period_ps
            + if duration.as_picos() % period_ps > 0 {
                1
            } else {
                0
            };
        let ticks: u64 = ticks.try_into().map_err(|_| LpTimerError::DurationOutOfRange)?;
        self.compare_value = Self::now().checked_add(ticks).ok_or(LpTimerError::DurationOutOfRange)?;
        Ok(())
    }

    fn set_deadline(&mut self, deadline: Self::Timestamp) -> Result<(), LpTimerError> {
        if deadline < Self::now() {
            return Err(LpTimerError::DeadlinePassed);
        }
        self.compare_value = deadline;
        Ok(())
    }

    fn get_duration(&self) -> Result<ExtDuration, LpTimerError> {
        let now = Self::now();
        let remaining_ticks = self.compare_value.saturating_sub(now);
        Ok(ExtDuration::from_picos(remaining_ticks as u128 * TIMER_CYCLE_PERIOD.as_picos()))
    }

    fn start(&mut self) -> Result<(), LpTimerError> {
        // Program the compare value and enable the timer with its interrupt
        // unmasked.
        write_cntp_cval(self.compare_value);
        let ctl = (read_cntp_ctl() & !CNTP_CTL_IMASK) | CNTP_CTL_ENABLE;
        write_cntp_ctl(ctl);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), LpTimerError> {
        let ctl = read_cntp_ctl() & !CNTP_CTL_ENABLE;
        write_cntp_ctl(ctl);
        Ok(())
    }

    fn reset(&mut self) -> Result<(), LpTimerError> {
        self.start()
    }

    fn get_interrupt_mask(&mut self) -> Result<bool, LpTimerError> {
        Ok(read_cntp_ctl() & CNTP_CTL_IMASK != 0)
    }

    fn set_interrupt_mask(&mut self, mask: bool) -> Result<(), LpTimerError> {
        let mut ctl = read_cntp_ctl();
        if mask {
            ctl |= CNTP_CTL_IMASK;
        } else {
            ctl &= !CNTP_CTL_IMASK;
        }
        write_cntp_ctl(ctl);
        Ok(())
    }

    /// The ARM Generic Timer interrupt is delivered on a fixed, hardware-wired
    /// PPI, so the dispatch number cannot be reprogrammed. We record the
    /// requested value for informational purposes only.
    fn set_isr_dispatch_number(&mut self, num: Self::IntDispatchNum) -> Result<(), LpTimerError> {
        self.dispatch_num = num;
        Ok(())
    }
}
