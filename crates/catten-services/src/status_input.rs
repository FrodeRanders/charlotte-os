//! Bounded status headers; each TLS connection keeps its original input budget.
use alloc::vec::Vec;

use zeroize::Zeroizing;

use crate::{
    deadline::Deadline,
    deployment_http::HEADER_LIMIT,
    deployment_input::{
        HEADER_MS,
        MAX_CLIENTS,
    },
    management_io::{
        self,
        Advance,
        Connection,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    NodeDashboard,
    NodeMetrics,
    ClusterDashboard,
    ClusterMetrics,
    Other,
}

struct Header(Zeroizing<Vec<u8>>);
impl Header {
    fn new() -> Result<Self, ()> {
        let mut bytes = Zeroizing::new(Vec::new());
        bytes.try_reserve_exact(HEADER_LIMIT).map_err(|_| ())?;
        Ok(Self(bytes))
    }

    fn append(&mut self, bytes: &[u8]) -> Result<Option<Target>, ()> {
        if bytes.len() > HEADER_LIMIT - self.0.len() {
            return Err(());
        }
        self.0.extend_from_slice(bytes);
        if !self.0.windows(4).any(|end| end == b"\r\n\r\n") {
            return Ok(None);
        }
        let target = match management_io::keyhole_path(&self.0).ok_or(())? {
            b"/" | b"/index.html" => Target::NodeDashboard,
            b"/metrics" | b"/metric" => Target::NodeMetrics,
            b"/cluster" | b"/cluster/" | b"/cluster/index.html" => Target::ClusterDashboard,
            b"/cluster/metrics" => Target::ClusterMetrics,
            _ => Target::Other,
        };
        Ok(Some(target))
    }
}

trait Calls {
    type Socket;
    type Reply;
    fn begin(socket: &mut Self::Socket, cq: u32) -> Result<(), ()>;
    fn poll(
        &mut self,
        socket: &mut Self::Socket,
        deadline: &Deadline,
    ) -> Result<Advance<Self::Reply>, ()>;
    fn append(
        socket: &Self::Socket,
        header: &mut Header,
        reply: Self::Reply,
    ) -> Result<Option<Target>, ()>;
    fn remaining(socket: &Self::Socket) -> u64;
}

struct Input<S> {
    // The complete connection owns/cancels its pending RPC before session/socket
    // destruction. Header storage has no borrowed or scalar remote resource.
    socket: S,
    header: Header,
    deadline: Deadline,
}
type Finished<S> = (S, Result<Target, ()>);
struct Machine<C: Calls> {
    inputs: [Option<Input<C::Socket>>; MAX_CLIENTS],
    calls: C,
    cursor: usize,
    cq: u32,
}
impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            inputs: core::array::from_fn(|_| None),
            calls,
            cursor: 0,
            cq,
        }
    }

    fn admit(&mut self, socket: C::Socket, now: u64, frequency: u64) -> Result<(), C::Socket> {
        let Some(slot) = self.inputs.iter_mut().find(|slot| slot.is_none()) else {
            return Err(socket);
        };
        let Ok(header) = Header::new() else {
            return Err(socket);
        };
        let mut input = Input {
            socket,
            header,
            deadline: Deadline::at(now, frequency, HEADER_MS),
        };
        if C::begin(&mut input.socket, self.cq).is_err() {
            return Err(input.socket);
        }
        *slot = Some(input);
        Ok(())
    }

    fn advance(&mut self, now: u64) -> (bool, Option<Finished<C::Socket>>) {
        for offset in 0..MAX_CLIENTS {
            let index = (self.cursor + offset) % MAX_CLIENTS;
            let Some(input) = self.inputs[index].as_mut() else {
                continue;
            };
            let result = if input.deadline.expired_at(now) {
                Err(())
            } else {
                match self.calls.poll(&mut input.socket, &input.deadline) {
                    Ok(Advance::Waiting) => continue,
                    Ok(Advance::Progress) => {
                        self.cursor = (index + 1) % MAX_CLIENTS;
                        return (true, None);
                    }
                    Ok(Advance::Ready(reply)) => C::append(&input.socket, &mut input.header, reply),
                    Err(()) => Err(()),
                }
            };
            self.cursor = (index + 1) % MAX_CLIENTS;
            let result = match result {
                Ok(None) => return (true, None),
                Ok(Some(target)) => Ok(target),
                Err(()) => Err(()),
            };
            let Input {
                socket,
                ..
            } = self.inputs[index].take().unwrap();
            return (true, Some((socket, result)));
        }
        (false, None)
    }

    fn cancel_one(&mut self) -> Option<C::Socket> {
        let input = self.inputs.iter_mut().find(|slot| slot.is_some())?.take()?;
        // Return the same connection with its accepted RPC still owned. The
        // close pool confirms cancellation before consuming its socket.
        Some(input.socket)
    }

    fn occupied_slots(&self) -> usize {
        self.inputs.iter().flatten().count()
    }

    fn remaining_ms(&self, now: u64) -> Option<u64> {
        self.inputs
            .iter()
            .flatten()
            .map(|input| input.deadline.remaining_ms_at(now).min(C::remaining(&input.socket)))
            .min()
    }
}

struct Runtime<'a>(core::marker::PhantomData<&'a ()>);
impl<'a> Calls for Runtime<'a> {
    type Reply = management_io::Application;
    type Socket = Connection<'a>;

    fn begin(socket: &mut Self::Socket, cq: u32) -> Result<(), ()> {
        socket.begin_receive(cq).map_err(|_| ())
    }

    fn poll(
        &mut self,
        socket: &mut Self::Socket,
        deadline: &Deadline,
    ) -> Result<Advance<Self::Reply>, ()> {
        socket.poll_receive(deadline).map_err(|_| ())
    }

    fn append(
        socket: &Self::Socket,
        header: &mut Header,
        (identity, bytes): Self::Reply,
    ) -> Result<Option<Target>, ()> {
        let target = header.append(&bytes)?;
        if target.is_some() {
            let permission = charlotte_management_tls::Permission::Observe;
            let allowed = identity.allows(permission);
            socket.audit(identity, permission, allowed);
            if !allowed {
                return Err(());
            }
        }
        Ok(target)
    }

    fn remaining(socket: &Self::Socket) -> u64 {
        socket.remaining_ms()
    }
}

pub struct Inputs<'a>(Machine<Runtime<'a>>);
impl<'a> Inputs<'a> {
    pub fn new(cq: u32) -> Self {
        Self(Machine::new(Runtime(core::marker::PhantomData), cq))
    }

    // Return the complete rejected connection without another fallible allocation.
    #[allow(clippy::result_large_err)]
    pub fn admit(&mut self, socket: Connection<'a>) -> Result<(), Connection<'a>> {
        let (now, frequency) = catten_syscall::monotonic_clock();
        self.0.admit(socket, now, frequency)
    }

    pub fn advance(&mut self) -> (bool, Option<Finished<Connection<'a>>>) {
        self.0.advance(catten_syscall::monotonic_clock().0)
    }

    pub fn cancel_one(&mut self) -> Option<Connection<'a>> {
        self.0.cancel_one()
    }

    pub fn occupied_slots(&self) -> usize {
        self.0.occupied_slots()
    }

    pub fn remaining_ms(&self) -> Option<u64> {
        self.0.remaining_ms(catten_syscall::monotonic_clock().0)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::VecDeque,
        vec,
    };

    use super::*;
    struct Fake([VecDeque<Advance<Vec<u8>>>; MAX_CLIENTS]);
    impl Calls for Fake {
        type Reply = Vec<u8>;
        type Socket = usize;

        fn begin(_: &mut usize, cq: u32) -> Result<(), ()> {
            assert_eq!(cq, 7);
            Ok(())
        }

        fn poll(&mut self, socket: &mut usize, _: &Deadline) -> Result<Advance<Vec<u8>>, ()> {
            Ok(self.0[*socket].pop_front().unwrap_or(Advance::Waiting))
        }

        fn append(_: &usize, header: &mut Header, bytes: Vec<u8>) -> Result<Option<Target>, ()> {
            header.append(&bytes)
        }

        fn remaining(_: &usize) -> u64 {
            u64::MAX
        }
    }
    fn machine() -> Machine<Fake> {
        Machine::new(Fake(core::array::from_fn(|_| VecDeque::new())), 7)
    }
    #[test]
    fn idle_peer_does_not_hide_a_complete_header() {
        let mut m = machine();
        m.admit(0, 0, 1000).unwrap();
        m.admit(1, 0, 1000).unwrap();
        m.calls.0[1]
            .push_back(Advance::Ready(b"GET /metrics HTTP/1.1\r\nHost: test\r\n\r\n".to_vec()));
        assert_eq!(m.advance(2000), (true, Some((1, Ok(Target::NodeMetrics)))));
        assert_eq!(m.remaining_ms(2000), Some(3000));
        assert_eq!(m.occupied_slots(), 1);
        assert_eq!(m.cancel_one(), Some(0));
    }
    #[test]
    fn fragments_and_unrelated_progress_keep_the_original_deadline() {
        let mut m = machine();
        m.admit(0, 0, 1000).unwrap();
        m.calls.0[0].extend([Advance::Ready(b"GET /metrics ".to_vec()), Advance::Progress]);
        assert_eq!(m.advance(4000), (true, None));
        assert_eq!(m.advance(4999), (true, None));
        assert_eq!(m.remaining_ms(4999), Some(1));
        assert_eq!(m.advance(5000), (true, Some((0, Err(())))));
    }
    #[test]
    fn readable_inputs_rotate_and_capacity_returns_only_on_handoff() {
        let mut m = machine();
        for id in 0..MAX_CLIENTS {
            m.admit(id, 0, 1000).unwrap();
            m.calls.0[id].extend([Advance::Progress, Advance::Progress]);
        }
        assert_eq!(m.admit(9, 0, 1000), Err(9));
        for expected in (0..MAX_CLIENTS).cycle().take(MAX_CLIENTS * 2) {
            assert_eq!(m.cursor, expected);
            assert_eq!(m.advance(1), (true, None));
        }
        assert_eq!(m.occupied_slots(), MAX_CLIENTS);
        assert_eq!(m.cancel_one(), Some(0));
        m.admit(0, 1, 1000).unwrap();
    }
    #[test]
    fn exact_headers_reject_overflow_bodies_and_pipeline_suffixes() {
        for bytes in [
            vec![b'x'; HEADER_LIMIT + 1],
            b"GET / HTTP/1.1\r\nHost: test\r\nContent-Length: 1\r\n\r\nx".to_vec(),
            b"GET / HTTP/1.1\r\nHost: test\r\n\r\nextra".to_vec(),
        ] {
            assert!(Header::new().unwrap().append(&bytes).is_err());
        }
        let mut header = Header::new().unwrap();
        assert_eq!(header.append(b"GET /cluster/metrics HTTP/1.1\r\nHost:"), Ok(None));
        assert_eq!(header.append(b" test\r\n\r\n"), Ok(Some(Target::ClusterMetrics)));
    }
}
