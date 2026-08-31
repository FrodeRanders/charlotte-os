//! Capability-oriented S3 data-plane service.
//!
//! One instance is bound at launch to a single endpoint, bucket, key prefix,
//! credential identity, and rights mask. Applications receive the service
//! capability, not the credentials. Object bodies stream as moved memory
//! pages; remote GET/PUT operation IDs are owned and have explicit close or
//! abort operations with application-side `Drop` fallbacks.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{
        String,
        ToString,
    },
    vec::Vec,
};
use core::fmt::Write;

use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    config,
    owned::{
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        ReplyToken,
    },
};
use catten_services::{
    entropy,
    ns,
    s3 as protocol,
    sleep_ms,
    socket,
    time,
    tls_client,
    try_registered_name_owned,
    wait_for_local_ready_owned,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    IpcRights,
    thread_exit,
};
use charlotte_launch::{
    s3_status as status,
    sha256::Sha256,
};
use charlotte_protocol_s3::{
    ObjectMetadata,
    ObjectRequest,
};
use charlotte_s3::{
    http::{
        ChunkedDecoder,
        Error as HttpError,
        ResponseHead,
    },
    sigv4::{
        Credentials,
        EMPTY_PAYLOAD_SHA256,
        Header,
        Request as SignRequest,
        Timestamp,
        canonical_uri,
        hex_lower,
        sign,
    },
};
use zeroize::Zeroizing;
catten_rt::entry!(main);

const MAX_OPERATIONS: usize = 8;
const OPERATION_LEASE_SECONDS: u64 = 300;
const SEND_ATTEMPTS: usize = 4_096;
const SEND_RETRY_MS: u64 = 10;
const RECEIVE_ATTEMPTS: usize = 3_000;
const RECEIVE_RETRY_MS: u64 = 10;
const MAX_HEAD_READS: usize = 16;
struct Profile {
    ip: [u8; 4],
    host: String,
    port: u16,
    tls: bool,
    ca_der: Vec<u8>,
    region: String,
    bucket: String,
    prefix: String,
    access_key: String,
    secret_key: Zeroizing<Vec<u8>>,
    namespace: Option<String>,
    rights: u64,
}

impl Profile {
    fn from_context(ctx: &Context) -> Option<Self> {
        if let Some(memory) = ctx.profile_memory() {
            let mapping = memory.map_read_only().ok()?;
            return Self::from_wire(mapping.as_slice());
        }
        Self::from_manifest(ctx)
    }

    fn from_wire(bytes: &[u8]) -> Option<Self> {
        let profile = charlotte_protocol_s3::Profile::decode(bytes)?;
        Self::validate(Self {
            ip: profile.endpoint_ipv4,
            host: core::str::from_utf8(profile.host).ok()?.into(),
            port: profile.port,
            tls: profile.tls,
            ca_der: profile.ca_certificate_der.to_vec(),
            region: core::str::from_utf8(profile.region).ok()?.into(),
            bucket: core::str::from_utf8(profile.bucket).ok()?.into(),
            prefix: core::str::from_utf8(profile.prefix).ok()?.into(),
            access_key: core::str::from_utf8(profile.access_key).ok()?.into(),
            secret_key: Zeroizing::new(profile.secret_key.to_vec()),
            namespace: if profile.namespace.is_empty() {
                None
            } else {
                Some(core::str::from_utf8(profile.namespace).ok()?.into())
            },
            rights: profile.rights,
        })
    }

    fn from_manifest(ctx: &Context) -> Option<Self> {
        let ip = match ctx.manifest_value(protocol::manifest::IP)? {
            ManifestValue::Bytes(bytes) if bytes.len() == 4 => {
                [bytes[0], bytes[1], bytes[2], bytes[3]]
            }
            _ => return None,
        };
        let host = manifest_text(ctx, protocol::manifest::HOST)?;
        let bucket = manifest_text(ctx, protocol::manifest::BUCKET)?;
        let access_key = manifest_text(ctx, protocol::manifest::ACCESS_KEY)?;
        let secret_key = manifest_bytes(ctx, protocol::manifest::SECRET_KEY)?;
        let region = manifest_text(ctx, protocol::manifest::REGION)
            .unwrap_or_else(|| "us-east-1".to_string());
        let prefix = manifest_text(ctx, protocol::manifest::PREFIX).unwrap_or_default();
        let namespace = manifest_text(ctx, protocol::manifest::NAMESPACE);
        let tls =
            matches!(ctx.manifest_value(protocol::manifest::TLS), Some(ManifestValue::Unsigned(1)))
                || matches!(
                    ctx.manifest_value(protocol::manifest::TLS),
                    Some(ManifestValue::Bytes(b"1"))
                );
        let ca_der = manifest_bytes(ctx, protocol::manifest::CA_DER).unwrap_or_default();
        let port = match ctx.manifest_value(protocol::manifest::PORT) {
            Some(ManifestValue::Unsigned(port)) => u16::try_from(port).ok()?,
            _ if tls => 443,
            _ => 80,
        };
        let rights = match ctx.manifest_value(protocol::manifest::RIGHTS)? {
            ManifestValue::Unsigned(rights) => rights,
            _ => return None,
        };
        Self::validate(Self {
            ip,
            host,
            port,
            tls,
            ca_der,
            region,
            bucket,
            prefix,
            access_key,
            secret_key: Zeroizing::new(secret_key),
            namespace,
            rights,
        })
    }

    fn validate(profile: Self) -> Option<Self> {
        if profile.port == 0
            || !valid_host(&profile.host)
            || profile.bucket.is_empty()
            || profile.bucket.bytes().any(|byte| byte <= b' ' || byte >= 0x7f || byte == b'/')
            || profile.access_key.is_empty()
            || !valid_header_value(&profile.access_key)
            || profile.secret_key.is_empty()
            || profile.region.is_empty()
            || profile.region.bytes().any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
            || profile.namespace.as_deref().is_some_and(|value| !valid_header_value(value))
            || profile.rights
                & !(protocol::RIGHT_GET
                    | protocol::RIGHT_PUT
                    | protocol::RIGHT_DELETE
                    | protocol::RIGHT_LIST)
                != 0
            || profile.rights == 0
            || profile.prefix.starts_with('/')
            || unsafe_path_segments(&profile.prefix)
            || (profile.tls && profile.ca_der.is_empty())
        {
            return None;
        }
        Some(profile)
    }

    fn has(&self, right: u64) -> bool {
        self.rights & right != 0
    }

    fn object_path(&self, key: &[u8]) -> Option<String> {
        let key = core::str::from_utf8(key).ok()?;
        if key.starts_with('/') || key.as_bytes().contains(&0) || unsafe_path_segments(key) {
            return None;
        }
        let mut path = format!("/{}/", self.bucket);
        if !self.prefix.is_empty() {
            path.push_str(self.prefix.trim_end_matches('/'));
            path.push('/');
        }
        path.push_str(key);
        Some(path)
    }

    fn authority(&self) -> String {
        if (self.tls && self.port == 443) || (!self.tls && self.port == 80) {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn valid_header_value(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
}

fn valid_host(host: &str) -> bool {
    valid_header_value(host)
        && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn unsafe_path_segments(path: &str) -> bool {
    path.split('/').any(|segment| segment == "." || segment == "..")
}

fn manifest_text(ctx: &Context, key: u64) -> Option<String> {
    let ManifestValue::Bytes(bytes) = ctx.manifest_value(key)? else {
        return None;
    };
    core::str::from_utf8(bytes).ok().map(ToString::to_string)
}

fn manifest_bytes(ctx: &Context, key: u64) -> Option<Vec<u8>> {
    let ManifestValue::Bytes(bytes) = ctx.manifest_value(key)? else {
        return None;
    };
    Some(bytes.to_vec())
}

#[derive(Clone)]
struct OwnedRequest {
    flags: u16,
    content_length: u64,
    range_start: u64,
    range_end: u64,
    payload_sha256: [u8; 32],
    key: Vec<u8>,
}

fn decode_request(memory: Option<OwnedMemory>, exact_len: u64) -> Option<OwnedRequest> {
    let memory = memory?;
    let exact_len = usize::try_from(exact_len).ok()?;
    let mapping = memory.map_read_only().ok()?;
    if exact_len > mapping.len() {
        return None;
    }
    let request = ObjectRequest::decode(&mapping.as_slice()[..exact_len])?;
    Some(OwnedRequest {
        flags: request.flags,
        content_length: request.content_length,
        range_start: request.range_start,
        range_end: request.range_end,
        payload_sha256: request.payload_sha256,
        key: request.key.to_vec(),
    })
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn time_snapshot(connection: ConnectionRef<'_>) -> Option<time::TimeSnapshot> {
    let result = connection.call(time::OP_NOW, 0).ok()?.wait().ok()?;
    if result.result != time::SNAPSHOT_LEN as i64 {
        return None;
    }
    let mapping = result.memory?.map_read_only().ok()?;
    time::TimeSnapshot::decode(mapping.as_slice())
}

fn time_now(connection: ConnectionRef<'_>) -> Option<(Timestamp, u64)> {
    let snapshot = time_snapshot(connection)?;
    if snapshot.state != time::STATE_SYNCHRONIZED || snapshot.unix_seconds <= 0 {
        return None;
    }
    Some((
        Timestamp {
            year: snapshot.utc.year,
            month: snapshot.utc.month,
            day: snapshot.utc.day,
            hour: snapshot.utc.hour,
            minute: snapshot.utc.minute,
            second: snapshot.utc.second,
        },
        snapshot.unix_seconds as u64,
    ))
}

fn monotonic_seconds(connection: ConnectionRef<'_>) -> Option<u64> {
    let snapshot = time_snapshot(connection)?;
    (snapshot.counter_frequency_hz > 0)
        .then_some(snapshot.monotonic_ticks / snapshot.counter_frequency_hz)
}

enum Transport<'connection> {
    Plain(socket::OwnedSocket<'connection>),
    Tls(Box<tls_client::OwnedTlsStream<'connection>>),
}

impl Transport<'_> {
    fn send_all(&mut self, bytes: &[u8]) -> Result<(), ()> {
        match self {
            Self::Plain(socket) => {
                socket.send_all(bytes, SEND_ATTEMPTS, SEND_RETRY_MS).map_err(|_| ())
            }
            Self::Tls(stream) => stream.send_all(bytes).map_err(|_| ()),
        }
    }

    fn receive(&mut self) -> Result<Vec<u8>, ()> {
        match self {
            Self::Plain(socket) => receive_plain(socket).ok_or(()),
            Self::Tls(stream) => stream.receive().map_err(|_| ()),
        }
    }

    fn close(self) -> Result<(), ()> {
        match self {
            Self::Plain(socket) => socket.close().map_err(|_| ()),
            Self::Tls(stream) => stream.close().map_err(|_| ()),
        }
    }
}

fn connect<'connection>(
    tcp: ConnectionRef<'connection>,
    entropy: Option<ConnectionRef<'connection>>,
    profile: &Profile,
    unix_seconds: u64,
) -> Result<Transport<'connection>, ()> {
    let socket = socket::OwnedSocket::open(tcp, socket::DOMAIN_TCP).map_err(|_| ())?;
    socket.connect_ipv4(profile.ip, profile.port).map_err(|_| ())?;
    if profile.tls {
        let result = tls_client::OwnedTlsStream::open(
            socket,
            entropy,
            tls_client::OpenConfig {
                server_name: &profile.host,
                ca_certificate_der: &profile.ca_der,
                client_certificate_der: None,
                client_private_key_der: None,
                unix_seconds,
                socket_bounds: tls_client::SocketBounds {
                    send_attempts: SEND_ATTEMPTS,
                    send_retry_ms: SEND_RETRY_MS,
                    receive_attempts: RECEIVE_ATTEMPTS,
                    receive_retry_ms: RECEIVE_RETRY_MS,
                    receive_chunk_len: protocol::MAX_CHUNK_LEN,
                },
            },
        );
        match result {
            Ok(stream) => Ok(Transport::Tls(Box::new(stream))),
            Err(tls_client::OpenError::Handshake(code)) => {
                config::write::<u32>(status::ERROR, 0x5400 | code);
                catten_rt::logln!("[s3] TLS handshake verification failed");
                Err(())
            }
            Err(tls_client::OpenError::EntropyUnavailable) => {
                config::write::<u32>(status::ERROR, 0x5400);
                catten_rt::logln!("[s3] TLS unavailable: system entropy source failed");
                Err(())
            }
            Err(tls_client::OpenError::InvalidConfiguration) => Err(()),
        }
    } else {
        Ok(Transport::Plain(socket))
    }
}

fn receive_plain(socket: &socket::OwnedSocket<'_>) -> Option<Vec<u8>> {
    let chunk = socket.receive_timeout(RECEIVE_ATTEMPTS, RECEIVE_RETRY_MS).ok()??;
    let (memory, len) = chunk.into_parts();
    let mapping = memory.map_read_only().ok()?;
    Some(mapping.as_slice()[..len].to_vec())
}

fn read_head(socket: &mut Transport<'_>) -> Option<(ResponseHead, Vec<u8>)> {
    let mut bytes = Vec::new();
    for _ in 0..MAX_HEAD_READS {
        bytes.extend_from_slice(&socket.receive().ok()?);
        match ResponseHead::parse(&bytes) {
            Ok((head, body_offset)) => {
                let body = bytes.split_off(body_offset);
                return Some((head, body));
            }
            Err(HttpError::Incomplete) => {}
            Err(_) => return None,
        }
    }
    None
}

fn signed_head(
    profile: &Profile,
    request: &OwnedRequest,
    method: &str,
    timestamp: Timestamp,
) -> Option<Vec<u8>> {
    let path = profile.object_path(&request.key)?;
    let encoded_path = canonical_uri(&path).ok()?;
    let authority = profile.authority();
    let amz_date = timestamp.amz_date().ok()?;
    let payload_hash = if method == "PUT" {
        hex_lower(&request.payload_sha256)
    } else {
        EMPTY_PAYLOAD_SHA256.to_string()
    };
    let range = if request.flags & protocol::FLAG_RANGE == 0 {
        None
    } else if request.range_end == 0 {
        Some(format!("bytes={}-", request.range_start))
    } else {
        Some(format!("bytes={}-{}", request.range_start, request.range_end - 1))
    };
    let mut headers = Vec::with_capacity(6);
    headers.push(Header {
        name: "host",
        value: &authority,
    });
    headers.push(Header {
        name: "x-amz-content-sha256",
        value: &payload_hash,
    });
    headers.push(Header {
        name: "x-amz-date",
        value: &amz_date,
    });
    if let Some(namespace) = profile.namespace.as_deref() {
        headers.push(Header {
            name: "x-emc-namespace",
            value: namespace,
        });
    }
    if let Some(range) = range.as_deref() {
        headers.push(Header {
            name: "range",
            value: range,
        });
    }
    if request.flags & protocol::FLAG_IF_NONE_MATCH != 0 {
        headers.push(Header {
            name: "if-none-match",
            value: "*",
        });
    }
    let signature = sign(
        &SignRequest {
            method,
            path: &path,
            query: &[],
            headers: &headers,
            payload_sha256: &payload_hash,
            region: &profile.region,
            service: "s3",
            timestamp,
        },
        Credentials {
            access_key: &profile.access_key,
            secret_key: &profile.secret_key,
        },
    )
    .ok()?;

    let mut output = String::new();
    let _ = write!(
        output,
        "{} {} HTTP/1.1\r\nHost: {}\r\nx-amz-content-sha256: {}\r\nx-amz-date: \
         {}\r\nAuthorization: {}\r\n",
        method, encoded_path, authority, payload_hash, amz_date, signature.authorization
    );
    if let Some(namespace) = profile.namespace.as_deref() {
        let _ = write!(output, "x-emc-namespace: {}\r\n", namespace);
    }
    if let Some(range) = range {
        let _ = write!(output, "Range: {}\r\n", range);
    }
    if request.flags & protocol::FLAG_IF_NONE_MATCH != 0 {
        output.push_str("If-None-Match: *\r\n");
    }
    if method == "PUT" {
        let _ = write!(output, "Content-Length: {}\r\n", request.content_length);
    }
    output.push_str("Connection: close\r\n\r\n");
    Some(output.into_bytes())
}

fn remote_error(status: u16) -> i64 {
    match status {
        404 => protocol::ERR_NOT_FOUND,
        409 | 412 => protocol::ERR_PRECONDITION,
        _ => protocol::ERR_REMOTE,
    }
}

fn metadata_memory(head: &ResponseHead) -> Option<(OwnedMemory, usize)> {
    let metadata = ObjectMetadata {
        status: head.status,
        content_length: head.content_length.unwrap_or(0),
        etag: head.etag.as_deref().unwrap_or_default().as_bytes(),
        version_id: head.version_id.as_deref().unwrap_or_default().as_bytes(),
        request_id: head.request_id.as_deref().unwrap_or_default().as_bytes(),
    };
    let memory = OwnedMemory::allocate(1).ok()?;
    let mut mapping = memory.map_writable().ok()?;
    let len = metadata.encode(mapping.as_mut_slice())?;
    let memory = mapping.unmap().ok()?;
    Some((memory, len))
}

enum Body {
    Length(u64),
    Chunked(ChunkedDecoder),
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ClientIdentity {
    domain: u64,
    generation: u64,
}

struct GetState<'connection> {
    socket: Transport<'connection>,
    owner: ClientIdentity,
    last_activity: u64,
    body: Body,
    buffered: Vec<u8>,
    offset: usize,
}

impl GetState<'_> {
    fn next(&mut self) -> Result<Option<Vec<u8>>, ()> {
        loop {
            if self.offset < self.buffered.len() {
                let end = (self.offset + protocol::MAX_CHUNK_LEN).min(self.buffered.len());
                let result = self.buffered[self.offset..end].to_vec();
                self.offset = end;
                if self.offset == self.buffered.len() {
                    self.buffered.clear();
                    self.offset = 0;
                }
                if let Body::Length(remaining) = &mut self.body {
                    if result.len() as u64 > *remaining {
                        return Err(());
                    }
                    *remaining -= result.len() as u64;
                }
                return Ok(Some(result));
            }
            match &mut self.body {
                Body::Length(0) => return Ok(None),
                Body::Length(_) => self.buffered = self.socket.receive()?,
                Body::Chunked(decoder) if decoder.is_complete() => return Ok(None),
                Body::Chunked(decoder) => {
                    let raw = self.socket.receive()?;
                    decoder.decode(&raw, &mut self.buffered).map_err(|_| ())?;
                }
            }
        }
    }
}

struct PutState<'connection> {
    socket: Transport<'connection>,
    owner: ClientIdentity,
    last_activity: u64,
    expected: u64,
    written: u64,
    expected_sha256: [u8; 32],
    sha256: Sha256,
}

fn reply_bytes(reply: ReplyToken, bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > protocol::MAX_CHUNK_LEN {
        let _ = reply.reply(protocol::ERR_PROTOCOL);
        return false;
    }
    let Ok(memory) = OwnedMemory::allocate(1) else {
        let _ = reply.reply(protocol::ERR_TRANSPORT);
        return false;
    };
    let Ok(mut mapping) = memory.map_writable() else {
        let _ = reply.reply(protocol::ERR_TRANSPORT);
        return false;
    };
    mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
    let Ok(memory) = mapping.unmap() else {
        let _ = reply.reply(protocol::ERR_TRANSPORT);
        return false;
    };
    reply.reply_move(memory, bytes.len() as i64).is_ok()
}

struct Service<'connection> {
    profile: Profile,
    tcp: ConnectionRef<'connection>,
    clock: ConnectionRef<'connection>,
    entropy: Option<ConnectionRef<'connection>>,
    gets: BTreeMap<u64, GetState<'connection>>,
    puts: BTreeMap<u64, PutState<'connection>>,
    next_id: u64,
    requests: u32,
    failures: u32,
}

impl<'connection> Service<'connection> {
    fn operation_id(&mut self) -> Option<u64> {
        if self.gets.len() + self.puts.len() >= MAX_OPERATIONS {
            return None;
        }
        loop {
            let id = self.next_id.max(1).min(u32::MAX as u64);
            self.next_id = if id == u32::MAX as u64 {
                1
            } else {
                id + 1
            };
            if !self.gets.contains_key(&id) && !self.puts.contains_key(&id) {
                return Some(id);
            }
        }
    }

    fn begin_get(
        &mut self,
        request: OwnedRequest,
        owner: ClientIdentity,
        now: u64,
    ) -> Result<(u64, ResponseHead), i64> {
        if !self.profile.has(protocol::RIGHT_GET) {
            return Err(protocol::ERR_DENIED);
        }
        if request.flags & protocol::FLAG_IF_NONE_MATCH != 0
            || request.content_length != 0
            || request.payload_sha256 != [0; 32]
        {
            return Err(protocol::ERR_INVALID);
        }
        let id = self.operation_id().ok_or(protocol::ERR_BUSY)?;
        let (timestamp, unix_seconds) = time_now(self.clock).ok_or(protocol::ERR_UNSYNCHRONIZED)?;
        let mut socket = connect(self.tcp, self.entropy, &self.profile, unix_seconds)
            .map_err(|_| protocol::ERR_TRANSPORT)?;
        let head =
            signed_head(&self.profile, &request, "GET", timestamp).ok_or(protocol::ERR_INVALID)?;
        if socket.send_all(&head).is_err() {
            return Err(protocol::ERR_TRANSPORT);
        }
        let (response, initial_body) = read_head(&mut socket).ok_or(protocol::ERR_PROTOCOL)?;
        if response.status != 200 && response.status != 206 {
            return Err(remote_error(response.status));
        }
        let body = if response.chunked {
            let mut decoder = ChunkedDecoder::new();
            let mut decoded = Vec::new();
            decoder.decode(&initial_body, &mut decoded).map_err(|_| protocol::ERR_PROTOCOL)?;
            self.gets.insert(
                id,
                GetState {
                    socket,
                    owner,
                    last_activity: now,
                    body: Body::Chunked(decoder),
                    buffered: decoded,
                    offset: 0,
                },
            );
            return Ok((id, response));
        } else {
            Body::Length(response.content_length.ok_or(protocol::ERR_PROTOCOL)?)
        };
        self.gets.insert(
            id,
            GetState {
                socket,
                owner,
                last_activity: now,
                body,
                buffered: initial_body,
                offset: 0,
            },
        );
        Ok((id, response))
    }

    fn begin_put(
        &mut self,
        request: OwnedRequest,
        owner: ClientIdentity,
        now: u64,
    ) -> Result<u64, i64> {
        if !self.profile.has(protocol::RIGHT_PUT) {
            return Err(protocol::ERR_DENIED);
        }
        if request.flags & protocol::FLAG_RANGE != 0
            || request.range_start != 0
            || request.range_end != 0
        {
            return Err(protocol::ERR_INVALID);
        }
        let id = self.operation_id().ok_or(protocol::ERR_BUSY)?;
        let (timestamp, unix_seconds) = time_now(self.clock).ok_or(protocol::ERR_UNSYNCHRONIZED)?;
        let mut socket = connect(self.tcp, self.entropy, &self.profile, unix_seconds)
            .map_err(|_| protocol::ERR_TRANSPORT)?;
        let head =
            signed_head(&self.profile, &request, "PUT", timestamp).ok_or(protocol::ERR_INVALID)?;
        if socket.send_all(&head).is_err() {
            return Err(protocol::ERR_TRANSPORT);
        }
        self.puts.insert(
            id,
            PutState {
                socket,
                owner,
                last_activity: now,
                expected: request.content_length,
                written: 0,
                expected_sha256: request.payload_sha256,
                sha256: Sha256::new(),
            },
        );
        Ok(id)
    }

    fn simple_request(&self, request: &OwnedRequest, method: &str) -> Result<ResponseHead, i64> {
        let (timestamp, unix_seconds) = time_now(self.clock).ok_or(protocol::ERR_UNSYNCHRONIZED)?;
        let mut socket = connect(self.tcp, self.entropy, &self.profile, unix_seconds)
            .map_err(|_| protocol::ERR_TRANSPORT)?;
        let request_head =
            signed_head(&self.profile, request, method, timestamp).ok_or(protocol::ERR_INVALID)?;
        if socket.send_all(&request_head).is_err() {
            return Err(protocol::ERR_TRANSPORT);
        }
        let (head, _) = read_head(&mut socket).ok_or(protocol::ERR_PROTOCOL)?;
        let _ = socket.close();
        Ok(head)
    }

    fn account(&mut self, result: i64) {
        self.requests = self.requests.wrapping_add(1);
        if result < 0 {
            self.failures = self.failures.wrapping_add(1);
        }
        config::write::<u32>(status::REQUESTS, self.requests);
        config::write::<u32>(status::FAILURES, self.failures);
        config::write::<u32>(status::ACTIVE_GETS, self.gets.len() as u32);
        config::write::<u32>(status::ACTIVE_PUTS, self.puts.len() as u32);
    }

    fn sweep_expired(&mut self, now: u64) {
        self.gets.retain(|_, operation| {
            now.saturating_sub(operation.last_activity) <= OPERATION_LEASE_SECONDS
        });
        self.puts.retain(|_, operation| {
            now.saturating_sub(operation.last_activity) <= OPERATION_LEASE_SECONDS
        });
    }
}

fn handle_message(service: &mut Service<'_>, mut message: catten_rt::owned::IncomingMessage) {
    let Some(reply) = message.reply.take() else {
        return;
    };
    let owner = ClientIdentity {
        domain: message.sender,
        generation: message.sender_generation,
    };
    let now = monotonic_seconds(service.clock).unwrap_or(0);
    service.sweep_expired(now);
    let mut accounted = 0;
    match message.opcode {
        protocol::OP_GET_BEGIN => {
            let Some(request) = decode_request(message.memory.take(), message.arg0) else {
                accounted = protocol::ERR_INVALID;
                let _ = reply.reply(accounted);
                service.account(accounted);
                return;
            };
            match service.begin_get(request, owner, now) {
                Ok((id, head)) => {
                    let Some((memory, _)) = metadata_memory(&head) else {
                        service.gets.remove(&id);
                        accounted = protocol::ERR_TRANSPORT;
                        let _ = reply.reply(accounted);
                        service.account(accounted);
                        return;
                    };
                    if reply.reply_move(memory, id as i64).is_err() {
                        service.gets.remove(&id);
                        accounted = protocol::ERR_TRANSPORT;
                    }
                }
                Err(error) => {
                    accounted = error;
                    let _ = reply.reply(error);
                }
            }
        }
        protocol::OP_GET_READ => {
            match service.gets.get_mut(&message.arg0).filter(|get| get.owner == owner) {
                Some(get) => match get.next() {
                    Ok(Some(bytes)) => {
                        get.last_activity = now;
                        if !reply_bytes(reply, &bytes) {
                            service.gets.remove(&message.arg0);
                            accounted = protocol::ERR_TRANSPORT;
                        }
                    }
                    Ok(None) => {
                        if reply.reply(0).is_err() {
                            service.gets.remove(&message.arg0);
                            accounted = protocol::ERR_TRANSPORT;
                        }
                    }
                    Err(()) => {
                        service.gets.remove(&message.arg0);
                        accounted = protocol::ERR_TRANSPORT;
                        let _ = reply.reply(accounted);
                    }
                },
                None => {
                    accounted = protocol::ERR_INVALID;
                    let _ = reply.reply(accounted);
                }
            }
        }
        protocol::OP_GET_CLOSE => {
            let owned = service.gets.get(&message.arg0).is_some_and(|get| get.owner == owner);
            if owned && let Some(get) = service.gets.remove(&message.arg0) {
                let _ = get.socket.close();
                let _ = reply.reply(0);
            } else {
                accounted = protocol::ERR_INVALID;
                let _ = reply.reply(accounted);
            }
        }
        protocol::OP_PUT_BEGIN => {
            let Some(request) = decode_request(message.memory.take(), message.arg0) else {
                accounted = protocol::ERR_INVALID;
                let _ = reply.reply(accounted);
                service.account(accounted);
                return;
            };
            match service.begin_put(request, owner, now) {
                Ok(id) => {
                    if reply.reply(id as i64).is_err() {
                        service.puts.remove(&id);
                        accounted = protocol::ERR_TRANSPORT;
                    }
                }
                Err(error) => {
                    accounted = error;
                    let _ = reply.reply(error);
                }
            }
        }
        protocol::OP_PUT_WRITE => {
            let id = message.arg0 & 0xffff_ffff;
            let len = (message.arg0 >> 32) as usize;
            let result = if len == 0 || len > protocol::MAX_CHUNK_LEN {
                Err(protocol::ERR_INVALID)
            } else if let Some(put) = service.puts.get_mut(&id).filter(|put| put.owner == owner) {
                let memory = message.memory.take().ok_or(protocol::ERR_INVALID);
                memory.and_then(|memory| {
                    let mapping = memory.map_read_only().map_err(|_| protocol::ERR_TRANSPORT)?;
                    if len > mapping.len() || put.written.saturating_add(len as u64) > put.expected
                    {
                        return Err(protocol::ERR_INVALID);
                    }
                    if put.socket.send_all(&mapping.as_slice()[..len]).is_err() {
                        return Err(protocol::ERR_TRANSPORT);
                    }
                    put.sha256.update(&mapping.as_slice()[..len]);
                    put.written += len as u64;
                    put.last_activity = now;
                    Ok(len as i64)
                })
            } else {
                Err(protocol::ERR_INVALID)
            };
            if result == Err(protocol::ERR_TRANSPORT) {
                service.puts.remove(&id);
            }
            accounted = result.unwrap_or_else(|error| error);
            if reply.reply(accounted).is_err() {
                service.puts.remove(&id);
                accounted = protocol::ERR_TRANSPORT;
            }
        }
        protocol::OP_PUT_FINISH => {
            let owned = service.puts.get(&message.arg0).is_some_and(|put| put.owner == owner);
            let result = if owned {
                service.puts.remove(&message.arg0).ok_or(protocol::ERR_INVALID).and_then(
                    |mut put| {
                        if put.written != put.expected {
                            return Err(protocol::ERR_INVALID);
                        }
                        if put.sha256.finalize() != put.expected_sha256 {
                            return Err(protocol::ERR_INVALID);
                        }
                        let (head, _) = read_head(&mut put.socket).ok_or(protocol::ERR_PROTOCOL)?;
                        if !(200..300).contains(&head.status) {
                            return Err(remote_error(head.status));
                        }
                        let metadata = metadata_memory(&head).ok_or(protocol::ERR_TRANSPORT)?;
                        let _ = put.socket.close();
                        Ok(metadata)
                    },
                )
            } else {
                Err(protocol::ERR_DENIED)
            };
            match result {
                Ok((memory, len)) => {
                    let _ = reply.reply_move(memory, len as i64);
                }
                Err(error) => {
                    accounted = error;
                    let _ = reply.reply(error);
                }
            }
        }
        protocol::OP_PUT_ABORT => {
            let owned = service.puts.get(&message.arg0).is_some_and(|put| put.owner == owner);
            if owned && let Some(put) = service.puts.remove(&message.arg0) {
                let _ = put.socket.close();
                let _ = reply.reply(0);
            } else {
                accounted = protocol::ERR_INVALID;
                let _ = reply.reply(accounted);
            }
        }
        protocol::OP_HEAD | protocol::OP_DELETE => {
            let Some(request) = decode_request(message.memory.take(), message.arg0) else {
                accounted = protocol::ERR_INVALID;
                let _ = reply.reply(accounted);
                service.account(accounted);
                return;
            };
            if request.flags != 0
                || request.content_length != 0
                || request.range_start != 0
                || request.range_end != 0
                || request.payload_sha256 != [0; 32]
            {
                accounted = protocol::ERR_INVALID;
                let _ = reply.reply(accounted);
                service.account(accounted);
                return;
            }
            let right = if message.opcode == protocol::OP_HEAD {
                protocol::RIGHT_GET
            } else {
                protocol::RIGHT_DELETE
            };
            if !service.profile.has(right) {
                accounted = protocol::ERR_DENIED;
                let _ = reply.reply(accounted);
            } else {
                let method = if message.opcode == protocol::OP_HEAD {
                    "HEAD"
                } else {
                    "DELETE"
                };
                match service.simple_request(&request, method) {
                    Ok(head)
                        if message.opcode == protocol::OP_HEAD
                            && (200..300).contains(&head.status) =>
                    {
                        let Some((memory, len)) = metadata_memory(&head) else {
                            accounted = protocol::ERR_TRANSPORT;
                            let _ = reply.reply(accounted);
                            service.account(accounted);
                            return;
                        };
                        let _ = reply.reply_move(memory, len as i64);
                    }
                    Ok(head)
                        if message.opcode == protocol::OP_DELETE
                            && (200..300).contains(&head.status) =>
                    {
                        let _ = reply.reply(0);
                    }
                    Ok(head) => {
                        accounted = remote_error(head.status);
                        let _ = reply.reply(accounted);
                    }
                    Err(error) => {
                        accounted = error;
                        let _ = reply.reply(error);
                    }
                }
            }
        }
        protocol::OP_STATUS => {
            // Packed without credentials: rights (low 32), TLS bit, port.
            let summary = (service.profile.rights & 0xffff_ffff)
                | ((service.profile.tls as u64) << 32)
                | ((service.profile.port as u64) << 40);
            let _ = reply.reply(summary as i64);
        }
        _ => {
            accounted = protocol::ERR_BAD_OPCODE;
            let _ = reply.reply(accounted);
        }
    }
    service.account(accounted);
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let profile = Profile::from_context(ctx).unwrap_or_else(|| fail(0xe001));
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xe002));
    let (_, tcp_connection) =
        wait_for_registered_name_owned(ns_connection, socket::NAME).unwrap_or_else(|| fail(0xe003));
    let (_, time_connection) =
        wait_for_registered_name_owned(ns_connection, time::NAME).unwrap_or_else(|| fail(0xe004));
    if !wait_for_local_ready_owned(ns_connection) {
        fail(0xe005);
    }
    let entropy_connection =
        try_registered_name_owned(ns_connection, entropy::NAME).map(|(_, connection)| connection);

    let endpoint = Endpoint::create(protocol::INTERFACE, protocol::VERSION, 32)
        .unwrap_or_else(|_| fail(0xe006));
    let registration = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            protocol::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| fail(0xe007));
    if !registration.wait().is_ok_and(|reply| reply.result >= 1)
        || endpoint.bind_completion_queue(0).is_err()
    {
        fail(0xe007);
    }
    catten_rt::logln!(
        "[s3] serving endpoint={}:{} bucket={} prefix={} region={} tls={} rights={:#x}",
        profile.host,
        profile.port,
        profile.bucket,
        profile.prefix,
        profile.region,
        profile.tls,
        profile.rights
    );
    config::write::<u32>(status::STAGE, 2);

    let mut service = Service {
        profile,
        tcp: tcp_connection.as_ref(),
        clock: time_connection.as_ref(),
        entropy: entropy_connection.as_ref().map(Connection::as_ref),
        gets: BTreeMap::new(),
        puts: BTreeMap::new(),
        next_id: 1,
        requests: 0,
        failures: 0,
    };
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            drop(service);
            drop(endpoint);
            return request;
        }
        match endpoint.try_receive() {
            Ok(None) => sleep_ms(10),
            Ok(Some(message)) => handle_message(&mut service, message),
            Err(catten_rt::owned::ReceiveError::EndpointClosed) => fail(0xe008),
            Err(_) => {
                service.failures = service.failures.wrapping_add(1);
                config::write::<u32>(status::FAILURES, service.failures);
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}
