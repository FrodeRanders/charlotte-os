#[cfg(target_arch = "x86_64")]
use limine::mp::MP_FLAG_X2APIC;
use spin::{
    LazyLock,
    RwLock,
};

use crate::{
    ap_main,
    early_logln,
    environment::boot_protocol::limine::MP_REQUEST,
    logln,
};

pub(super) static LP_COUNT: LazyLock<RwLock<u32>> = LazyLock::new(|| {
    RwLock::new({
        if let Some(mp_res) = MP_REQUEST.response() {
            mp_res.cpus().len() as u32
        } else {
            panic!("Limine was not able to start the secondary logical processors!")
        }
    })
});

#[derive(Debug)]
pub enum MpError {
    SecondaryLpStartupFailed,
}

pub fn start_secondary_lps() -> Result<(), MpError> {
    logln!("Starting Secondary LPs...");
    if let Some(res) = MP_REQUEST.response() {
        logln!("Obtained multiprocessor response from Limine");
        cfg_select! {
            target_arch = "x86_64" => {
                if res.flags & MP_FLAG_X2APIC as u32 != 0 {
                    logln!("Limine has set all LAPICs to x2APIC mode.")
                } else {
                    panic!("Processor not supported: x2APIC mode is not available.");
                }
            },
            _ => {/* Non-x86_64 ISAs require no special secondary processor startup handling */}
        }
        let lps = res.cpus();
        for lp in lps {
            logln!("Writing entry point address for LP {}", (lp.processor_id));
            lp.bootstrap(ap_main, 0);
        }
        Ok(())
    } else {
        Err(MpError::SecondaryLpStartupFailed)
    }
}

#[cfg(target_arch = "aarch64")]
use core::sync::atomic::AtomicU64;
use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use crate::cpu::isa::lp::ops::*;

pub static ID_COUNTER: AtomicU32 = AtomicU32::new(0);

#[cfg(target_arch = "aarch64")]
const MAX_TRACKED_LPS: usize = 256;
#[cfg(target_arch = "aarch64")]
const UNKNOWN_MPIDR: u64 = u64::MAX;
/// Hardware affinity indexed by Charlotte's scheduler-local LP id.
///
/// Secondary processors enter concurrently, so their logical ids need not
/// match MPIDR.Aff0. GIC routing must translate rather than assume equality.
#[cfg(target_arch = "aarch64")]
static LP_MPIDRS: [AtomicU64; MAX_TRACKED_LPS] =
    [const { AtomicU64::new(UNKNOWN_MPIDR) }; MAX_TRACKED_LPS];

#[cfg(target_arch = "aarch64")]
pub fn mpidr_for_lp(lp_id: crate::cpu::isa::lp::LpId) -> Option<u64> {
    let value = LP_MPIDRS.get(lp_id as usize)?.load(Ordering::Acquire);
    (value != UNKNOWN_MPIDR).then_some(value)
}

pub unsafe fn assign_id() {
    let lp_id = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    store_lp_id(lp_id);
    #[cfg(target_arch = "aarch64")]
    LP_MPIDRS
        .get(lp_id as usize)
        .unwrap_or_else(|| {
            panic!(
                "logical processor id {lp_id} exceeds the AArch64 MPIDR routing table capacity \
                 ({MAX_TRACKED_LPS})"
            )
        })
        .store(mpidr(), Ordering::Release);
    if lp_id == 0 {
        early_logln!(
            "Logical Processor with local interrupt controller ID = {} has been designated LP {}.",
            (get_lic_id()),
            (get_lp_id())
        );
    } else {
        logln!(
            "Logical Processor with local interrupt controller ID = {} has been designated LP {}.",
            (get_lic_id()),
            (get_lp_id())
        );
    }
    #[cfg(target_arch = "aarch64")]
    crate::cpu::isa::lp::ops::log_mpidr();
}
