//! Complete thread publication owns the candidate backing through the final
//! generation/abort/limit check and post-guard old/candidate deallocation.
use super::*;
use crate::klib::collections::id_table::PreparedGrowth;

struct Preparation {
    thread: Option<Thread>,
    growth: Option<PreparedGrowth<Thread, crate::memory::budget::thread_table::Charge>>,
}

impl Preparation {
    fn prepare_growth(&mut self) -> Result<(), Error> {
        let request = { MASTER_THREAD_TABLE.read().growth_request() }
            .map_err(|_| Error::ThreadPreparationFailed)?;
        if let Some(request) = request {
            let thread = self.thread.as_ref().unwrap();
            let root = thread
                .address_space
                .or_else(|| {
                    (thread.asid == crate::memory::KERNEL_ASID)
                        .then(|| {
                            crate::memory::current_address_space_handle(crate::memory::KERNEL_ASID)
                        })
                        .flatten()
                })
                .ok_or(Error::ThreadTerminated)?;
            let amount = request.amount().map_err(|_| Error::ThreadPreparationFailed)?;
            let charge = crate::memory::budget::reserve_thread_table(root, amount)
                .map_err(|_| Error::ThreadPreparationFailed)?;
            self.growth =
                Some(request.prepare(charge).map_err(|_| Error::ThreadPreparationFailed)?);
        }
        Ok(())
    }

    fn finish_growth(&mut self) {
        if let Some(growth) = self.growth.take() {
            growth.finish();
        }
    }

    fn finish(&mut self) {
        self.finish_growth();
        if let Some(thread) = self.thread.take() {
            let _ = thread.release_unstarted();
        }
    }
}

/// Publication shares the exact root's abort gate. Preparation and disposal
/// run before/after lifecycle, publication and master-table serialization.
pub fn publish_thread(thread: Thread) -> Result<ThreadId, Error> {
    let mut preparation = Preparation {
        thread: Some(thread),
        growth: None,
    };
    // Optimistic admission can lose capacity to another creator. Only this
    // typed, proven-unstarted rejection permits bounded re-preparation. This
    // never waits for quota or retries accepted work; all other errors stop.
    for _ in 0..8 {
        if let Err(error) = preparation.prepare_growth() {
            preparation.finish();
            return Err(error);
        }
        match try_publish(&mut preparation) {
            Ok(tid) => {
                preparation.finish();
                return Ok(tid);
            }
            Err((error, capacity_changed)) => {
                preparation.finish_growth();
                if !capacity_changed {
                    preparation.finish();
                    return Err(error);
                }
            }
        }
    }
    preparation.finish();
    Err(Error::ThreadPreparationFailed)
}

fn try_publish(preparation: &mut Preparation) -> Result<ThreadId, (Error, bool)> {
    let (asid, root) = {
        let thread = preparation.thread.as_ref().unwrap();
        (thread.asid, thread.address_space)
    };
    // The complete owner stays in preparation across every rejected check.
    (|| {
        let _lifecycle = (asid != crate::memory::KERNEL_ASID)
            .then(|| crate::memory::ADDRESS_SPACE_LIFECYCLE.lock());
        let _publication_gate = THREAD_PUBLICATION_GATE.lock();
        let maximum = if asid != crate::memory::KERNEL_ASID {
            let Some(handle) = root else {
                return Err((Error::ThreadTerminated, false));
            };
            if handle.id() != asid || !thread_admission_open(handle) {
                return Err((Error::ThreadTerminated, false));
            }
            Some(crate::memory::domain_limits(asid).max_threads)
        } else {
            None
        };
        let mut table = MASTER_THREAD_TABLE.write();
        if maximum.is_some_and(|maximum| {
            table
                .iter()
                .filter(|entry| entry.as_ref().is_some_and(|thread| thread.asid == asid))
                .count()
                >= maximum
        }) {
            return Err((Error::DomainThreadLimitExceeded, false));
        }
        table.try_insert_from(&mut preparation.thread, preparation.growth.as_mut()).map_err(
            |error| {
                (
                    Error::ThreadPreparationFailed,
                    error == crate::klib::collections::id_table::Error::CapacityChanged,
                )
            },
        )
    })()
}
