use alloc::boxed::Box;

use spin::rwlock::RwLock;
use spin::{RwLockReadGuard, RwLockWriteGuard};

use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::multiprocessor::get_lp_count;
use crate::klib::collections::boxed_slice::make_boxed_slice;

#[derive(Debug)]
pub enum Error {
    TargetBusy,
    InvalidIndex,
}

#[derive(Debug)]
pub struct PerLp<T> {
    data: Box<[RwLock<T>]>,
}

impl<'a, T> PerLp<T> {
    pub fn new<F: Fn() -> T>(initializer: F) -> Self {
        PerLp {
            data: make_boxed_slice(get_lp_count() as usize, || RwLock::new(initializer())),
        }
    }

    pub fn try_get(&'a self) -> Result<RwLockReadGuard<'a, T>, Error> {
        match self.data[get_lp_id() as usize].try_read() {
            Some(guard) => Ok(guard),
            None => Err(Error::TargetBusy),
        }
    }

    pub fn try_get_mut(&'a self) -> Result<RwLockWriteGuard<'a, T>, Error> {
        match self.data[get_lp_id() as usize].try_write() {
            Some(guard) => Ok(guard),
            None => Err(Error::TargetBusy),
        }
    }
}
