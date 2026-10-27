//! Owning, CQ-bound management TLS transport. Every accepted operation stays
//! with the original socket, session and deadline until observation/cancellation.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::{
    fmt,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    time::Duration,
};

use catten_rt::{
    Context,
    ShutdownRequest,
    owned::{
        Connection as ServiceConnection,
        IpcError,
        PendingCall,
    },
};
use charlotte_management_tls::{
    Identity,
    Permission,
    PreparedConfig,
    Session,
    TimeProvider,
    UnixTime,
};
use zeroize::Zeroizing;

use crate::{
    deadline::Deadline,
    socket::{
        OwnedSocket,
        SocketError,
    },
    tls_client,
};

pub enum Advance<T> {
    Waiting,
    Progress,
    Ready(T),
}
pub type Application = (Identity, Zeroizing<Vec<u8>>);
pub enum StartupError {
    MissingProfile,
    Credentials,
    Dependency,
    Entropy,
    Shutdown(ShutdownRequest),
}
pub enum Interrupted {
    Shutdown(ShutdownRequest),
    Transport(Error),
}
#[derive(Debug)]
pub enum Error {
    Socket(SocketError),
    Observation(IpcError),
    Tls(charlotte_management_tls::Error),
    Clock,
    State,
    Reply,
    Deadline,
}

#[derive(Clone, Copy)]
struct Anchor {
    seconds: u64,
    ticks: u64,
    frequency: u64,
}
#[derive(Default)]
struct Clock(spin::Mutex<Option<Anchor>>);
impl fmt::Debug for Clock {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("ManagementServiceClock")
    }
}
fn project(anchor: Anchor, now: u64) -> Option<u64> {
    if anchor.frequency == 0 || now < anchor.ticks {
        return None;
    }
    anchor.seconds.checked_add((now - anchor.ticks) / anchor.frequency)
}
impl Clock {
    fn install(&self, seconds: u64) -> Result<(), Error> {
        let (ticks, frequency) = catten_syscall::monotonic_clock();
        if seconds == 0 || frequency == 0 {
            return Err(Error::Clock);
        }
        let mut state = self.0.lock();
        if state.is_some_and(|previous| project(previous, ticks).is_none_or(|old| seconds < old)) {
            return Err(Error::Clock);
        }
        *state = Some(Anchor {
            seconds,
            ticks,
            frequency,
        });
        Ok(())
    }
}
impl TimeProvider for Clock {
    fn current_time(&self) -> Option<UnixTime> {
        let (now, _) = catten_syscall::monotonic_clock();
        let anchor = (*self.0.lock())?;
        let seconds = project(anchor, now)?;
        (seconds != 0).then(|| UnixTime::since_unix_epoch(Duration::from_secs(seconds)))
    }
}
fn milliseconds() -> Result<u64, Error> {
    let (ticks, frequency) = catten_syscall::monotonic_clock();
    if frequency == 0 {
        return Err(Error::Clock);
    }
    u64::try_from(u128::from(ticks) * 1000 / u128::from(frequency)).map_err(|_| Error::Clock)
}

/// One immutable credential factory and the original time-service connection.
/// Each accepted client must obtain a fresh positive time reply before activation;
/// an unavailable/unsynchronized source rejects that client, without polling.
pub struct Factory {
    prepared: PreparedConfig,
    clock: Arc<Clock>,
    time: ServiceConnection,
    next_id: AtomicU64,
}
impl Factory {
    pub fn load(ctx: &Context) -> Result<Self, StartupError> {
        let profile = ctx.profile_memory().ok_or(StartupError::MissingProfile)?;
        let mapping = profile.map_read_only().map_err(|_| StartupError::Credentials)?;
        let clock = Arc::new(Clock::default());
        let prepared = PreparedConfig::from_profile(mapping.as_slice(), clock.clone())
            .map_err(|_| StartupError::Credentials)?;
        drop(mapping);
        let names = ctx.bootstrap_connection().ok_or(StartupError::Dependency)?;
        let (_, time) =
            crate::wait_for_registered_name_or_shutdown_owned(ctx, names, crate::time::NAME)
                .map_err(StartupError::Shutdown)?;
        // Source discovery is from kernel-supported entropy, never a manifest
        // role/name classification. A missing architectural source waits for the
        // real RNG publication; it does not select deterministic fallback bytes.
        let architectural = Zeroizing::new(catten_syscall::random_u64());
        let service = if architectural.is_some() {
            crate::try_registered_name_owned(names, crate::entropy::NAME)
                .map(|(_, connection)| connection)
        } else {
            Some(
                crate::wait_for_registered_name_or_shutdown_owned(ctx, names, crate::entropy::NAME)
                    .map_err(StartupError::Shutdown)?
                    .1,
            )
        };
        tls_client::install_domain_entropy(tls_client::EntropyConfig {
            service,
            limits: tls_client::EntropyLimits::DEFAULT,
        })
        .map_err(|_| StartupError::Entropy)?;
        Ok(Self {
            prepared,
            clock,
            time,
            next_id: AtomicU64::new(1),
        })
    }
}

#[derive(Clone, Copy)]
enum PendingKind {
    Accept,
    Time,
    Receive,
    Send(usize),
}
struct Send {
    plaintext: usize,
    final_response: bool,
    alert_queued: bool,
}

/// Field order cancels the exact pending operation before destroying local TLS
/// state or releasing the socket. No owning scalar capability is reconstructed.
pub struct Connection<'a> {
    pending: Option<PendingCall<'static>>,
    session: Option<Session>,
    socket: Option<OwnedSocket<'a>>,
    factory: &'a Factory,
    pending_kind: Option<PendingKind>,
    started_ms: Option<u64>,
    send: Option<Send>,
    cq: u32,
    retry: Option<Deadline>,
    audit_id: u64,
}
impl<'a> Connection<'a> {
    pub fn accept_blocking(&mut self, ctx: &Context) -> Result<(), Interrupted> {
        let admission = Deadline::after(crate::deployment_listener::SETUP_MS);
        loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                return Err(Interrupted::Shutdown(request));
            }
            drain(ctx);
            let state = if self.pending.is_some() {
                self.observe()
            } else {
                self.arm(PendingKind::Accept, &admission)
            }
            .map_err(Interrupted::Transport)?;
            if self.started_ms.is_some() {
                return Ok(());
            }
            match state {
                Advance::Progress => continue,
                _ => {
                    crate::reactor::wait(
                        if self.pending.is_some() {
                            None
                        } else {
                            Some(
                                admission
                                    .remaining_ms()
                                    .min(self.retry.map_or(u64::MAX, |retry| retry.remaining_ms())),
                            )
                        },
                        0,
                    );
                }
            }
        }
    }

    /// Sequential status service adapter. The deployment reactor uses the
    /// nonblocking transitions directly, preserving its multi-client fairness.
    pub fn receive_blocking(
        &mut self,
        ctx: &Context,
        deadline: &Deadline,
    ) -> Result<Application, Interrupted> {
        self.begin_receive(0).map_err(Interrupted::Transport)?;
        loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                return Err(Interrupted::Shutdown(request));
            }
            drain(ctx);
            match self.poll_receive(deadline).map_err(Interrupted::Transport)? {
                Advance::Ready(bytes) => return Ok(bytes),
                Advance::Progress => continue,
                Advance::Waiting => {
                    crate::reactor::wait(Some(deadline.remaining_ms().min(self.remaining_ms())), 0);
                }
            }
        }
    }

    pub fn send_blocking(
        &mut self,
        ctx: &Context,
        bytes: &[u8],
        deadline: &Deadline,
    ) -> Result<(), Interrupted> {
        let mut offset = 0;
        while offset < bytes.len() {
            let end = bytes.len().min(offset + self.plaintext_capacity());
            self.begin_send(&bytes[offset..end], end == bytes.len(), 0)
                .map_err(Interrupted::Transport)?;
            loop {
                if let Some(request) = ctx.lifecycle().shutdown_requested() {
                    return Err(Interrupted::Shutdown(request));
                }
                drain(ctx);
                match self.poll_send(deadline).map_err(Interrupted::Transport)? {
                    Advance::Ready(count) => {
                        offset += count;
                        break;
                    }
                    Advance::Progress => continue,
                    Advance::Waiting => {
                        crate::reactor::wait(
                            Some(deadline.remaining_ms().min(self.remaining_ms())),
                            0,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub fn unaccepted(socket: OwnedSocket<'a>, factory: &'a Factory) -> Self {
        Self {
            pending: None,
            session: None,
            socket: Some(socket),
            factory,
            pending_kind: None,
            started_ms: None,
            send: None,
            cq: 0,
            retry: None,
            audit_id: 0,
        }
    }

    fn socket(&self) -> &OwnedSocket<'a> {
        self.socket.as_ref().expect("management socket owner")
    }

    pub fn listen_call(&self, port: u16) -> Result<PendingCall<'static>, SocketError> {
        self.socket().listen_call(port)
    }

    pub fn accept_call(&self) -> Result<PendingCall<'static>, IpcError> {
        self.socket().accept_call()
    }

    pub fn accepted(&mut self) -> Result<(), Error> {
        if self.started_ms.is_some() {
            return Err(Error::State);
        }
        self.started_ms = Some(milliseconds()?);
        self.audit_id = self
            .factory
            .next_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| next.checked_add(1))
            .map_err(|_| Error::State)?;
        Ok(())
    }

    pub fn begin_receive(&mut self, cq: u32) -> Result<(), Error> {
        if self.send.is_some() || self.started_ms.is_none() {
            return Err(Error::State);
        }
        self.cq = cq;
        Ok(())
    }

    pub fn identity(&self) -> Option<Identity> {
        self.session.as_ref().and_then(Session::identity)
    }

    pub fn remaining_ms(&self) -> u64 {
        if self.final_sent() {
            return u64::MAX;
        }
        let now = match milliseconds() {
            Ok(now) => now,
            Err(_) => return 0,
        };
        let remaining = self.session.as_ref().map_or_else(
            || {
                self.started_ms.map_or(u64::MAX, |start| {
                    self.factory
                        .prepared
                        .limits()
                        .handshake_ms
                        .saturating_sub(now.saturating_sub(start))
                })
            },
            |session| session.remaining_ms(now),
        );
        self.retry.map_or(remaining, |retry| remaining.min(retry.remaining_ms()))
    }

    pub fn plaintext_capacity(&self) -> usize {
        self.factory.prepared.limits().max_plaintext_bytes
    }

    pub fn into_socket(mut self) -> OwnedSocket<'a> {
        // Default pending cleanup confirms cancellation or aborts while the
        // complete containing owner remains live. A timeout grants no replay.
        drop(self.pending.take());
        drop(self.session.take());
        self.socket.take().unwrap()
    }

    pub fn audit(&self, identity: Identity, permission: Permission, allowed: bool) {
        struct Digest([u8; 32]);
        impl fmt::Display for Digest {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(out, "{byte:02x}")?;
                }
                Ok(())
            }
        }
        catten_rt::logln!(
            "[management] connection={} certificate={} role={:?} permission={:?} allowed={}",
            self.audit_id,
            Digest(identity.certificate_sha256()),
            identity.role(),
            permission,
            allowed
        );
    }

    fn arm(&mut self, kind: PendingKind, deadline: &Deadline) -> Result<Advance<()>, Error> {
        if deadline.expired() || self.remaining_ms() == 0 {
            return Err(Error::Deadline);
        }
        if self.retry.is_some_and(|retry| !retry.expired()) {
            return Ok(Advance::Waiting);
        }
        self.retry = None;
        let call = match kind {
            PendingKind::Accept => self.socket().accept_call().map_err(SocketError::Ipc),
            PendingKind::Time => {
                self.factory.time.call(crate::time::OP_UNIX_SECONDS, 0).map_err(SocketError::Ipc)
            }
            PendingKind::Receive => self.socket().receive_call(),
            PendingKind::Send(length) => self
                .socket()
                .send_call(&self.session.as_ref().unwrap().outgoing()[..length], deadline),
        };
        match call {
            Ok(call) => {
                self.pending = Some(call);
                self.pending_kind = Some(kind);
                if self.pending.as_mut().unwrap().bind_cq(self.cq).is_err() {
                    catten_rt::domain_abort();
                }
                Ok(Advance::Progress)
            }
            Err(SocketError::Ipc(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL))) => {
                // Only rejected queue admission lacks a readiness source.
                self.retry = Some(Deadline::after(crate::socket_close::ADMISSION_BACKOFF_MS));
                Ok(Advance::Waiting)
            }
            Err(error) => Err(Error::Socket(error)),
        }
    }

    fn observe(&mut self) -> Result<Advance<()>, Error> {
        let Some(reply) = self.pending.as_mut().unwrap().poll().map_err(Error::Observation)? else {
            return Ok(Advance::Waiting);
        };
        drop(self.pending.take());
        let now = milliseconds()?;
        match self.pending_kind.take().unwrap() {
            PendingKind::Accept => {
                if reply.memory.is_some() || reply.connection.is_some() || reply.result != 0 {
                    return Err(Error::Reply);
                }
                self.accepted()?;
            }
            PendingKind::Time => {
                if reply.memory.is_some() || reply.connection.is_some() || reply.result <= 0 {
                    return Err(Error::Clock);
                }
                self.factory.clock.install(reply.result as u64)?;
                let config = self.factory.prepared.activate().map_err(|_| Error::Clock)?;
                self.session = Some(
                    Session::new(config, self.started_ms.ok_or(Error::State)?)
                        .map_err(Error::Tls)?,
                );
            }
            PendingKind::Receive => {
                let chunk = OwnedSocket::decode_receive(reply)
                    .map_err(Error::Socket)?
                    .ok_or(Error::Reply)?;
                let (memory, length) = chunk.into_parts();
                let mapping = memory.map_read_only().map_err(|_| Error::Reply)?;
                let bytes = mapping.as_slice().get(..length).ok_or(Error::Reply)?;
                self.session.as_mut().unwrap().receive_tls(bytes, now).map_err(Error::Tls)?;
            }
            PendingKind::Send(submitted) => {
                if reply.memory.is_some()
                    || reply.connection.is_some()
                    || reply.result <= 0
                    || reply.result as u64 > submitted as u64
                {
                    return Err(Error::Reply);
                }
                self.session
                    .as_mut()
                    .unwrap()
                    .transmitted(reply.result as usize, now)
                    .map_err(Error::Tls)?;
            }
        }
        Ok(Advance::Progress)
    }

    pub fn poll_receive(&mut self, deadline: &Deadline) -> Result<Advance<Application>, Error> {
        if deadline.expired() || self.remaining_ms() == 0 {
            return Err(Error::Deadline);
        }
        if self.pending.is_some() {
            return self.observe().map(|state| match state {
                Advance::Waiting => Advance::Waiting,
                _ => Advance::Progress,
            });
        }
        if self.session.is_none() {
            return self.arm(PendingKind::Time, deadline).map(|state| match state {
                Advance::Waiting => Advance::Waiting,
                _ => Advance::Progress,
            });
        }
        let event = self.session.as_mut().unwrap().step(milliseconds()?).map_err(|error| {
            if error.is_client_policy_rejection() {
                catten_rt::logln!(
                    "[management] connection={} client policy rejected",
                    self.audit_id
                );
            }
            Error::Tls(error)
        })?;
        match event {
            charlotte_management_tls::Event::Transmit => {
                let length = self
                    .session
                    .as_ref()
                    .unwrap()
                    .outgoing()
                    .len()
                    .min(crate::socket_send::MAX_PAYLOAD);
                self.arm(PendingKind::Send(length), deadline).map(|state| match state {
                    Advance::Waiting => Advance::Waiting,
                    _ => Advance::Progress,
                })
            }
            charlotte_management_tls::Event::NeedInput => {
                self.arm(PendingKind::Receive, deadline).map(|state| match state {
                    Advance::Waiting => Advance::Waiting,
                    _ => Advance::Progress,
                })
            }
            charlotte_management_tls::Event::Progress => Ok(Advance::Progress),
            charlotte_management_tls::Event::Application {
                identity,
                bytes,
            } => Ok(Advance::Ready((identity, bytes))),
            charlotte_management_tls::Event::Closed => Err(Error::State),
        }
    }

    pub fn begin_send(&mut self, bytes: &[u8], final_response: bool, cq: u32) -> Result<(), Error> {
        if self.send.is_some() {
            return Err(Error::State);
        }
        if self.pending.is_some() {
            if self.identity().is_none() || !matches!(self.pending_kind, Some(PendingKind::Receive))
            {
                return Err(Error::State);
            }
            // Ending an authenticated input phase (for example its original
            // body deadline) cancels the accepted receive before sending the
            // rejection. Drop confirms cancellation or aborts with this complete
            // owner live. It cannot cancel/replay a time, accept or output RPC.
            drop(self.pending.take());
            self.pending_kind = None;
        }
        self.cq = cq;
        self.session
            .as_mut()
            .ok_or(Error::State)?
            .send_application(bytes, milliseconds()?)
            .map_err(Error::Tls)?;
        self.send = Some(Send {
            plaintext: bytes.len(),
            final_response,
            alert_queued: false,
        });
        Ok(())
    }

    pub fn poll_send(&mut self, deadline: &Deadline) -> Result<Advance<usize>, Error> {
        if !deadline.expired() && self.final_sent() {
            return Ok(Advance::Ready(self.send.take().unwrap().plaintext));
        }
        if deadline.expired() || self.remaining_ms() == 0 {
            return Err(Error::Deadline);
        }
        if self.pending.is_some() {
            return self.observe().map(|state| match state {
                Advance::Waiting => Advance::Waiting,
                _ => Advance::Progress,
            });
        }
        let remaining = self.session.as_ref().ok_or(Error::State)?.outgoing().len();
        if remaining != 0 {
            return self
                .arm(PendingKind::Send(remaining.min(crate::socket_send::MAX_PAYLOAD)), deadline)
                .map(|state| match state {
                    Advance::Waiting => Advance::Waiting,
                    _ => Advance::Progress,
                });
        }
        let send = self.send.as_mut().ok_or(Error::State)?;
        if send.final_response && !send.alert_queued {
            self.session.as_mut().unwrap().close_notify(milliseconds()?).map_err(Error::Tls)?;
            send.alert_queued = true;
            return Ok(Advance::Progress);
        }
        Ok(Advance::Ready(self.send.take().unwrap().plaintext))
    }

    fn final_sent(&self) -> bool {
        self.pending.is_none()
            && self.send.as_ref().is_some_and(|send| send.final_response && send.alert_queued)
            && self.session.as_ref().is_some_and(|session| session.outgoing().is_empty())
    }
}

pub fn drain(ctx: &Context) {
    let layout = ctx.completion_queue_layout();
    for _ in 0..layout.entries {
        if unsafe { catten_syscall::cq_read(layout.base, layout.entries) }.is_none() {
            break;
        }
    }
}

/// Missing/rejected credentials disable network admission while retaining a
/// cooperative lifecycle endpoint. There is no retry or plaintext listener.
pub fn disabled(ctx: &Context) -> ShutdownRequest {
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return request;
        }
        drain(ctx);
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return request;
        }
        crate::reactor::wait(None, 0);
    }
}

/// Exact read-only HTTP/1.1 framing for the keyhole. No malformed-method or
/// clamped-prefix dashboard fallback, request body or transfer encoding.
pub fn keyhole_path(bytes: &[u8]) -> Option<&[u8]> {
    let text = core::str::from_utf8(bytes).ok()?.strip_suffix("\r\n\r\n")?;
    let mut lines = text.split("\r\n");
    let mut words = lines.next()?.split(' ');
    if words.next()? != "GET" {
        return None;
    }
    let path = words.next()?;
    if !path.starts_with('/')
        || !path.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        || words.next()? != "HTTP/1.1"
        || words.next().is_some()
    {
        return None;
    }
    let mut host = false;
    let mut length = false;
    for line in lines {
        let (name, value) = line.split_once(':')?;
        if name.is_empty()
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| byte.is_ascii_control() && byte != b'\t')
        {
            return None;
        }
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            if host || value.is_empty() {
                return None;
            }
            host = true;
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return None;
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length || value.is_empty() || !value.bytes().all(|byte| byte == b'0') {
                return None;
            }
            length = true;
        }
    }
    host.then_some(path.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_projection_rejects_rollback_zero_frequency_and_overflow() {
        let anchor = Anchor {
            seconds: 100,
            ticks: 200,
            frequency: 10,
        };
        assert_eq!(project(anchor, 219), Some(101));
        assert_eq!(project(anchor, 199), None);
        assert_eq!(
            project(
                Anchor {
                    frequency: 0,
                    ..anchor
                },
                200
            ),
            None
        );
        assert_eq!(
            project(
                Anchor {
                    seconds: u64::MAX,
                    ..anchor
                },
                210
            ),
            None
        );
    }

    #[test]
    fn keyhole_requires_complete_bodyless_get_and_single_host() {
        assert_eq!(
            keyhole_path(b"GET /metrics HTTP/1.1\r\nHost: test\r\n\r\n"),
            Some(&b"/metrics"[..])
        );
        assert_eq!(
            keyhole_path(b"GET / HTTP/1.1\r\nhost: test\r\nContent-Length: 0\r\n\r\n"),
            Some(&b"/"[..])
        );
        for bytes in [
            &b"GET / HTTP/1.1\r\n\r\n"[..],
            b"POST / HTTP/1.1\r\nHost: test\r\n\r\n",
            b"GET / HTTP/1.0\r\nHost: test\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: test\r\n",
            b"GET / HTTP/1.1\r\nHost: test\r\nHost: other\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: test\r\nContent-Length: 1\r\n\r\nx",
            b"GET / HTTP/1.1\r\nHost: test\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: test\r\n\r\nGET / HTTP/1.1\r\nHost: test\r\n\r\n",
        ] {
            assert_eq!(keyhole_path(bytes), None, "{bytes:?}");
        }
    }
}
