use alloc::{
    sync::Arc,
    vec::Vec,
};

use rustls::{
    server::UnbufferedServerConnection,
    unbuffered::ConnectionState,
};
use zeroize::{
    Zeroize,
    Zeroizing,
};

use crate::{
    Config,
    Identity,
    fingerprint,
};

#[derive(Clone, Debug)]
pub enum Error {
    Tls(rustls::Error),
    Capacity,
    HandshakeDeadline,
    SessionDeadline,
    MonotonicClock,
    CredentialClockUnavailable,
    CredentialClockRollback,
    Unauthenticated,
    InvalidReceipt,
    NotReady,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientRejection {
    Policy,
    Expired,
    NotYetValid,
}
impl ClientRejection {
    pub fn audit_message(self) -> &'static str {
        match self {
            Self::Policy => "client policy rejected",
            Self::Expired => "client certificate expired",
            Self::NotYetValid => "client certificate not yet valid",
        }
    }
}
impl Error {
    /// A bounded audit category, without exposing parser/crypto contents or
    /// minting an authenticated identity for a rejected peer. This includes
    /// enrollment and the verifier's credential/time/capacity policy gates.
    pub fn client_rejection(&self) -> Option<ClientRejection> {
        use rustls::CertificateError;
        match self {
            Self::Tls(rustls::Error::InvalidCertificate(error)) => match error {
                CertificateError::ApplicationVerificationFailure => Some(ClientRejection::Policy),
                CertificateError::Expired
                | CertificateError::ExpiredContext {
                    ..
                } => Some(ClientRejection::Expired),
                CertificateError::NotValidYet
                | CertificateError::NotValidYetContext {
                    ..
                } => Some(ClientRejection::NotYetValid),
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum Event {
    /// Send `outgoing()` and retain it through confirmed prefix receipts.
    Transmit,
    /// Register socket receive readiness; do not poll this engine on a timer.
    NeedInput,
    /// Bounded local work made progress; give other reactor sources a turn.
    Progress,
    /// This owner is produced only after chain and possession verification.
    Application {
        identity: Identity,
        bytes: Zeroizing<Vec<u8>>,
    },
    Closed,
}

/// A TLS session owns its record storage and original absolute deadlines. It
/// owns no remote resource: the containing service transaction must retain its
/// socket and pending CQ operations alongside this owner through cancellation.
pub struct Session {
    connection: UnbufferedServerConnection,
    config: Config,
    incoming: Zeroizing<Vec<u8>>,
    outgoing: Zeroizing<Vec<u8>>,
    sent: usize,
    identity: Option<Identity>,
    handshake_deadline: u64,
    session_deadline: u64,
    last_now: u64,
    failure: Option<Error>,
    closed: bool,
    closing: bool,
    last_utc: Option<u64>,
    verified_utc: Option<u64>,
}

impl Session {
    /// `now_ms` comes from the kernel monotonic clock, independently of the
    /// certificate-validity UTC provider. Every public transition checks it.
    pub fn new(config: Config, now_ms: u64) -> Result<Self, Error> {
        let handshake_deadline =
            now_ms.checked_add(config.limits.handshake_ms).ok_or(Error::MonotonicClock)?;
        let session_deadline =
            now_ms.checked_add(config.limits.session_ms).ok_or(Error::MonotonicClock)?;
        Ok(Self {
            connection: UnbufferedServerConnection::new(Arc::clone(&config.tls))
                .map_err(Error::Tls)?,
            config,
            incoming: Zeroizing::new(Vec::new()),
            outgoing: Zeroizing::new(Vec::new()),
            sent: 0,
            identity: None,
            handshake_deadline,
            session_deadline,
            last_now: now_ms,
            failure: None,
            closed: false,
            closing: false,
            last_utc: None,
            verified_utc: None,
        })
    }

    fn check(&mut self, now: u64) -> Result<(), Error> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let error = if now < self.last_now {
            Some(Error::MonotonicClock)
        } else if now >= self.session_deadline {
            Some(Error::SessionDeadline)
        } else if self.identity.is_none() && now >= self.handshake_deadline {
            Some(Error::HandshakeDeadline)
        } else {
            None
        };
        self.last_now = now;
        if let Some(error) = error {
            self.fail(error)
        } else {
            self.check_credentials().or_else(|error| self.fail(error))
        }
    }

    fn check_credentials(&mut self) -> Result<(), Error> {
        let now = self.config.current_time().ok_or(Error::CredentialClockUnavailable)?;
        let seconds = now.as_secs();
        if self.last_utc.is_some_and(|previous| seconds < previous) {
            return Err(Error::CredentialClockRollback);
        }
        self.last_utc = Some(seconds);
        if let Some(identity) = self.identity.as_mut()
            && self.verified_utc != Some(seconds)
        {
            let chain = self.connection.peer_certificates().ok_or(Error::Unauthenticated)?;
            self.config.verify_peer(chain, now).map_err(Error::Tls)?;
            self.verified_utc = Some(seconds);
            identity.verified_at(seconds);
        }
        Ok(())
    }

    /// Check before admitting application/transport work; idle owners still
    /// wait on real readiness. Revalidate at most once per observed UTC second,
    /// not once per TLS fragment. Loss/rollback and verification errors latch.
    pub fn check_authority(&mut self, now_ms: u64) -> Result<(), Error> {
        self.check(now_ms)
    }

    fn fail<T>(&mut self, error: Error) -> Result<T, Error> {
        self.failure = Some(error.clone());
        self.incoming.zeroize();
        self.outgoing.zeroize();
        self.sent = 0;
        self.identity = None;
        Err(error)
    }

    /// A CQ producer's receive result supplies exactly these bytes. An empty
    /// transport read is EOF, never a reason to reissue a completed receive.
    pub fn receive_tls(&mut self, bytes: &[u8], now_ms: u64) -> Result<(), Error> {
        self.check(now_ms)?;
        if self.closed || self.closing {
            return self.fail(Error::Closed);
        }
        if bytes.is_empty() {
            return self.fail(Error::Closed);
        }
        if bytes.len() > self.config.limits.max_input_bytes.saturating_sub(self.incoming.len())
            || self.incoming.try_reserve(bytes.len()).is_err()
        {
            return self.fail(Error::Capacity);
        }
        self.incoming.extend_from_slice(bytes);
        Ok(())
    }

    /// No caller may extract keys or reconfigure the verifier of a live session.
    pub fn identity(&mut self, now_ms: u64) -> Result<Option<Identity>, Error> {
        self.check(now_ms)?;
        if self.closed || self.closing || self.failure.is_some() {
            Ok(None)
        } else {
            Ok(self.identity)
        }
    }

    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        if self.closed || self.failure.is_some() {
            return 0;
        }
        let end = if self.identity.is_some() {
            self.session_deadline
        } else {
            self.handshake_deadline.min(self.session_deadline)
        };
        end.saturating_sub(now_ms)
    }

    pub fn outgoing(&self) -> &[u8] {
        &self.outgoing[self.sent..]
    }

    /// Acknowledges only a confirmed, positive prefix. Uncertain/failed sends
    /// must terminate the containing transport owner; never replay their bytes.
    pub fn transmitted(&mut self, count: usize, now_ms: u64) -> Result<(), Error> {
        self.check(now_ms)?;
        if count == 0 || count > self.outgoing().len() {
            return self.fail(Error::InvalidReceipt);
        }
        self.sent += count;
        if self.sent == self.outgoing.len() {
            self.outgoing.zeroize();
            self.sent = 0;
            if self.closing {
                self.closed = true;
            }
        }
        Ok(())
    }

    fn authenticate(&mut self) -> Result<(), Error> {
        if self.identity.is_none() && !self.connection.is_handshaking() {
            let leaf = self
                .connection
                .peer_certificates()
                .and_then(|chain| chain.first())
                .ok_or(Error::Unauthenticated)?;
            let digest = fingerprint(leaf);
            self.identity = self
                .config
                .operators
                .iter()
                .find(|entry| entry.certificate_sha256 == digest)
                .copied()
                .map(Identity::authenticated);
            if self.identity.is_none() {
                return Err(Error::Unauthenticated);
            }
            // Finished proves possession; validate again at identity admission
            // because time may have advanced since the Certificate message.
            self.check_credentials()?;
        }
        Ok(())
    }

    pub fn step(&mut self, now_ms: u64) -> Result<Event, Error> {
        self.check(now_ms)?;
        if self.closed {
            return Ok(Event::Closed);
        }
        if !self.outgoing().is_empty() {
            return Ok(Event::Transmit);
        }
        // Output storage is admitted before advancing Rustls, so a rejected
        // allocation cannot consume an encoded record and then request replay.
        if self.outgoing.try_reserve(self.config.limits.max_output_bytes).is_err() {
            return self.fail(Error::Capacity);
        }
        for _ in 0..self.config.limits.work_per_step {
            let status = self.connection.process_tls_records(&mut self.incoming);
            let mut discard = status.discard;
            let mut plaintext = None;
            let result = match status.state {
                Err(error) => Err(Error::Tls(error)),
                Ok(ConnectionState::EncodeTlsData(mut encode)) => {
                    self.outgoing.resize(self.config.limits.max_output_bytes, 0);
                    match encode.encode(&mut self.outgoing) {
                        Ok(len) => {
                            self.outgoing.truncate(len);
                            Ok(Some(Event::Transmit))
                        }
                        Err(_) => Err(Error::Capacity),
                    }
                }
                Ok(ConnectionState::TransmitTlsData(transmit)) => {
                    // Each earlier encoding was sent exactly once and fully
                    // confirmed before step could reach this transition.
                    transmit.done();
                    Ok(None)
                }
                Ok(ConnectionState::ReadTraffic(mut traffic)) => match traffic.next_record() {
                    Some(Ok(record)) => {
                        discard += record.discard;
                        if record.payload.len() > self.config.limits.max_plaintext_bytes {
                            Err(Error::Capacity)
                        } else {
                            let mut bytes = Zeroizing::new(Vec::new());
                            match bytes.try_reserve_exact(record.payload.len()) {
                                Err(_) => Err(Error::Capacity),
                                Ok(()) => {
                                    bytes.extend_from_slice(record.payload);
                                    plaintext = Some(bytes);
                                    Ok(None)
                                }
                            }
                        }
                    }
                    Some(Err(error)) => Err(Error::Tls(error)),
                    None => Ok(None),
                },
                Ok(ConnectionState::BlockedHandshake | ConnectionState::WriteTraffic(_)) => {
                    Ok(Some(Event::NeedInput))
                }
                Ok(ConnectionState::PeerClosed | ConnectionState::Closed) => {
                    self.closed = true;
                    Ok(Some(Event::Closed))
                }
                // Early data is disabled in the private configuration.
                Ok(_) => Err(Error::Unauthenticated),
            };
            if discard > self.incoming.len() {
                return self.fail(Error::Capacity);
            }
            if discard != 0 {
                self.incoming[..discard].zeroize();
                let length = self.incoming.len();
                self.incoming.copy_within(discard.., 0);
                self.incoming[length - discard..].zeroize();
                self.incoming.truncate(length - discard);
            }
            let event = match result {
                Ok(event) => event,
                Err(error) => return self.fail(error),
            };
            if let Err(error) = self.authenticate() {
                return self.fail(error);
            }
            if let Some(bytes) = plaintext {
                let Some(identity) = self.identity else {
                    return self.fail(Error::Unauthenticated);
                };
                return Ok(Event::Application {
                    identity,
                    bytes,
                });
            }
            if let Some(event) = event {
                return Ok(event);
            }
        }
        Ok(Event::Progress)
    }

    /// Encrypt one application record only after authentication and after all
    /// earlier wire bytes have a confirmed receipt. The HTTP adapter must check
    /// the authenticated role before invoking a read or mutation handler.
    pub fn send_application(&mut self, bytes: &[u8], now_ms: u64) -> Result<(), Error> {
        self.write(bytes, false, now_ms)
    }

    pub fn close_notify(&mut self, now_ms: u64) -> Result<(), Error> {
        self.write(&[], true, now_ms)
    }

    fn write(&mut self, bytes: &[u8], close: bool, now_ms: u64) -> Result<(), Error> {
        self.check(now_ms)?;
        if self.closed || self.closing {
            return self.fail(Error::Closed);
        }
        if self.identity.is_none() || !self.outgoing().is_empty() {
            return Err(Error::NotReady);
        }
        if (!close && bytes.is_empty()) || bytes.len() > self.config.limits.max_plaintext_bytes {
            return self.fail(Error::Capacity);
        }
        if self.outgoing.try_reserve(self.config.limits.max_output_bytes).is_err() {
            return self.fail(Error::Capacity);
        }
        self.outgoing.resize(self.config.limits.max_output_bytes, 0);
        let status = self.connection.process_tls_records(&mut []);
        let result = match status.state {
            Ok(ConnectionState::WriteTraffic(mut traffic)) if status.discard == 0 => if close {
                traffic.queue_close_notify(&mut self.outgoing)
            } else {
                traffic.encrypt(bytes, &mut self.outgoing)
            }
            .map_err(|_| Error::Capacity),
            Err(error) => Err(Error::Tls(error)),
            _ => Err(Error::NotReady),
        };
        match result {
            Ok(len) => {
                self.outgoing.truncate(len);
                self.closing = close;
                Ok(())
            }
            Err(error) => self.fail(error),
        }
    }
}
