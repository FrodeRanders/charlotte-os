//! BSP-owned backing, followed by exactly one owning-LP hardware setup.
use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicU8,
    Ordering,
};

use crate::cpu::multiprocessor::spin::mutex::Mutex;

const PREPARED: u8 = 0;
const INITIALIZING: u8 = 1;
const READY: u8 = 2;

pub(crate) struct BootTimer<T> {
    state: Arc<Mutex<T>>,
    phase: AtomicU8,
}

impl<T> BootTimer<T> {
    /// Mandatory BSP-only boot allocation. The value must not touch LP hardware.
    pub fn prepare(state: T) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
            phase: AtomicU8::new(PREPARED),
        }
    }

    /// Call only on this slot's LP with CPU interrupts masked before scheduling.
    /// Interrupted/failed setup leaves INITIALIZING; it never authorizes replay.
    pub fn initialize(&self, initialize: impl FnOnce(&mut T)) {
        assert_eq!(
            self.phase.compare_exchange(
                PREPARED,
                INITIALIZING,
                Ordering::AcqRel,
                Ordering::Acquire
            ),
            Ok(PREPARED),
            "timer boot setup repeated or uncertain"
        );
        {
            let mut state = self.state.lock();
            initialize(&mut state);
        }
        self.phase.store(READY, Ordering::Release);
    }

    pub fn ready(&self) -> bool {
        self.phase.load(Ordering::Acquire) == READY
    }

    /// Runtime access never constructs backing, waits for setup or touches hardware.
    pub fn get(&self) -> Arc<Mutex<T>> {
        assert!(self.ready(), "timer hardware was not initialized at boot");
        self.state.clone()
    }
}
