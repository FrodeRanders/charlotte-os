//! Full-name generation requests and one-shot CQ-bound retirement ownership.
use catten_rt::owned::{
    ConnectionRef,
    OwnedMemory,
    PendingCall,
};

use crate::{
    deadline::Deadline,
    ns,
    ns_generation::{
        self,
        Action,
        Request,
    },
};

const TIMEOUT_MS: u64 = 5_000;
#[derive(Debug)]
pub enum Error {
    Invalid,
    ResourceLimit,
    Submission,
    WaitRejected,
    Timeout,
    BadReply,
}

fn prepare(action: Action, name: &[u8], generation: u64) -> Result<(OwnedMemory, u64), Error> {
    let bytes = ns_generation::encode(
        action,
        Request {
            name,
            generation,
        },
    )
    .map_err(|error| match error {
        ns_generation::EncodeError::Invalid => Error::Invalid,
        ns_generation::EncodeError::AllocationFailed => Error::ResourceLimit,
    })?;
    let memory = OwnedMemory::allocate(1).map_err(|_| Error::ResourceLimit)?;
    let mut mapping = memory.map_writable().map_err(|_| Error::ResourceLimit)?;
    mapping.as_mut_slice()[..bytes.len()].copy_from_slice(&bytes);
    let memory = mapping.unmap().map_err(|_| Error::ResourceLimit)?;
    Ok((memory, bytes.len() as u64))
}

/// Immediate metadata observation, never a deferred lookup or invocation grant.
/// Rejection is distinct from a confirmed absent/dead publication.
pub fn observe(names: ConnectionRef<'_>, name: &[u8]) -> Result<Option<u64>, Error> {
    let deadline = Deadline::after(TIMEOUT_MS);
    let (memory, length) = prepare(Action::Observe, name, 0)?;
    if deadline.expired() {
        return Err(Error::Timeout);
    }
    let mut pending = names
        .call_move(ns::OP_PUBLICATION_GENERATION, length, memory)
        .map_err(|_| Error::Submission)?;
    let result = pending
        .wait_timeout(deadline.remaining_ms())
        .map_err(|_| Error::WaitRejected)?
        .ok_or(Error::Timeout)?;
    if result.connection.is_some() || result.memory.is_some() {
        return Err(Error::BadReply);
    }
    match result.result {
        ns::ERR_NOT_FOUND => Ok(None),
        generation if generation > 0 => Ok(Some(generation as u64)),
        _ => Err(Error::BadReply),
    }
}

/// Registry-generation and stable-principal authorization. A missing/zero
/// registrar never grants ownership; the manager bit comes from the kernel.
pub fn retirement_allowed(
    current: u64,
    expected: u64,
    registrar: Option<u64>,
    actor: Option<u64>,
    manager: bool,
) -> bool {
    expected != 0
        && current == expected
        && actor
            .is_some_and(|principal| principal != 0 && (manager || registrar == Some(principal)))
}

trait Call {
    fn bind(&mut self, cq: u32) -> Result<(), ()>;
    fn poll(&mut self) -> Result<Option<i64>, ()>;
}
impl Call for PendingCall<'static> {
    fn bind(&mut self, cq: u32) -> Result<(), ()> {
        self.bind_cq(cq).map_err(|_| ())
    }

    fn poll(&mut self) -> Result<Option<i64>, ()> {
        // CallResult owns and drops every unused returned resource, including
        // unexpected attachments on a scalar retirement reply.
        self.poll().map(|result| result.map(|reply| reply.result)).map_err(|_| ())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Removed,
    NoMatch,
    Rejected(i64),
    Uncertain,
}
struct Operation<P> {
    pending: Option<P>,
    generation: u64,
    deadline: Deadline,
}
impl<P: Call> Operation<P> {
    fn bind(
        pending: P,
        generation: u64,
        deadline: Deadline,
        cq: u32,
        now: u64,
    ) -> Result<Self, Self> {
        let mut owner = Self {
            pending: Some(pending),
            generation,
            deadline,
        };
        if deadline.expired_at(now) || owner.pending.as_mut().unwrap().bind(cq).is_err() {
            Err(owner)
        } else {
            Ok(owner)
        }
    }

    fn advance_at(&mut self, now: u64) -> Option<Outcome> {
        let pending = self.pending.as_mut()?;
        let outcome = if self.deadline.expired_at(now) {
            Outcome::Uncertain
        } else {
            match pending.poll() {
                Ok(None) => return None,
                Ok(Some(result)) if u64::try_from(result) == Ok(self.generation) => {
                    Outcome::Removed
                }
                Ok(Some(ns::ERR_NOT_FOUND)) => Outcome::NoMatch,
                Ok(Some(result)) if result < 0 => Outcome::Rejected(result),
                _ => Outcome::Uncertain,
            }
        };
        drop(self.pending.take());
        Some(outcome)
    }
}
/// One accepted retirement RPC. No outcome, wake, timeout or Drop permits replay.
/// Drop uses the existing runtime cancellation policy; rejected release aborts
/// the domain while retaining its uncertain resources.
pub struct Unregistration(Operation<PendingCall<'static>>);
pub enum BeginError {
    Unstarted(Error),
    Accepted(Unregistration),
}
impl Unregistration {
    pub fn begin(
        names: ConnectionRef<'_>,
        name: &[u8],
        generation: u64,
        cq: u32,
    ) -> Result<Self, BeginError> {
        let deadline = Deadline::after(TIMEOUT_MS);
        let (memory, length) =
            prepare(Action::Retire, name, generation).map_err(BeginError::Unstarted)?;
        if deadline.expired() {
            return Err(BeginError::Unstarted(Error::Timeout));
        }
        let pending = names
            .call_move(ns::OP_UNREGISTER_GENERATION, length, memory)
            .map_err(|_| BeginError::Unstarted(Error::Submission))?;
        Operation::bind(pending, generation, deadline, cq, catten_syscall::monotonic_clock().0)
            .map(Self)
            .map_err(|owner| BeginError::Accepted(Self(owner)))
    }

    pub fn advance(&mut self) -> Option<Outcome> {
        self.0.advance_at(catten_syscall::monotonic_clock().0)
    }

    pub fn remaining_ms(&self) -> u64 {
        self.0.deadline.remaining_ms()
    }
}
#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::{
        collections::VecDeque,
        rc::Rc,
        vec,
    };
    use core::cell::{
        Cell,
        RefCell,
    };

    use super::*;
    struct Fake {
        replies: VecDeque<Result<Option<i64>, ()>>,
        dropped: Rc<Cell<u32>>,
        binds: Rc<RefCell<alloc::vec::Vec<u32>>>,
        reject: bool,
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get() + 1);
        }
    }
    impl Call for Fake {
        fn bind(&mut self, cq: u32) -> Result<(), ()> {
            self.binds.borrow_mut().push(cq);
            if self.reject {
                Err(())
            } else {
                Ok(())
            }
        }

        fn poll(&mut self) -> Result<Option<i64>, ()> {
            self.replies.pop_front().unwrap_or(Ok(None))
        }
    }
    fn fake(replies: &[Result<Option<i64>, ()>], reject: bool) -> Fake {
        Fake {
            replies: replies.iter().copied().collect(),
            dropped: Rc::new(Cell::new(0)),
            binds: Rc::new(RefCell::new(vec![])),
            reject,
        }
    }
    #[test]
    fn exact_generation_and_authenticated_principal_are_required() {
        assert!(retirement_allowed(7, 7, Some(42), Some(42), false));
        assert!(retirement_allowed(7, 7, Some(42), Some(9), true));
        for (current, expected, registrar, actor, manager) in [
            (7, 8, Some(42), Some(42), true),
            (7, 0, Some(42), Some(42), true),
            (7, 7, Some(42), Some(9), false),
            (7, 7, None, Some(9), false),
            (7, 7, None, None, true),
            (7, 7, Some(0), Some(0), true),
        ] {
            assert!(!retirement_allowed(current, expected, registrar, actor, manager));
        }
    }
    #[test]
    fn readiness_and_terminal_results_close_once_without_replay() {
        for (reply, outcome) in [
            (Ok(Some(7)), Outcome::Removed),
            (Ok(Some(-1)), Outcome::NoMatch),
            (Ok(Some(-2)), Outcome::Rejected(-2)),
            (Ok(Some(8)), Outcome::Uncertain),
            (Err(()), Outcome::Uncertain),
        ] {
            let call = fake(&[Ok(None), reply], false);
            let drops = call.dropped.clone();
            let binds = call.binds.clone();
            let mut owner = Operation::bind(call, 7, Deadline::at(0, 1000, 50), 3, 0).ok().unwrap();
            assert_eq!(*binds.borrow(), vec![3]);
            assert_eq!(owner.advance_at(10), None);
            assert_eq!(owner.advance_at(11), Some(outcome));
            assert_eq!(owner.advance_at(12), None);
            drop(owner);
            assert_eq!(drops.get(), 1);
        }
    }
    #[test]
    fn binding_rejection_retains_the_accepted_owner() {
        let call = fake(&[], true);
        let drops = call.dropped.clone();
        let owner = Operation::bind(call, 7, Deadline::at(0, 1000, u64::MAX), 0, 0).err().unwrap();
        assert_eq!(drops.get(), 0);
        drop(owner);
        assert_eq!(drops.get(), 1);
    }
    #[test]
    fn unrelated_turns_timeout_and_drop_preserve_the_original_budget() {
        let call = fake(&[], false);
        let drops = call.dropped.clone();
        let mut owner = Operation {
            pending: Some(call),
            generation: 7,
            deadline: Deadline::at(10, 1000, 50),
        };
        for now in [10, 20, 59] {
            assert_eq!(owner.advance_at(now), None);
        }
        assert_eq!(owner.advance_at(60), Some(Outcome::Uncertain));
        assert_eq!(drops.get(), 1);
        let call = fake(&[], false);
        let drops = call.dropped.clone();
        drop(Operation {
            pending: Some(call),
            generation: 7,
            deadline: Deadline::at(0, 1000, 50),
        });
        assert_eq!(drops.get(), 1);
    }
}
