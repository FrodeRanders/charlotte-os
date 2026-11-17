//! Owned asynchronous full-name lookup and scalar invocation on one CQ.

use catten_rt::owned::{
    Connection,
    ConnectionRef,
    OwnedMemory,
    PendingCall,
};

use crate::{
    deadline::Deadline,
    dns,
    ns,
};

struct Reply<C, M> {
    result: i64,
    connection: Option<C>,
    _memory: Option<M>,
}

type PollResult<C, M> = Result<Option<Reply<C, M>>, ()>;

trait Calls {
    type Pending;
    type Connection;
    type Memory;
    fn now(&self) -> u64;
    fn bind(&mut self, pending: &mut Self::Pending, cq: u32) -> Result<(), ()>;
    fn poll(&mut self, pending: &mut Self::Pending) -> PollResult<Self::Connection, Self::Memory>;
    fn invoke(
        &mut self,
        connection: &Self::Connection,
        opcode: u32,
        arg: i64,
    ) -> Result<Self::Pending, i64>;
}

enum Phase<P, C> {
    Lookup(P),
    // Cancel/observe the pending RPC before releasing its connection.
    Invoke {
        pending: P,
        _connection: C,
    },
}

struct Machine<R: Calls> {
    phase: Option<Phase<R::Pending, R::Connection>>,
    calls: R,
    opcode: u32,
    arg: i64,
    deadline: Deadline,
    cq: u32,
}

impl<R: Calls> Machine<R> {
    #[allow(clippy::too_many_arguments)]
    fn begin(
        mut calls: R,
        name: &[u8],
        opcode: u32,
        arg: i64,
        deadline: Deadline,
        cq: u32,
        lookup: impl FnOnce(&mut R, &[u8]) -> Result<R::Pending, i64>,
    ) -> Result<Self, i64> {
        if name.is_empty() || name.len() > crate::MAX_NAME_LEN {
            return Err(dns::ERR_TOO_LARGE);
        }
        if deadline.expired_at(calls.now()) {
            return Err(dns::ERR_NOT_FOUND);
        }
        let pending = lookup(&mut calls, name)?;
        let mut owner = Self {
            phase: Some(Phase::Lookup(pending)),
            calls,
            opcode,
            arg,
            deadline,
            cq,
        };
        if owner.deadline.expired_at(owner.calls.now()) {
            return Err(dns::ERR_NOT_FOUND);
        }
        let Some(Phase::Lookup(pending)) = owner.phase.as_mut() else {
            unreachable!()
        };
        owner.calls.bind(pending, cq).map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        Ok(owner)
    }

    fn advance(&mut self) -> Option<i64> {
        let phase = self.phase.as_mut()?;
        if self.deadline.expired_at(self.calls.now()) {
            let result = match phase {
                Phase::Lookup(_) => dns::ERR_NOT_FOUND,
                Phase::Invoke {
                    ..
                } => dns::ERR_UNCERTAIN,
            };
            drop(self.phase.take());
            return Some(result);
        }
        let pending = match phase {
            Phase::Lookup(pending)
            | Phase::Invoke {
                pending,
                ..
            } => pending,
        };
        let reply = match self.calls.poll(pending) {
            Ok(None) => return None,
            result => result,
        };
        match self.phase.take().unwrap() {
            Phase::Lookup(pending) => {
                drop(pending);
                let Ok(Some(mut reply)) = reply else {
                    return Some(dns::ERR_NOT_FOUND);
                };
                if reply.result < 1 {
                    return Some(dns::ERR_NOT_FOUND);
                }
                let Some(connection) = reply.connection.take() else {
                    return Some(dns::ERR_NOT_FOUND);
                };
                drop(reply); // Dispose every unused returned resource before the next RPC.
                if self.deadline.expired_at(self.calls.now()) {
                    return Some(dns::ERR_NOT_FOUND);
                }
                let pending = match self.calls.invoke(&connection, self.opcode, self.arg) {
                    Ok(pending) => pending,
                    Err(code) => return Some(code),
                };
                // Retain the accepted operation before binding readiness. Binding
                // rejection is uncertain invocation, never permission to replay.
                self.phase = Some(Phase::Invoke {
                    pending,
                    _connection: connection,
                });
                let Some(Phase::Invoke {
                    pending,
                    ..
                }) = self.phase.as_mut()
                else {
                    unreachable!()
                };
                if self.deadline.expired_at(self.calls.now())
                    || self.calls.bind(pending, self.cq).is_err()
                {
                    drop(self.phase.take());
                    return Some(dns::ERR_UNCERTAIN);
                }
                None
            }
            Phase::Invoke {
                pending,
                _connection,
            } => {
                drop(pending);
                drop(_connection);
                Some(match reply {
                    Ok(Some(reply)) => reply.result,
                    _ => dns::ERR_UNCERTAIN,
                })
            }
        }
    }
}

struct KernelCalls;
impl Calls for KernelCalls {
    type Connection = Connection;
    type Memory = OwnedMemory;
    type Pending = PendingCall<'static>;

    fn now(&self) -> u64 {
        catten_syscall::monotonic_clock().0
    }

    fn bind(&mut self, pending: &mut Self::Pending, cq: u32) -> Result<(), ()> {
        pending.bind_cq(cq).map_err(|_| ())
    }

    fn poll(&mut self, pending: &mut Self::Pending) -> PollResult<Connection, OwnedMemory> {
        pending
            .poll()
            .map(|reply| {
                reply.map(|reply| Reply {
                    result: reply.result,
                    connection: reply.connection,
                    _memory: reply.memory,
                })
            })
            .map_err(|_| ())
    }

    fn invoke(
        &mut self,
        connection: &Connection,
        opcode: u32,
        arg: i64,
    ) -> Result<Self::Pending, i64> {
        connection.call(opcode, arg as u64).map_err(|error| match error {
            catten_rt::owned::IpcError::Status(catten_syscall::ipc_status::RESOURCE_LIMIT) => {
                dns::ERR_RESOURCE_LIMIT
            }
            _ => dns::ERR_NOT_FOUND,
        })
    }
}

/// One retaining operation. Drop uses the runtime's cancel/close policy,
/// including domain abort when rejected cleanup cannot release an owner safely.
pub struct LocalInvocation(Machine<KernelCalls>);
impl LocalInvocation {
    pub fn begin(
        names: ConnectionRef<'_>,
        name: &[u8],
        opcode: u32,
        arg: i64,
        deadline: Deadline,
        cq: u32,
    ) -> Result<Self, i64> {
        Machine::begin(KernelCalls, name, opcode, arg, deadline, cq, |_, name| {
            let memory = crate::stage_name_owned(name).ok_or(dns::ERR_RESOURCE_LIMIT)?;
            names.call_move(ns::OP_TRY_LOOKUP_NAMED, name.len() as u64, memory).map_err(
                |(_memory, error)| match error {
                    catten_rt::owned::IpcError::Status(
                        catten_syscall::ipc_status::RESOURCE_LIMIT,
                    ) => dns::ERR_RESOURCE_LIMIT,
                    _ => dns::ERR_NOT_FOUND,
                },
            )
        })
        .map(Self)
    }

    /// Observe once when the shared reactor wakes, without parking it on one RPC.
    pub fn advance(&mut self) -> Option<i64> {
        self.0.advance()
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
        vec::Vec,
    };
    use core::cell::{
        Cell,
        RefCell,
    };

    use super::*;

    #[derive(Default)]
    struct Ledger {
        live: usize,
        dropped: Vec<&'static str>,
        names: Vec<Vec<u8>>,
        invoked: usize,
        binds: Vec<(&'static str, u32)>,
    }
    struct Resource(&'static str, Rc<RefCell<Ledger>>);
    impl Resource {
        fn new(label: &'static str, ledger: &Rc<RefCell<Ledger>>) -> Self {
            ledger.borrow_mut().live += 1;
            Self(label, ledger.clone())
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            let mut ledger = self.1.borrow_mut();
            ledger.live -= 1;
            ledger.dropped.push(self.0);
        }
    }
    enum Event {
        Pending,
        Ready(i64, Option<Resource>, Option<Resource>),
        Error,
    }
    struct Call {
        owner: Resource,
        events: VecDeque<Event>,
    }
    struct Fake {
        ledger: Rc<RefCell<Ledger>>,
        now: Rc<Cell<u64>>,
        lookup: VecDeque<Event>,
        invocation: VecDeque<Event>,
        bind_failure: Option<&'static str>,
        invoke_failure: bool,
        lookup_delay: u64,
        invoke_delay: u64,
    }
    impl Calls for Fake {
        type Connection = Resource;
        type Memory = Resource;
        type Pending = Call;

        fn now(&self) -> u64 {
            self.now.get()
        }

        fn bind(&mut self, pending: &mut Call, cq: u32) -> Result<(), ()> {
            self.ledger.borrow_mut().binds.push((pending.owner.0, cq));
            if self.bind_failure == Some(pending.owner.0) {
                Err(())
            } else {
                Ok(())
            }
        }

        fn poll(&mut self, pending: &mut Call) -> Result<Option<Reply<Resource, Resource>>, ()> {
            match pending.events.pop_front().unwrap_or(Event::Pending) {
                Event::Pending => Ok(None),
                Event::Error => Err(()),
                Event::Ready(result, connection, memory) => Ok(Some(Reply {
                    result,
                    connection,
                    _memory: memory,
                })),
            }
        }

        fn invoke(&mut self, _: &Resource, opcode: u32, arg: i64) -> Result<Call, i64> {
            assert_eq!((opcode, arg), (7, -42));
            self.ledger.borrow_mut().invoked += 1;
            self.now.set(self.now.get() + self.invoke_delay);
            if self.invoke_failure {
                return Err(dns::ERR_RESOURCE_LIMIT);
            }
            Ok(Call {
                owner: Resource::new("invoke", &self.ledger),
                events: core::mem::take(&mut self.invocation),
            })
        }
    }
    fn backend() -> Fake {
        Fake {
            ledger: Rc::default(),
            now: Rc::new(Cell::new(0)),
            lookup: VecDeque::new(),
            invocation: VecDeque::new(),
            bind_failure: None,
            invoke_failure: false,
            lookup_delay: 0,
            invoke_delay: 0,
        }
    }
    fn begin(fake: Fake, name: &[u8]) -> Result<Machine<Fake>, i64> {
        Machine::begin(fake, name, 7, -42, Deadline::at(0, 1000, 10), 3, |fake, name| {
            fake.ledger.borrow_mut().names.push(name.to_vec());
            fake.now.set(fake.now.get() + fake.lookup_delay);
            Ok(Call {
                owner: Resource::new("lookup", &fake.ledger),
                events: core::mem::take(&mut fake.lookup),
            })
        })
    }
    fn successful_lookup(fake: &mut Fake) {
        fake.lookup.push_back(Event::Ready(
            1,
            Some(Resource::new("connection", &fake.ledger)),
            Some(Resource::new("lookup-memory", &fake.ledger)),
        ));
    }

    #[test]
    fn full_name_success_binds_both_phases_and_disposes_all_returned_resources() {
        let mut fake = backend();
        let ledger = fake.ledger.clone();
        successful_lookup(&mut fake);
        fake.invocation.push_back(Event::Ready(
            42,
            Some(Resource::new("returned-connection", &ledger)),
            Some(Resource::new("returned-memory", &ledger)),
        ));
        let name = vec![b'n'; 256];
        let mut call = begin(fake, &name).ok().unwrap();
        assert_eq!(call.advance(), None);
        assert_eq!(call.advance(), Some(42));
        assert_eq!(call.advance(), None);
        drop(call);
        let ledger = ledger.borrow();
        assert_eq!(ledger.names, [name]);
        assert_eq!(ledger.invoked, 1);
        assert_eq!(ledger.binds, [("lookup", 3), ("invoke", 3)]);
        assert_eq!(ledger.live, 0);
        assert!(
            ledger.dropped.iter().position(|x| *x == "invoke")
                < ledger.dropped.iter().position(|x| *x == "connection")
        );
    }
    #[test]
    fn invalid_names_and_expired_admission_submit_nothing() {
        for name in [b"".as_slice(), &vec![0; 257]] {
            let fake = backend();
            let ledger = fake.ledger.clone();
            assert_eq!(begin(fake, name).err(), Some(dns::ERR_TOO_LARGE));
            assert!(ledger.borrow().names.is_empty());
        }
        let fake = backend();
        let ledger = fake.ledger.clone();
        fake.now.set(10);
        assert_eq!(begin(fake, b"long-name\0").err(), Some(dns::ERR_NOT_FOUND));
        assert!(ledger.borrow().names.is_empty());
    }
    #[test]
    fn lookup_rejections_never_invoke_and_release_returned_owners() {
        for event in [Event::Error, Event::Ready(-1, None, None), Event::Ready(1, None, None)] {
            let mut fake = backend();
            let ledger = fake.ledger.clone();
            fake.lookup.push_back(event);
            let mut call = begin(fake, b"long-name\0").ok().unwrap();
            assert_eq!(call.advance(), Some(dns::ERR_NOT_FOUND));
            drop(call);
            assert_eq!(ledger.borrow().invoked, 0);
            assert_eq!(ledger.borrow().live, 0);
        }
        let mut fake = backend();
        let ledger = fake.ledger.clone();
        fake.bind_failure = Some("lookup");
        assert_eq!(begin(fake, b"long-name").err(), Some(dns::ERR_RESOURCE_LIMIT));
        assert_eq!(ledger.borrow().live, 0);
    }
    #[test]
    fn invoke_submission_and_binding_rejections_preserve_uncertainty_without_replay() {
        for failure in ["submit", "bind"] {
            let mut fake = backend();
            let ledger = fake.ledger.clone();
            successful_lookup(&mut fake);
            fake.invoke_failure = failure == "submit";
            fake.bind_failure = Some("invoke");
            let mut call = begin(fake, b"long-name").ok().unwrap();
            assert_eq!(
                call.advance(),
                Some(
                    if failure == "submit" {
                        dns::ERR_RESOURCE_LIMIT
                    } else {
                        dns::ERR_UNCERTAIN
                    }
                )
            );
            assert_eq!(call.advance(), None);
            drop(call);
            assert_eq!(ledger.borrow().invoked, 1);
            assert_eq!(ledger.borrow().live, 0);
        }
    }
    #[test]
    fn pending_wakes_keep_original_deadline_and_timeout_releases_complete_phase() {
        for invoke in [false, true] {
            let mut fake = backend();
            let ledger = fake.ledger.clone();
            let now = fake.now.clone();
            if invoke {
                successful_lookup(&mut fake);
            }
            let mut call = begin(fake, b"long-name").ok().unwrap();
            assert_eq!(call.advance(), None);
            now.set(9);
            assert_eq!(call.advance(), None);
            now.set(10);
            assert_eq!(
                call.advance(),
                Some(
                    if invoke {
                        dns::ERR_UNCERTAIN
                    } else {
                        dns::ERR_NOT_FOUND
                    }
                )
            );
            drop(call);
            assert_eq!(ledger.borrow().live, 0);
            assert_eq!(ledger.borrow().invoked, usize::from(invoke));
        }
    }
    #[test]
    fn drop_cancels_each_pending_phase_and_unobserved_returned_resources() {
        for invoke in [false, true] {
            let mut fake = backend();
            let ledger = fake.ledger.clone();
            successful_lookup(&mut fake);
            fake.invocation.push_back(Event::Ready(
                42,
                None,
                Some(Resource::new("unobserved-memory", &ledger)),
            ));
            let mut call = begin(fake, b"long-name").ok().unwrap();
            if invoke {
                assert_eq!(call.advance(), None);
            }
            drop(call);
            assert_eq!(ledger.borrow().live, 0);
        }
    }
    #[test]
    fn preparation_and_accepted_submission_cannot_restart_budget() {
        let mut fake = backend();
        let ledger = fake.ledger.clone();
        fake.lookup_delay = 10;
        assert_eq!(begin(fake, b"long-name").err(), Some(dns::ERR_NOT_FOUND));
        assert_eq!(ledger.borrow().live, 0);
        let mut fake = backend();
        let ledger = fake.ledger.clone();
        successful_lookup(&mut fake);
        fake.invoke_delay = 10;
        let mut call = begin(fake, b"long-name").ok().unwrap();
        assert_eq!(call.advance(), Some(dns::ERR_UNCERTAIN));
        drop(call);
        assert_eq!(ledger.borrow().invoked, 1);
        assert_eq!(ledger.borrow().live, 0);
    }
}
