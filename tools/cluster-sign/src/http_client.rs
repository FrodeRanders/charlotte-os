//! One monotonic network budget and one owned socket per ingress request.
use std::{
    io::{
        self,
        Read,
        Write,
    },
    net::{
        SocketAddr,
        TcpStream,
    },
    time::{
        Duration,
        Instant,
    },
};

use crate::Result;

/// Numeric endpoints keep name resolution outside the bounded ingress operation.
pub(super) fn endpoint(value: &str) -> Result<SocketAddr> {
    value.parse::<SocketAddr>().ok().filter(|address| address.port() != 0).ok_or_else(|| {
        "ingress endpoint must be numeric IPv4:port or [IPv6]:port with a nonzero port".to_owned()
    })
}

#[derive(Clone, Copy)]
pub(super) struct Deadline {
    end: Instant,
}
impl Deadline {
    pub(super) fn after_seconds(seconds: u64) -> Result<Self> {
        if seconds == 0 {
            return Err("timeout must be greater than zero".to_owned());
        }
        Instant::now()
            .checked_add(Duration::from_secs(seconds))
            .map(|end| Self {
                end,
            })
            .ok_or_else(|| "timeout exceeds the monotonic clock range".to_owned())
    }

    fn remaining_at(self, now: Instant) -> io::Result<Duration> {
        self.end
            .checked_duration_since(now)
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "operation deadline expired"))
    }

    pub(super) fn remaining(self) -> io::Result<Duration> {
        self.remaining_at(Instant::now())
    }

    pub(super) fn expired(self) -> bool {
        self.remaining().is_err()
    }

    pub(super) fn capped(self, seconds: u64) -> Self {
        let end = Instant::now().checked_add(Duration::from_secs(seconds)).unwrap_or(self.end);
        Self {
            end: self.end.min(end),
        }
    }

    /// The HTTP API has no readiness subscription. Keep its existing one-second
    /// observation interval bounded by this operation's remaining budget.
    pub(super) fn pause(self) -> io::Result<()> {
        std::thread::sleep(self.remaining()?.min(Duration::from_secs(1)));
        self.remaining().map(|_| ())
    }
}

fn write_with_budget(
    mut bytes: &[u8],
    mut remaining: impl FnMut() -> io::Result<Duration>,
    mut write: impl FnMut(&[u8], Duration) -> io::Result<usize>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let budget = remaining()?;
        match write(bytes, budget) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) if count <= bytes.len() => bytes = &bytes[count..],
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    remaining().map(|_| ())
}

/// Connect and submit once. Every read/write recomputes the remaining budget;
/// timeout after any submitted prefix is uncertain and never causes replay.
pub(super) fn exchange(
    endpoint: SocketAddr,
    header: &[u8],
    body: &[u8],
    deadline: Deadline,
) -> Result<String> {
    let mut stream = TcpStream::connect_timeout(
        &endpoint,
        deadline.remaining().map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    for bytes in [header, body] {
        write_with_budget(
            bytes,
            || deadline.remaining(),
            |bytes, remaining| {
                stream.set_write_timeout(Some(remaining))?;
                stream.write(bytes)
            },
        )
        .map_err(|error| {
            format!("send ingress request: {error}; submitted work may be committed")
        })?;
    }
    let response = read_framed_http_response(|bytes| {
        stream.set_read_timeout(Some(deadline.remaining()?))?;
        stream.read(bytes)
    })
    .map_err(|error| format!("read ingress response: {error}; submitted work may be committed"))?;
    deadline.remaining().map_err(|error| format!("{error}; submitted work may be committed"))?;
    Ok(response)
}

pub(super) const HTTP_RESPONSE_LIMIT: usize = 8192;

/// Observe one complete bounded response, without requiring transport EOF.
pub(super) fn read_framed_http_response(
    mut read: impl FnMut(&mut [u8]) -> std::io::Result<usize>,
) -> Result<String> {
    let mut bytes = Vec::new();
    let mut total = None;
    loop {
        if let Some(length) = total.filter(|length| bytes.len() >= *length) {
            bytes.truncate(length);
            return String::from_utf8(bytes).map_err(|_| "response is not UTF-8".to_owned());
        }
        let remaining = HTTP_RESPONSE_LIMIT - bytes.len();
        if remaining == 0 {
            return Err("response exceeds 8192 bytes".to_owned());
        }
        let mut chunk = [0u8; 1024];
        let count = match read(&mut chunk[..remaining.min(1024)]) {
            Ok(0) => return Err("response ended before its declared body completed".to_owned()),
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        };
        bytes.extend_from_slice(&chunk[..count]);
        let header_end = if total.is_none() {
            bytes.windows(4).position(|window| window == b"\r\n\r\n")
        } else {
            None
        };
        if let Some(offset) = header_end {
            let end = offset + 4;
            let header = core::str::from_utf8(&bytes[..offset])
                .map_err(|_| "response header is not UTF-8".to_owned())?;
            let mut body_len = None;
            for line in header.split("\r\n").skip(1) {
                let (name, value) =
                    line.split_once(':').ok_or_else(|| "malformed response header".to_owned())?;
                if name.eq_ignore_ascii_case("transfer-encoding") {
                    return Err("unsupported response transfer encoding".to_owned());
                }
                if name.eq_ignore_ascii_case("content-length") {
                    let value = value.trim();
                    if body_len.is_some()
                        || value.is_empty()
                        || !value.bytes().all(|byte| byte.is_ascii_digit())
                    {
                        return Err("ambiguous response content length".to_owned());
                    }
                    body_len = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| "response length overflow".to_owned())?,
                    );
                }
            }
            let length = end
                .checked_add(body_len.ok_or_else(|| "response lacks content length".to_owned())?)
                .filter(|length| *length <= HTTP_RESPONSE_LIMIT)
                .ok_or_else(|| "response exceeds 8192 bytes".to_owned())?;
            total = Some(length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_endpoint_grammar_rejects_resolution_and_header_material() {
        for valid in ["127.0.0.1:8081", "[::1]:8081", "[2001:db8::1]:443"] {
            assert!(endpoint(valid).is_ok());
        }
        for invalid in [
            "localhost:8081",
            "example.org:443",
            "127.0.0.1",
            "127.0.0.1:0",
            "::1:8081",
            "127.0.0.1:8081\r\nOther: x",
            "http://127.0.0.1:8081",
        ] {
            assert!(endpoint(invalid).is_err(), "{invalid:?}");
        }
    }
    #[test]
    fn zero_overflow_expiry_and_phase_caps_are_checked() {
        assert!(Deadline::after_seconds(0).is_err());
        assert!(Deadline::after_seconds(u64::MAX).is_err());
        let start = Instant::now();
        let deadline = Deadline {
            end: start + Duration::from_secs(10),
        };
        assert_eq!(
            deadline.remaining_at(start + Duration::from_secs(4)).unwrap(),
            Duration::from_secs(6)
        );
        assert_eq!(
            deadline.remaining_at(deadline.end).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            deadline.remaining_at(deadline.end + Duration::from_secs(1)).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(deadline.capped(30).end, deadline.end);
        assert!(deadline.capped(2).end <= deadline.end);
    }
    #[test]
    fn expired_exchange_opens_no_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let deadline = Deadline {
            end: Instant::now(),
        };
        assert!(exchange(listener.local_addr().unwrap(), b"request", b"body", deadline).is_err());
        assert_eq!(listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn partial_and_interrupted_writes_use_remaining_budget_without_replay() {
        let mut budgets = [10, 8, 5, 3].into_iter();
        let mut attempts = Vec::new();
        write_with_budget(
            b"abc",
            || Ok(Duration::from_secs(budgets.next().unwrap())),
            |bytes, budget| {
                attempts.push((bytes.to_vec(), budget.as_secs()));
                if attempts.len() == 1 {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                Ok(if bytes.len() == 3 {
                    2
                } else {
                    1
                })
            },
        )
        .unwrap();
        assert_eq!(attempts, [(b"abc".to_vec(), 10), (b"abc".to_vec(), 8), (b"c".to_vec(), 5)]);
        assert_eq!(budgets.next(), None);
    }
    #[test]
    fn expired_write_budget_stops_before_another_submission() {
        let mut checks = 0;
        let mut calls = 0;
        let error = write_with_budget(
            b"abc",
            || {
                checks += 1;
                if checks > 1 {
                    Err(io::ErrorKind::TimedOut.into())
                } else {
                    Ok(Duration::from_secs(1))
                }
            },
            |_, _| {
                calls += 1;
                Ok(1)
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(calls, 1);
    }
    #[test]
    fn terminal_write_errors_do_not_resubmit_the_prefix() {
        for kind in
            [io::ErrorKind::WriteZero, io::ErrorKind::ConnectionReset, io::ErrorKind::InvalidData]
        {
            let mut calls = 0;
            let error = write_with_budget(
                b"abc",
                || Ok(Duration::from_secs(1)),
                |_, _| {
                    calls += 1;
                    match kind {
                        io::ErrorKind::WriteZero => Ok(0),
                        io::ErrorKind::InvalidData => Ok(4),
                        _ => Err(kind.into()),
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(calls, 1);
        }
    }
}
