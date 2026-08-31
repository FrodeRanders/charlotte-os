//! Bounded off-cluster deployment-notification ingress.
//!
//! The listener accepts `POST /v1/deployments` with one signed `CDEPLOY3`
//! descriptor, `POST /v1/releases` with one signed `CRELEASE` component set,
//! `POST /v1/operations` with one encrypted `COPSBND2` admission proof,
//! and `GET /v1/deployments/{percent-encoded-name}` for rollout observation.
//! The ingress carries no object-store or application secret: authenticity,
//! integrity, placement, and authority come from signatures checked by
//! `clusterctl`. Assigned nodes fetch referenced ELFs through their separately
//! provisioned S3 service.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    format,
    vec::Vec,
};

use catten_rt::{
    Context,
    owned::OwnedMemory,
};
use catten_services::{
    clusterctl,
    sleep_ms,
    socket,
    wait_for_local_ready_owned,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;

const HEADER_LIMIT: usize = 1024;
const ACCEPT_RETRY_MS: u64 = 50;
const RECEIVE_RETRIES: usize = 300;
const RECEIVE_RETRY_MS: u64 = 100;

enum Request {
    Notify(Vec<u8>),
    Release(Vec<u8>),
    Operations(Vec<u8>),
    Status(Vec<u8>),
}

fn fail() -> ! {
    unsafe { thread_exit() }
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n").map(|offset| offset + 4)
}

fn content_length(headers: &[u8]) -> Option<usize> {
    for raw_line in headers.split(|byte| *byte == b'\n').skip(1) {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let colon = line.iter().position(|byte| *byte == b':')?;
        if line[..colon].eq_ignore_ascii_case(b"content-length") {
            let value = line[colon + 1..]
                .iter()
                .copied()
                .skip_while(|byte| byte.is_ascii_whitespace())
                .take_while(|byte| !byte.is_ascii_whitespace());
            let mut length = 0usize;
            let mut digits = 0usize;
            for byte in value {
                if !byte.is_ascii_digit() {
                    return None;
                }
                length = length.checked_mul(10)?.checked_add(usize::from(byte - b'0'))?;
                digits += 1;
            }
            return (digits > 0).then_some(length);
        }
    }
    None
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_path_name(path: &[u8]) -> Option<Vec<u8>> {
    let prefix = clusterctl::NOTIFY_PATH;
    if !path.starts_with(prefix) || path.get(prefix.len()) != Some(&b'/') {
        return None;
    }
    let encoded = path.get(prefix.len() + 1..)?;
    let mut name = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        let byte = encoded[index];
        if byte == b'%' {
            let high = hex_digit(*encoded.get(index + 1)?)?;
            let low = hex_digit(*encoded.get(index + 2)?)?;
            name.push((high << 4) | low);
            index += 3;
        } else {
            name.push(byte);
            index += 1;
        }
    }
    (charlotte_launch::deployment::valid_artifact_name(&name)).then_some(name)
}

fn complete_request(request: &[u8]) -> Result<Option<Request>, ()> {
    let Some(body_start) = header_end(request) else {
        return if request.len() <= HEADER_LIMIT {
            Ok(None)
        } else {
            Err(())
        };
    };
    if body_start > HEADER_LIMIT {
        return Err(());
    }
    let request_line_end = request.iter().position(|byte| *byte == b'\r').ok_or(())?;
    let request_line = request.get(..request_line_end).ok_or(())?;
    let mut words = request_line.split(|byte| *byte == b' ');
    let method = words.next().ok_or(())?;
    let path = words.next().ok_or(())?;
    if !words.next().is_some_and(|version| version.starts_with(b"HTTP/1."))
        || words.next().is_some()
    {
        return Err(());
    }
    if method == b"GET" {
        return decode_path_name(path).map(Request::Status).map(Some).ok_or(());
    }
    if method != b"POST" {
        return Err(());
    }
    let (minimum, maximum) = match path {
        clusterctl::NOTIFY_PATH => (
            charlotte_launch::deployment::HEADER_LEN,
            charlotte_launch::deployment::MAX_DESCRIPTOR_LEN,
        ),
        clusterctl::RELEASE_PATH => {
            (charlotte_launch::release::HEADER_LEN, charlotte_launch::release::MAX_RELEASE_LEN)
        }
        clusterctl::OPERATIONS_PATH => (
            charlotte_launch::operations_bundle::HEADER_LEN,
            charlotte_launch::operations_bundle::MAX_BUNDLE_LEN,
        ),
        _ => return Err(()),
    };
    let length = content_length(&request[..body_start]).ok_or(())?;
    if !(minimum..=maximum).contains(&length) {
        return Err(());
    }
    let end = body_start.checked_add(length).ok_or(())?;
    if request.len() < end {
        Ok(None)
    } else {
        request
            .get(body_start..end)
            .map(|body| {
                Some(
                    if path == clusterctl::RELEASE_PATH {
                        Request::Release(body.to_vec())
                    } else if path == clusterctl::OPERATIONS_PATH {
                        Request::Operations(body.to_vec())
                    } else {
                        Request::Notify(body.to_vec())
                    },
                )
            })
            .ok_or(())
    }
}

fn receive_request(socket: &socket::OwnedSocket<'_>) -> Result<Request, ()> {
    let mut request = Vec::new();
    loop {
        let chunk =
            socket.receive_timeout(RECEIVE_RETRIES, RECEIVE_RETRY_MS).map_err(|_| ())?.ok_or(())?;
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().map_err(|_| ())?;
        let bytes = mapping.as_slice().get(..len).ok_or(())?;
        if request.len().saturating_add(bytes.len())
            > HEADER_LIMIT + charlotte_launch::operations_bundle::MAX_BUNDLE_LEN
        {
            return Err(());
        }
        request.extend_from_slice(bytes);
        if let Some(request) = complete_request(&request)? {
            return Ok(request);
        }
    }
}

fn notify_cluster(controller: catten_rt::owned::ConnectionRef<'_>, descriptor: &[u8]) -> i64 {
    let Some(decoded) = charlotte_launch::deployment::decode(descriptor) else {
        return clusterctl::ERR_UNTRUSTED_DESCRIPTOR;
    };
    if decoded.artifact_name.is_empty() {
        return clusterctl::ERR_TOO_LARGE;
    }
    let memory = match OwnedMemory::allocate(1) {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    let mut mapping = match memory.map_writable() {
        Ok(mapping) => mapping,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    mapping.as_mut_slice()[..8].copy_from_slice(&(descriptor.len() as u64).to_le_bytes());
    mapping.as_mut_slice()[8..8 + descriptor.len()].copy_from_slice(descriptor);
    let memory = match mapping.unmap() {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    match controller.call_move(clusterctl::OP_NOTIFY, 0, memory) {
        Ok(call) => call.wait().map_or(clusterctl::ERR_NOT_LEADER, |reply| reply.result),
        Err((_memory, _error)) => clusterctl::ERR_NOT_LEADER,
    }
}

fn notify_release(controller: catten_rt::owned::ConnectionRef<'_>, envelope: &[u8]) -> i64 {
    if charlotte_launch::release::decode(envelope).is_none() {
        return clusterctl::ERR_UNTRUSTED_DESCRIPTOR;
    }
    let memory = match OwnedMemory::allocate(1) {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    let mut mapping = match memory.map_writable() {
        Ok(mapping) => mapping,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    mapping.as_mut_slice()[..8].copy_from_slice(&(envelope.len() as u64).to_le_bytes());
    mapping.as_mut_slice()[8..8 + envelope.len()].copy_from_slice(envelope);
    let memory = match mapping.unmap() {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    match controller.call_move(clusterctl::OP_NOTIFY_RELEASE, 0, memory) {
        Ok(call) => call.wait().map_or(clusterctl::ERR_NOT_LEADER, |reply| reply.result),
        Err((_memory, _error)) => clusterctl::ERR_NOT_LEADER,
    }
}

fn notify_operations(controller: catten_rt::owned::ConnectionRef<'_>, bundle: &[u8]) -> i64 {
    if charlotte_launch::operations_bundle::decode(bundle).is_none() {
        return clusterctl::ERR_UNTRUSTED_DESCRIPTOR;
    }
    let bytes = match bundle.len().checked_add(8) {
        Some(bytes) => bytes,
        None => return clusterctl::ERR_TOO_LARGE,
    };
    let memory = match OwnedMemory::allocate(bytes.div_ceil(4096).max(1)) {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    let mut mapping = match memory.map_writable() {
        Ok(mapping) => mapping,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    mapping.as_mut_slice()[..8].copy_from_slice(&(bundle.len() as u64).to_le_bytes());
    mapping.as_mut_slice()[8..8 + bundle.len()].copy_from_slice(bundle);
    let memory = match mapping.unmap() {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    match controller.call_move(clusterctl::OP_NOTIFY_OPERATIONS, 0, memory) {
        Ok(call) => call.wait().map_or(clusterctl::ERR_NOT_LEADER, |reply| reply.result),
        Err((_memory, _error)) => clusterctl::ERR_NOT_LEADER,
    }
}

fn query_rollout(
    controller: catten_rt::owned::ConnectionRef<'_>,
    name: &[u8],
) -> Result<clusterctl::RolloutStatus, i64> {
    let memory = OwnedMemory::allocate(1).map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let mut mapping = memory.map_writable().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    mapping.as_mut_slice()[..name.len()].copy_from_slice(name);
    let memory = mapping.unmap().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let reply = controller
        .call_move(clusterctl::OP_ROLLOUT, name.len() as u64, memory)
        .map_err(|_| clusterctl::ERR_NOT_LEADER)?
        .wait()
        .map_err(|_| clusterctl::ERR_NOT_LEADER)?;
    if reply.result < 0 {
        return Err(reply.result);
    }
    if reply.result != clusterctl::ROLLOUT_STATUS_LEN as i64 {
        return Err(clusterctl::ERR_NOT_FOUND);
    }
    let memory = reply.memory.ok_or(clusterctl::ERR_NOT_FOUND)?;
    let mapping = memory.map_read_only().map_err(|_| clusterctl::ERR_NOT_FOUND)?;
    clusterctl::RolloutStatus::decode(
        mapping
            .as_slice()
            .get(..clusterctl::ROLLOUT_STATUS_LEN)
            .ok_or(clusterctl::ERR_NOT_FOUND)?,
    )
    .ok_or(clusterctl::ERR_NOT_FOUND)
}

fn response(result: Result<i64, ()>) -> alloc::string::String {
    let (status, body) = match result {
        Ok(generation) if generation > 0 => {
            ("202 Accepted", format!("{{\"generation\":{generation}}}\n"))
        }
        Ok(clusterctl::ERR_UNTRUSTED_DESCRIPTOR) => {
            ("403 Forbidden", format!("{{\"error\":{}}}\n", clusterctl::ERR_UNTRUSTED_DESCRIPTOR))
        }
        Ok(clusterctl::ERR_STALE_DESCRIPTOR | clusterctl::ERR_CONFLICTING_DESCRIPTOR) => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", result.unwrap_or_default()))
        }
        Ok(clusterctl::ERR_EXPIRED_OPERATION) => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", clusterctl::ERR_EXPIRED_OPERATION))
        }
        Ok(code) => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
        Err(()) => ("400 Bad Request", "{\"error\":\"malformed request\"}\n".into()),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body}",
        body.len()
    )
}

fn rollout_response(result: Result<clusterctl::RolloutStatus, i64>) -> alloc::string::String {
    let (http_status, body) = match result {
        Ok(status) => {
            let state = match status.state {
                clusterctl::ROLLOUT_READY => "ready",
                clusterctl::ROLLOUT_REPLACING => "replacing",
                _ => "committed",
            };
            (
                "200 OK",
                format!(
                    "{{\"state\":\"{state}\",\"deployment_generation\":{},\"service_generation\":\
                     {},\"node_key\":{}}}\n",
                    status.deployment_generation, status.service_generation, status.node_key
                ),
            )
        }
        Err(clusterctl::ERR_NOT_FOUND) => {
            ("404 Not Found", "{\"error\":\"deployment not found\"}\n".into())
        }
        Err(code) => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
    };
    format!(
        "HTTP/1.1 {http_status}\r\nContent-Type: application/json\r\nContent-Length: \
         {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn main(ctx: Context) -> ! {
    let names = ctx.bootstrap_connection().unwrap_or_else(|| fail());
    let (_, tcp) = wait_for_registered_name_owned(names, socket::NAME).unwrap_or_else(|| fail());
    let (_, controller) =
        wait_for_registered_name_owned(names, clusterctl::NAME).unwrap_or_else(|| fail());
    if !wait_for_local_ready_owned(names) {
        fail();
    }

    loop {
        let socket =
            socket::OwnedSocket::open(tcp.as_ref(), socket::DOMAIN_TCP).unwrap_or_else(|_| fail());
        let port = OwnedMemory::allocate(1).unwrap_or_else(|_| fail());
        let mut mapping = port.map_writable().unwrap_or_else(|_| fail());
        mapping.as_mut_slice()[..2].copy_from_slice(&clusterctl::NOTIFY_PORT.to_le_bytes());
        let port = mapping.unmap().unwrap_or_else(|_| fail());
        let listen = tcp
            .as_ref()
            .call_move(socket::OP_LISTEN, socket.id(), port)
            .unwrap_or_else(|_| fail())
            .wait()
            .unwrap_or_else(|_| fail());
        if listen.result != 0 {
            fail();
        }
        loop {
            let accepted = socket
                .call(socket::OP_ACCEPT, socket.id())
                .unwrap_or_else(|_| fail())
                .wait()
                .unwrap_or_else(|_| fail())
                .result;
            if accepted == 0 {
                break;
            }
            if accepted != socket::ERR_WOULD_BLOCK {
                fail();
            }
            sleep_ms(ACCEPT_RETRY_MS);
        }

        let reply = match receive_request(&socket) {
            Ok(Request::Notify(descriptor)) => {
                response(Ok(notify_cluster(controller.as_ref(), &descriptor)))
            }
            Ok(Request::Release(envelope)) => {
                response(Ok(notify_release(controller.as_ref(), &envelope)))
            }
            Ok(Request::Operations(bundle)) => {
                response(Ok(notify_operations(controller.as_ref(), &bundle)))
            }
            Ok(Request::Status(name)) => {
                rollout_response(query_rollout(controller.as_ref(), &name))
            }
            Err(()) => response(Err(())),
        };
        let _ = socket.send_all(reply.as_bytes(), 1000, 5);
        let _ = socket.close();
    }
}

catten_rt::entry!(main);
