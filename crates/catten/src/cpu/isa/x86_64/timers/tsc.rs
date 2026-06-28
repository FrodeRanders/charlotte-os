use core::arch::asm;

use spin::LazyLock;

use crate::cpu::isa::interface::system_info::CpuInfoIfce;
use crate::cpu::isa::system_info::{CpuInfo, IsaExtension};
use crate::environment::boot_protocol::limine::TSC_FREQUENCY_REQUEST;
use crate::klib::time::duration::ExtDuration;

pub static IS_TSC_INVARIANT: LazyLock<bool> =
    LazyLock::new(|| CpuInfo::is_extension_supported(IsaExtension::InvariantTsc));
pub static TSC_FREQUENCY_HZ: LazyLock<u64> = LazyLock::new(get_tsc_freq);
pub static TSC_CYCLE_PERIOD: LazyLock<ExtDuration> = LazyLock::new(|| {
    let ps = 1_000_000_000_000 / *TSC_FREQUENCY_HZ;
    ExtDuration::from_picos(ps as u128)
});

pub fn rdtsc() -> u64 {
    //! # Read the timestamp counter with proper serialization
    let tsc_low: u32;
    let tsc_high: u32;
    unsafe {
        asm! {
            "rdtscp",
            out("eax") tsc_low,
            out("edx") tsc_high,
            out("ecx") _
        }
    }
    ((tsc_high as u64) << 32) | tsc_low as u64
}

#[inline(always)]
fn get_tsc_freq() -> u64 {
    TSC_FREQUENCY_REQUEST
        .response()
        .expect("The TSC frequency could not be determined. Total system failure.")
        .frequency
}
