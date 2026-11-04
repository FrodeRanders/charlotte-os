//! Fixed kernel-lifetime image ownership. Published images are immutable and
//! never evicted, allowing borrows tied to the cache rather than its lock guard.
use alloc::vec::Vec;
use core::{
    mem::{
        self,
        ManuallyDrop,
    },
    ops::{
        Deref,
        DerefMut,
    },
};

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::admission::{
        ByteAllocator,
        Reservation,
        ServiceImages,
    },
};

pub(super) type Image = Vec<u8, ByteAllocator<ServiceImages>>;
enum Slot {
    Empty,
    Preparing {
        name: &'static [u8],
    },
    Published {
        name: &'static [u8],
        image: Image,
        _charge: Reservation<ServiceImages, 2>,
    },
}
pub(super) struct Cache<const N: usize> {
    entries: Mutex<[Slot; N]>,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum AdmissionError {
    Duplicate,
    Full,
}

impl<const N: usize> Cache<N> {
    #[cfg(feature = "kernel_tests")]
    pub(super) fn with_locked<T>(&self, work: impl FnOnce() -> T) -> T {
        let _guard = self.entries.lock();
        work()
    }

    pub(super) const fn new() -> Self {
        Self {
            entries: Mutex::new([const { Slot::Empty }; N]),
        }
    }

    pub(super) fn get(&self, name: &[u8]) -> Option<&[u8]> {
        let (pointer, len) = {
            let entries = self.entries.lock();
            entries.iter().find_map(|entry| match entry {
                Slot::Published {
                    name: candidate,
                    image,
                    ..
                } if *candidate == name => Some((image.as_ptr(), image.len())),
                _ => None,
            })?
        };
        // SAFETY: no API removes/replaces/modifies a Published slot or its Vec.
        // Storage stays live until Cache drops, which this returned self-borrow
        // prevents. The production cache is static; its images last the boot.
        Some(unsafe { core::slice::from_raw_parts(pointer, len) })
    }

    pub(super) fn prepare(
        &self,
        name: &'static [u8],
    ) -> Result<Preparation<'_, N>, AdmissionError> {
        // Only kernel bootstrap/lookup code can request these static artifact
        // names. Capture the immortal kernel namespace's trusted class; neither
        // signed artifact roles nor the requesting service choose it.
        let charge = Reservation::reserve(true, [0, 1]).map_err(|_| AdmissionError::Full)?;
        let selected = {
            let mut entries = self.entries.lock();
            let duplicate = entries.iter().any(|entry| match entry {
                Slot::Preparing {
                    name: candidate,
                    ..
                }
                | Slot::Published {
                    name: candidate,
                    ..
                } => *candidate == name,
                Slot::Empty => false,
            });
            if duplicate {
                Err(AdmissionError::Duplicate)
            } else if let Some(index) =
                entries.iter().position(|entry| matches!(entry, Slot::Empty))
            {
                entries[index] = Slot::Preparing {
                    name,
                };
                return Ok(Preparation(ManuallyDrop::new(Resources {
                    cache: self,
                    index,
                    image: None,
                    charge: Some(charge),
                })));
            } else {
                Err(AdmissionError::Full)
            }
        };
        drop(charge);
        selected
    }
}

#[must_use]
pub(super) struct Preparation<'a, const N: usize>(ManuallyDrop<Resources<'a, N>>);
pub(super) struct Resources<'a, const N: usize> {
    cache: &'a Cache<N>,
    index: usize,
    pub(super) image: Option<Image>,
    charge: Option<Reservation<ServiceImages, 2>>,
}
impl<'a, const N: usize> Deref for Preparation<'a, N> {
    type Target = Resources<'a, N>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<const N: usize> DerefMut for Preparation<'_, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<'a, const N: usize> Preparation<'a, N> {
    pub(super) fn cancel(mut self) {
        // Actual image disposal precedes slot/count refund; both occur after
        // the caller has left I/O and cache guards. Abandonment does neither.
        drop(self.image.take());
        let slot = { mem::replace(&mut self.cache.entries.lock()[self.index], Slot::Empty) };
        assert!(matches!(slot, Slot::Preparing { .. }));
        drop(slot);
        drop(self.charge.take());
    }

    pub(super) fn publish(mut self) -> &'a [u8] {
        let cache = self.cache;
        let name = {
            let mut entries = cache.entries.lock();
            let Slot::Preparing {
                name,
            } = mem::replace(&mut entries[self.index], Slot::Empty)
            else {
                panic!("image reservation missing")
            };
            entries[self.index] = Slot::Published {
                name,
                image: self.image.take().expect("image missing"),
                _charge: self.charge.take().expect("image charge missing"),
            };
            name
        };
        cache.get(name).expect("published image missing")
    }
}

#[cfg(test)]
#[path = "image_cache/tests.rs"]
mod tests;
