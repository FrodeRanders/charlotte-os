//! One bounded HTTP export owner; no sampler or placement state is borrowed.

use alloc::vec::Vec;

use catten_rt::{
    Context,
    owned::{
        CallResult,
        ConnectionRef,
        OwnedMemory,
        PendingCall,
    },
};
use charlotte_s3::http::{
    ChunkedDecoder,
    ResponseHeadBuffer,
};

use crate::{
    deadline::Deadline,
    otlp::{
        self,
        Acknowledgement,
    },
    socket::{
        self,
        OpeningSocket,
        OwnedSocket,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Admission,
    Deadline,
    Shutdown,
    Protocol,
    Transport,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Accepted(Acknowledgement),
    Rejected {
        status: u16,
        retry_after_ms: Option<u64>,
    },
}

struct Export<'a> {
    // Cancel an accepted pending call before closing its enclosing socket.
    pending: Option<PendingCall<'static>>,
    opening: Option<OpeningSocket<'a>>,
    socket: Option<OwnedSocket<'a>>,
    request: Vec<u8>,
    response: Vec<u8>,
    deadline: Deadline,
}

/// Consume hints without adopting results or attachments from their raw IDs.
///
/// # Safety
/// The caller must be the sole consumer of the launch-owned default CQ.
pub unsafe fn drain_hints(ctx: &Context) {
    let cq = ctx.completion_queue_layout();
    for _ in 0..cq.entries {
        // SAFETY: the caller retains the launch mapping and sole-reader role.
        if unsafe { catten_syscall::cq_read(cq.base, cq.entries) }.is_none() {
            break;
        }
    }
}

impl Export<'_> {
    fn check(&self, ctx: &Context) -> Result<(), Error> {
        if ctx.lifecycle().shutdown_requested().is_some() {
            return Err(Error::Shutdown);
        }
        if self.deadline.expired() {
            return Err(Error::Deadline);
        }
        Ok(())
    }

    fn park(&self, ctx: &Context) -> Result<(), Error> {
        self.check(ctx)?;
        // All accepted calls bind to the lifecycle-aware default CQ. Its
        // notification is only a hint; poll the retained owner again afterward.
        crate::reactor::wait(Some(self.deadline.remaining_ms()), 0);
        Ok(())
    }

    fn submit(&mut self, mut call: PendingCall<'static>) -> Result<(), Error> {
        call.bind_cq(0).map_err(|_| Error::Admission)?;
        self.pending = Some(call);
        Ok(())
    }

    fn reply(&mut self, ctx: &Context) -> Result<CallResult, Error> {
        loop {
            // SAFETY: post requires the default CQ's sole-consumer role.
            unsafe { drain_hints(ctx) };
            if let Some(reply) = self
                .pending
                .as_mut()
                .ok_or(Error::Protocol)?
                .poll()
                .map_err(|_| Error::Transport)?
            {
                drop(self.pending.take());
                return Ok(reply);
            }
            self.park(ctx)?;
        }
    }

    fn scalar(&mut self, ctx: &Context) -> Result<i64, Error> {
        let r = self.reply(ctx)?;
        if r.memory.is_some() || r.connection.is_some() {
            return Err(Error::Protocol);
        }
        Ok(r.result)
    }

    fn finish(mut self, result: Result<Outcome, Error>) -> Result<Outcome, Error> {
        // Checked local cancellation precedes remote socket teardown on every
        // success/failure/shutdown path. Unobserved creation already has its
        // own retaining-abort fallback; it cannot be replaced by a new socket.
        drop(self.pending.take());
        drop(self.opening.take());
        if let Some(socket) = self.socket.take()
            && socket.close().is_err()
        {
            catten_rt::logln!("[otlp] socket close unconfirmed; retaining exporter through abort");
            catten_rt::domain_abort();
        }
        result
    }
}

/// Export one batch through the launch-owned default CQ.
///
/// # Safety
/// The caller must be the sole default-CQ consumer for this operation.
pub unsafe fn post(
    ctx: &Context,
    tcpip: ConnectionRef<'_>,
    profile: &charlotte_launch::otlp::Profile,
    request: Vec<u8>,
    deadline: Deadline,
) -> Result<Outcome, Error> {
    if !profile.valid() || request.is_empty() || request.len() > otlp::MAX_REQUEST {
        return Err(Error::Protocol);
    }
    let mut op = Export {
        pending: None,
        opening: None,
        socket: None,
        request,
        response: Vec::new(),
        deadline,
    };
    let result = (|| {
        op.check(ctx)?;
        op.opening =
            Some(OwnedSocket::open_call(tcpip, socket::DOMAIN_TCP).map_err(|_| Error::Admission)?);
        op.opening.as_mut().unwrap().bind_cq(0).map_err(|_| Error::Admission)?;
        loop {
            // SAFETY: inherited from post's sole-consumer contract.
            unsafe { drain_hints(ctx) };
            if let Some(socket) =
                op.opening.as_mut().unwrap().poll().map_err(|_| Error::Transport)?
            {
                op.socket = Some(socket);
                drop(op.opening.take());
                break;
            }
            op.park(ctx)?;
        }
        let memory = OwnedMemory::allocate(1).map_err(|_| Error::Admission)?;
        let mut mapping = memory.map_writable().map_err(|_| Error::Admission)?;
        mapping.as_mut_slice()[..4].copy_from_slice(&profile.address);
        mapping.as_mut_slice()[4..6].copy_from_slice(&profile.port.to_le_bytes());
        let memory = mapping.unmap().map_err(|_| Error::Admission)?;
        op.check(ctx)?;
        op.submit(
            tcpip
                .call_move(socket::OP_CONNECT, op.socket.as_ref().unwrap().id(), memory)
                .map_err(|_| Error::Admission)?,
        )?;
        if op.scalar(ctx)? != 0 {
            return Err(Error::Transport);
        }
        let mut offset = 0;
        while offset < op.request.len() {
            op.check(ctx)?;
            let end = (offset + crate::socket_send::MAX_PAYLOAD).min(op.request.len());
            let call = op
                .socket
                .as_ref()
                .unwrap()
                .send_call(&op.request[offset..end], &op.deadline)
                .map_err(|_| Error::Transport)?;
            op.submit(call)?;
            let written = op.scalar(ctx)?;
            if written <= 0 || written as usize > end - offset {
                return Err(Error::Transport);
            }
            offset += written as usize;
        }
        let mut parser = ResponseHeadBuffer::new();
        let mut head: Option<charlotte_s3::http::ResponseHead> = None;
        let mut chunked = ChunkedDecoder::new();
        let outcome = loop {
            op.check(ctx)?;
            let call = op.socket.as_ref().unwrap().receive_call().map_err(|_| Error::Admission)?;
            op.submit(call)?;
            let reply = op.reply(ctx)?;
            let chunk = OwnedSocket::decode_receive(reply).map_err(|_| Error::Transport)?;
            let Some(chunk) = chunk else {
                let h = head.as_ref().ok_or(Error::Protocol)?;
                if h.content_length.is_some() || h.chunked {
                    return Err(Error::Protocol);
                }
                break Outcome::Accepted(otlp::acknowledge(&op.response).ok_or(Error::Protocol)?);
            };
            let (memory, len) = chunk.into_parts();
            let mapping = memory.map_read_only().map_err(|_| Error::Admission)?;
            let bytes = &mapping.as_slice()[..len];
            let suffix;
            let bytes = if head.is_none() {
                let Some((parsed, body)) = parser.push(bytes).map_err(|_| Error::Protocol)? else {
                    continue;
                };
                if parsed.status != 200 {
                    break Outcome::Rejected {
                        status: parsed.status,
                        retry_after_ms: parsed
                            .retry_after_seconds
                            .map(|n| n.saturating_mul(1000).min(86_400_000)),
                    };
                }
                if !parsed.content_type().is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .eq_ignore_ascii_case("application/x-protobuf")
                }) {
                    return Err(Error::Protocol);
                }
                if parsed.content_length.is_some_and(|n| n > otlp::MAX_RESPONSE as u64) {
                    return Err(Error::Protocol);
                }
                head = Some(parsed);
                suffix = body;
                suffix.as_slice()
            } else {
                bytes
            };
            let h = head.as_ref().unwrap();
            let decoded;
            let bytes = if h.chunked {
                decoded = chunked.decode(bytes).map_err(|_| Error::Protocol)?;
                decoded.as_slice()
            } else {
                bytes
            };
            if op.response.len() + bytes.len() > otlp::MAX_RESPONSE {
                return Err(Error::Protocol);
            }
            op.response.extend_from_slice(bytes);
            if let Some(n) = h.content_length
                && op.response.len() as u64 > n
            {
                return Err(Error::Protocol);
            }
            if h.content_length == Some(op.response.len() as u64)
                || (h.chunked && chunked.is_complete())
            {
                break Outcome::Accepted(otlp::acknowledge(&op.response).ok_or(Error::Protocol)?);
            }
        };
        Ok(outcome)
    })();
    op.finish(result)
}
