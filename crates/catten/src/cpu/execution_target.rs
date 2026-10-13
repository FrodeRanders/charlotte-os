//! Execution capabilities common to every schedulable logical processor.

use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    AtomicUsize,
    Ordering,
};

use charlotte_launch::execution_target::{
    Architecture,
    ExecutionTarget,
    KNOWN_FEATURES,
};

const MAX_LPS: usize = 256;
static REPORTED: [AtomicBool; MAX_LPS] = [const { AtomicBool::new(false) }; MAX_LPS];
static FEATURES: AtomicU64 = AtomicU64::new(KNOWN_FEATURES);
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Boot-only, after the architecture has installed this LP's identity.
/// Integer extensions need no extra context save state. Unsupported vector
/// extensions are deliberately not advertised merely because CPUID has them.
pub(crate) fn record_current_lp() {
    let lp = super::isa::lp::ops::get_lp_id();
    let Some(reported) = REPORTED.get(lp as usize) else {
        return;
    };
    if reported.swap(true, Ordering::AcqRel) {
        return;
    }
    let features = local_features();
    FEATURES.fetch_and(features, Ordering::AcqRel);
    COUNT.fetch_add(1, Ordering::AcqRel);
}

pub(crate) fn current() -> Option<ExecutionTarget> {
    #[cfg(target_arch = "aarch64")]
    let architecture = Architecture::Aarch64;
    #[cfg(target_arch = "x86_64")]
    let architecture = Architecture::X86_64;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    return None;

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    {
        let expected = super::multiprocessor::get_lp_count() as usize;
        // Before every LP reports, expose only the architecture baseline.
        // A migrating thread may never rely on one faster core's extensions.
        let features = if expected != 0 && COUNT.load(Ordering::Acquire) == expected {
            FEATURES.load(Ordering::Acquire)
        } else {
            0
        };
        Some(ExecutionTarget {
            features,
            ..ExecutionTarget::baseline(architecture)
        })
    }
}

#[cfg(target_arch = "aarch64")]
fn local_features() -> u64 {
    let isar0: u64;
    unsafe {
        core::arch::asm!("mrs {}, ID_AA64ISAR0_EL1", out(reg) isar0, options(nomem, nostack, preserves_flags));
    }
    // ID_AA64ISAR0_EL1: Atomic[23:20] >= 2 is FEAT_LSE;
    // CRC32[19:16] >= 1 is FEAT_CRC32.
    u64::from((isar0 >> 20) & 0xf >= 2) | (u64::from((isar0 >> 16) & 0xf >= 1) << 1)
}

#[cfg(target_arch = "x86_64")]
fn local_features() -> u64 {
    // CPUID.01H:ECX[13] is CMPXCHG16B, ECX[23] is POPCNT.
    let ecx = core::arch::x86_64::__cpuid(1).ecx;
    u64::from(ecx & (1 << 13) != 0) | (u64::from(ecx & (1 << 23) != 0) << 1)
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
fn local_features() -> u64 {
    0
}
