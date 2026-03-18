use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::marker::PhantomData;

use crate::cpu::isa::lp::ops::get_lp_id;

pub enum Error {
    TargetBusy,
    InvalidIndex,
}

enum IntMutState {
    Available,
    Write,
    Read(usize),
}
pub struct PerLp<T> {
    data: Box<[UnsafeCell<(T, IntMutState)>]>,
    _phantom: PhantomData<T>,
}

pub struct PerLpRoGuard<'a, T> {
    referrent: &'a PerLp<T>,
    index: usize,
}

impl<'a, T> PerLp<T> {
    fn try_get(&'a self) -> Result<PerLpRoGuard<'a, T>, Error> {
        let (_, datum_state) =
            unsafe { self.data.get(get_lp_id() as usize).unwrap().as_mut_unchecked() };
        match datum_state {
            IntMutState::Available => {
                *datum_state = IntMutState::Read(1);
                Ok(PerLpRoGuard {
                    referrent: self,
                    index: get_lp_id() as usize,
                })
            }
            IntMutState::Read(count) => {
                *count += 1;
                Ok(PerLpRoGuard {
                    referrent: self,
                    index: get_lp_id() as usize,
                })
            }
            IntMutState::Write => Err(Error::TargetBusy),
        }
    }
}
