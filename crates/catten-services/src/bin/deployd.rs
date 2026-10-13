//! Bounded off-cluster deployment-notification ingress.
//!
//! The listener accepts `POST /v1/deployments` with one signed `CDEPLOY5`
//! descriptor, `POST /v1/releases` with one signed `CRELEASE` component set,
//! `POST /v1/operations` with one encrypted `COPSBND2` admission proof,
//! `POST /v1/shutdowns` with one signed node-targeted shutdown intent,
//! `POST /v1/ingress-policy` with one operations-signed assignment replacement,
//! and `GET /v1/deployments/{percent-encoded-name}` for rollout observation.
//! The ingress carries no object-store or application secret: authenticity,
//! integrity, placement, and authority come from signatures checked by
//! `clusterctl`. Assigned nodes fetch referenced ELFs through their separately
//! provisioned S3 service.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    borrow::Cow,
    format,
};

use catten_rt::{
    Context,
    ShutdownRequest,
    owned::OwnedMemory,
};
use catten_services::{
    clusterctl,
    clusterctl_client,
    deployment_http::{
        Error as RequestError,
        Request,
        RequestBuffer,
    },
    name_catalog,
    sleep_ms,
    socket,
    wait_for_local_ready_or_shutdown,
    wait_for_registered_name_or_shutdown_owned,
};
use catten_syscall::thread_exit;

const ACCEPT_RETRY_MS: u64 = 50;
const RECEIVE_RETRY_MS: u64 = 100;

const BAD_REQUEST_RESPONSE: &str = concat!(
    "HTTP/1.1 400 Bad Request\r\n",
    "Content-Type: application/json\r\nContent-Length: 30\r\nConnection: close\r\n\r\n",
    "{\"error\":\"malformed request\"}\n",
);

const ALLOCATION_FAILURE_RESPONSE: &str = concat!(
    "HTTP/1.1 503 Service Unavailable\r\n",
    "Content-Type: application/json\r\nContent-Length: 30\r\nConnection: close\r\n\r\n",
    "{\"error\":\"allocation failed\"}\n",
);

fn fail() -> ! {
    unsafe { thread_exit() }
}

fn receive_request(
    ctx: &Context,
    socket: &socket::OwnedSocket<'_>,
) -> Result<Result<Request, ShutdownRequest>, RequestError> {
    let mut request = RequestBuffer::default();
    let deadline = catten_services::deadline::Deadline::after(30_000);
    let header_deadline = catten_services::deadline::Deadline::after(5_000);
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return Ok(Err(request));
        }
        if deadline.expired() || (!request.has_header() && header_deadline.expired()) {
            return Err(RequestError::Malformed);
        }
        let chunk = match socket.receive_timeout(1, RECEIVE_RETRY_MS) {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return Err(RequestError::Malformed),
            Err(socket::SocketError::RetryExhausted) => continue,
            Err(_) => return Err(RequestError::Malformed),
        };
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().map_err(|_| RequestError::AllocationFailed)?;
        let bytes = mapping.as_slice().get(..len).ok_or(RequestError::Malformed)?;
        if let Some(request) = request.push(bytes)? {
            return Ok(Ok(request));
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
    clusterctl_client::call_move(controller, clusterctl::OP_NOTIFY, 0, memory)
        .map_or_else(|error| error.code(), |reply| reply.result)
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
    clusterctl_client::call_move(controller, clusterctl::OP_NOTIFY_RELEASE, 0, memory)
        .map_or_else(|error| error.code(), |reply| reply.result)
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
    clusterctl_client::call_move(controller, clusterctl::OP_NOTIFY_OPERATIONS, 0, memory)
        .map_or_else(|error| error.code(), |reply| reply.result)
}

fn notify_shutdown(controller: catten_rt::owned::ConnectionRef<'_>, envelope: &[u8]) -> i64 {
    if charlotte_launch::shutdown::decode(envelope).is_none() {
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
    clusterctl_client::call_move(controller, clusterctl::OP_NOTIFY_SHUTDOWN, 0, memory)
        .map_or_else(|error| error.code(), |reply| reply.result)
}

fn notify_ingress_policy(controller: catten_rt::owned::ConnectionRef<'_>, envelope: &[u8]) -> i64 {
    if charlotte_launch::ingress_policy::decode(envelope).is_none() {
        return clusterctl::ERR_UNTRUSTED_DESCRIPTOR;
    }
    let bytes = match envelope.len().checked_add(8) {
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
    mapping.as_mut_slice()[..8].copy_from_slice(&(envelope.len() as u64).to_le_bytes());
    mapping.as_mut_slice()[8..8 + envelope.len()].copy_from_slice(envelope);
    let memory = match mapping.unmap() {
        Ok(memory) => memory,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    clusterctl_client::call_move(controller, clusterctl::OP_NOTIFY_INGRESS_POLICY, 0, memory)
        .map_or_else(|error| error.code(), |reply| reply.result)
}

fn query_rollout(
    controller: catten_rt::owned::ConnectionRef<'_>,
    name: &[u8],
) -> Result<clusterctl::RolloutStatus, i64> {
    let memory = OwnedMemory::allocate(1).map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let mut mapping = memory.map_writable().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    mapping.as_mut_slice()[..name.len()].copy_from_slice(name);
    let memory = mapping.unmap().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let reply =
        clusterctl_client::call_move(controller, clusterctl::OP_ROLLOUT, name.len() as u64, memory)
            .map_err(|error| error.code())?;
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

fn query_ingress_policy(
    controller: catten_rt::owned::ConnectionRef<'_>,
) -> Result<alloc::string::String, i64> {
    let reply = clusterctl_client::call(controller, clusterctl::OP_INGRESS_POLICY_STATUS, 0)
        .map_err(|error| error.code())?;
    if reply.result < 0 {
        return Err(reply.result);
    }
    let len = usize::try_from(reply.result).map_err(|_| clusterctl::ERR_NOT_FOUND)?;
    let memory = reply.memory.ok_or(clusterctl::ERR_NOT_FOUND)?;
    let mapping = memory.map_read_only().map_err(|_| clusterctl::ERR_NOT_FOUND)?;
    let entry = name_catalog::decode_ingress_policy_result(
        mapping.as_slice().get(..len).ok_or(clusterctl::ERR_NOT_FOUND)?,
    )
    .ok_or(clusterctl::ERR_NOT_FOUND)?;
    let policy = charlotte_launch::ingress_policy::decode(&entry.envelope)
        .ok_or(clusterctl::ERR_NOT_FOUND)?;
    Ok(format!(
        "{{\"generation\":{},\"sequence\":{},\"assignments\":{},\"not_before_unix_seconds\":{},\"\
         expires_unix_seconds\":{}}}\n",
        entry.generation,
        policy.sequence,
        policy.assignments().count(),
        policy.not_before_unix_seconds,
        policy.expires_unix_seconds
    ))
}

fn response(result: i64) -> alloc::string::String {
    let (status, body) = match result {
        generation if generation > 0 => {
            ("202 Accepted", format!("{{\"generation\":{generation}}}\n"))
        }
        clusterctl::ERR_UNTRUSTED_DESCRIPTOR => {
            ("403 Forbidden", format!("{{\"error\":{}}}\n", clusterctl::ERR_UNTRUSTED_DESCRIPTOR))
        }
        clusterctl::ERR_STALE_DESCRIPTOR | clusterctl::ERR_CONFLICTING_DESCRIPTOR => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", result))
        }
        clusterctl::ERR_EXPIRED_OPERATION => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", clusterctl::ERR_EXPIRED_OPERATION))
        }
        clusterctl::ERR_OUTSIDE_VALIDITY => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", clusterctl::ERR_OUTSIDE_VALIDITY))
        }
        clusterctl::ERR_UNSATISFIABLE_PLACEMENT | clusterctl::ERR_INCOMPATIBLE_TARGET => {
            ("409 Conflict", format!("{{\"error\":{result}}}\n"))
        }
        code => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body}",
        body.len()
    )
}

fn status_response(result: Result<alloc::string::String, i64>) -> alloc::string::String {
    let (status, body) = match result {
        Ok(body) => ("200 OK", body),
        Err(clusterctl::ERR_NOT_FOUND) => {
            ("404 Not Found", "{\"error\":\"ingress policy not found\"}\n".into())
        }
        Err(code) => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{}",
        body.len(),
        body
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
                     {},\"node_key\":{},\"desired_replicas\":{},\"ready_replicas\":{}}}\n",
                    status.deployment_generation,
                    status.service_generation,
                    status.node_key,
                    status.desired_replicas,
                    status.ready_replicas
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

fn new_listener(
    tcp: catten_rt::owned::ConnectionRef<'_>,
) -> Result<socket::OwnedSocket<'_>, socket::SocketError> {
    let socket =
        socket::OwnedSocket::open(tcp, socket::DOMAIN_TCP).map_err(socket::SocketError::Ipc)?;
    socket.listen(clusterctl::NOTIFY_PORT)?;
    Ok(socket)
}

fn serve(ctx: &Context) -> ShutdownRequest {
    let names = ctx.bootstrap_connection().unwrap_or_else(|| fail());
    let (_, tcp) = match wait_for_registered_name_or_shutdown_owned(ctx, names, socket::NAME) {
        Ok(found) => found,
        Err(request) => return request,
    };
    let (_, controller) =
        match wait_for_registered_name_or_shutdown_owned(ctx, names, clusterctl::NAME) {
            Ok(found) => found,
            Err(request) => return request,
        };
    if let Err(request) = wait_for_local_ready_or_shutdown(ctx, names) {
        return request;
    }

    let trace = matches!(
        ctx.manifest_value(charlotte_launch::deployd_status::TRACE_KEY),
        Some(catten_rt::ManifestValue::Unsigned(1))
    );
    let mut announced = false;
    'connections: loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return request;
        }
        let socket = match new_listener(tcp.as_ref()) {
            Ok(socket) => socket,
            Err(error) => {
                if trace {
                    catten_rt::logln!("[deployd] listener rejected: {error:?}");
                }
                sleep_ms(ACCEPT_RETRY_MS);
                continue;
            }
        };
        if !announced {
            catten_rt::logln!("[deployd] listening port={}", clusterctl::NOTIFY_PORT);
            announced = true;
        }
        loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                return request;
            }
            let call = match socket.call(socket::OP_ACCEPT, socket.id()) {
                Ok(call) => call,
                Err(error) => {
                    if trace {
                        catten_rt::logln!(
                            "[deployd] accept admission={error:?} socket={}",
                            socket.id()
                        );
                    }
                    if error
                        == catten_rt::owned::IpcError::Status(
                            catten_syscall::ipc_status::QUEUE_FULL,
                        )
                    {
                        sleep_ms(ACCEPT_RETRY_MS);
                        continue;
                    }
                    continue 'connections;
                }
            };
            let accepted = match call.wait() {
                Ok(reply) => reply.result,
                Err(_) => continue 'connections,
            };
            if accepted == 0 {
                break;
            }
            if accepted != socket::ERR_WOULD_BLOCK {
                continue 'connections;
            }
            sleep_ms(ACCEPT_RETRY_MS);
        }

        let reply: Cow<'static, str> = match receive_request(ctx, &socket) {
            Ok(Err(request)) => return request,
            Ok(Ok(Request::Notify(descriptor))) => {
                Cow::Owned(response(notify_cluster(controller.as_ref(), &descriptor)))
            }
            Ok(Ok(Request::Release(envelope))) => {
                Cow::Owned(response(notify_release(controller.as_ref(), &envelope)))
            }
            Ok(Ok(Request::Operations(bundle))) => {
                Cow::Owned(response(notify_operations(controller.as_ref(), &bundle)))
            }
            Ok(Ok(Request::Shutdown(envelope))) => {
                Cow::Owned(response(notify_shutdown(controller.as_ref(), &envelope)))
            }
            Ok(Ok(Request::IngressPolicy(envelope))) => {
                Cow::Owned(response(notify_ingress_policy(controller.as_ref(), &envelope)))
            }
            Ok(Ok(Request::IngressPolicyStatus)) => {
                Cow::Owned(status_response(query_ingress_policy(controller.as_ref())))
            }
            Ok(Ok(Request::Status(name))) => {
                Cow::Owned(rollout_response(query_rollout(controller.as_ref(), &name)))
            }
            Err(RequestError::AllocationFailed) => Cow::Borrowed(ALLOCATION_FAILURE_RESPONSE),
            Err(_) => Cow::Borrowed(BAD_REQUEST_RESPONSE),
        };
        let _ = socket.send_all(reply.as_bytes(), 1000, 5);
        let _ = socket.close();
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
