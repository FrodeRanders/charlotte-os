//! Bounded off-cluster deployment-notification ingress.
//!
//! The listener accepts `POST /v1/deployments` with one signed `CDEPLOY1`
//! descriptor as its body. It carries no object-store or application secret:
//! authenticity, integrity, placement, and authority all come from the
//! descriptor signature checked by `clusterctl`. The assigned node fetches
//! the referenced ELF through its separately provisioned S3 service.
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
    name,
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

fn complete_body(request: &[u8]) -> Result<Option<&[u8]>, ()> {
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
    if words.next() != Some(b"POST".as_slice())
        || words.next() != Some(clusterctl::NOTIFY_PATH)
        || !words.next().is_some_and(|version| version.starts_with(b"HTTP/1."))
        || words.next().is_some()
    {
        return Err(());
    }
    let length = content_length(&request[..body_start]).ok_or(())?;
    if !(charlotte_launch::deployment::HEADER_LEN
        ..=charlotte_launch::deployment::MAX_DESCRIPTOR_LEN)
        .contains(&length)
    {
        return Err(());
    }
    let end = body_start.checked_add(length).ok_or(())?;
    if request.len() < end {
        Ok(None)
    } else {
        Ok(request.get(body_start..end))
    }
}

fn receive_descriptor(socket: &socket::OwnedSocket<'_>) -> Result<Vec<u8>, ()> {
    let mut request = Vec::new();
    loop {
        let chunk =
            socket.receive_timeout(RECEIVE_RETRIES, RECEIVE_RETRY_MS).map_err(|_| ())?.ok_or(())?;
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().map_err(|_| ())?;
        let bytes = mapping.as_slice().get(..len).ok_or(())?;
        if request.len().saturating_add(bytes.len())
            > HEADER_LIMIT + charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
        {
            return Err(());
        }
        request.extend_from_slice(bytes);
        if let Some(body) = complete_body(&request)? {
            return Ok(body.to_vec());
        }
    }
}

fn notify_cluster(controller: catten_rt::owned::ConnectionRef<'_>, descriptor: &[u8]) -> i64 {
    let Some(decoded) = charlotte_launch::deployment::decode(descriptor) else {
        return clusterctl::ERR_UNTRUSTED_DESCRIPTOR;
    };
    // The current replicated deployment operation retains the original
    // scalar short-name ABI. The signed format is already wider; extending
    // catalog placement to long names can therefore preserve this ingress.
    if decoded.artifact_name.is_empty() || decoded.artifact_name.len() > 8 {
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
    match controller.call_move(clusterctl::OP_NOTIFY, name(decoded.artifact_name), memory) {
        Ok(call) => call.wait().map_or(clusterctl::ERR_NOT_LEADER, |reply| reply.result),
        Err((_memory, _error)) => clusterctl::ERR_NOT_LEADER,
    }
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
        Ok(code) => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
        Err(()) => ("400 Bad Request", "{\"error\":\"malformed request\"}\n".into()),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body}",
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

        let result = receive_descriptor(&socket)
            .map(|descriptor| notify_cluster(controller.as_ref(), &descriptor));
        let reply = response(result);
        let _ = socket.send_all(reply.as_bytes(), 1000, 5);
        let _ = socket.close();
    }
}

catten_rt::entry!(main);
