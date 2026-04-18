use core::sync::atomic::{AtomicI64, Ordering};

pub struct RwLock<T> {
    raw_lock: AtomicI64,
    data: T,
}
