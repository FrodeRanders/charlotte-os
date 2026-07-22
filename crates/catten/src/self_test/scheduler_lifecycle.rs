//! Scheduler timer lifecycle regression coverage.

use crate::{
    cpu::{
        isa::lp::ops::get_lp_id,
        scheduler::{sleep_millis, spawn_thread},
    },
    logln,
    memory::KERNEL_ASID,
};

pub fn test_scheduler_lifecycle() {
    spawn_thread(KERNEL_ASID, worker);
    logln!("[scheduler lifecycle] timer-affinity worker deferred");
}

extern "C" fn worker() {
    let home = get_lp_id();
    for _ in 0..128 {
        sleep_millis(1);
        assert_eq!(get_lp_id(), home);
    }
    logln!(
        "[scheduler lifecycle] SUCCESS: 128 timer wakes retained LP{} affinity.",
        home
    );
}
