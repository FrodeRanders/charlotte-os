use alloc::vec::Vec;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use spin::LazyLock;

use crate::{
    cpu::{
        isa::lp::ops::{
            get_int_state,
            get_lp_id,
            mask_interrupts,
            unmask_interrupts,
        },
        multiprocessor::get_lp_count,
    },
    klib::sync_cell::SyncUnsafeCell,
};

pub static INT_STATE: LazyLock<IntState> = LazyLock::new(IntState::new);

pub struct IntState {
    raw_locks: Vec<AtomicBool>,
    save_counts: Vec<SyncUnsafeCell<usize>>,
    saved_int_bits: Vec<SyncUnsafeCell<bool>>,
}

impl IntState {
    pub fn new() -> Self {
        let num_cpus = get_lp_count() as usize;
        Self {
            raw_locks: (0..num_cpus).map(|_| AtomicBool::default()).collect(),
            save_counts: (0..num_cpus).map(|_| SyncUnsafeCell::new(0)).collect(),
            saved_int_bits: (0..num_cpus).map(|_| SyncUnsafeCell::new(false)).collect(),
        }
    }

    pub fn save_int(&self) {
        let lp_idx = get_lp_id() as usize;
        let int_state = get_int_state();
        mask_interrupts!();
        // Spin until we can acquire the lock
        while self.raw_locks[lp_idx]
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // Increment the save count through the cell pointer. The per-LP raw
        // lock above provides exclusive access for this logical processor, so
        // no reference to the cell contents is ever created.
        let count = self.save_counts[lp_idx].get();
        unsafe {
            let current = *count;
            *count = current + 1;
            // save and clear the interrupt enable bit if necessary.
            if current == 0 {
                *self.saved_int_bits[lp_idx].get() = int_state;
            }
        }
        // Release the raw lock
        self.raw_locks[lp_idx].store(false, Ordering::Release);
    }

    pub fn restore_int(&self) {
        let lp_idx = get_lp_id() as usize;
        mask_interrupts!();
        // Spin until we can acquire the lock
        while self.raw_locks[lp_idx]
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // Decrement the save count through the cell pointer.
        let count = self.save_counts[lp_idx].get();
        let mut restore_saved_int = false;
        unsafe {
            let current = *count;
            debug_assert!(current > 0, "unbalanced interrupt-state restore");
            *count = current.saturating_sub(1);
            // restore the interrupt enable bit if necessary.
            if current == 1 {
                restore_saved_int = *self.saved_int_bits[lp_idx].get();
            }
        }
        // Release the raw lock
        self.raw_locks[lp_idx].store(false, Ordering::Release);
        if restore_saved_int {
            unmask_interrupts!();
        }
    }
}

impl Default for IntState {
    fn default() -> Self {
        Self::new()
    }
}
