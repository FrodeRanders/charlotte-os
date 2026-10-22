//! Bounded off-cluster deployment-notification ingress.
//!
//! The listener accepts `POST /v1/deployments` with one signed `CDEPLOY6`
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

use catten_rt::{
    Context,
    ShutdownRequest,
};
use catten_services::{
    clusterctl,
    deployment_http::Error as RequestError,
    socket,
    wait_for_local_ready_or_shutdown,
    wait_for_registered_name_or_shutdown_owned,
};
use catten_syscall::thread_exit;

const ACCEPT_RETRY_MS: u64 = 50;

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
    let cq = ctx.completion_queue_layout();
    let mut inputs = catten_services::deployment_input::Inputs::new(0);
    let mut controls = catten_services::deployment_control::Controls::new(controller.as_ref(), 0);
    let mut outputs = catten_services::deployment_output::Outputs::new(0);
    let mut closes = catten_services::deployment_close::Closes::new(0);
    let mut shutdown = None;
    let mut accepting: [Option<Accepting<'_>>; catten_services::deployment_input::MAX_CLIENTS] =
        core::array::from_fn(|_| None);
    let mut retry: Option<catten_services::deadline::Deadline> = None;
    let mut announced = false;
    loop {
        // CQ entries are hints. Drain before checking the exact pending owners;
        // a completion arriving after this drain keeps the following wait ready.
        for _ in 0..cq.entries {
            if unsafe { catten_syscall::cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
        if shutdown.is_none() {
            shutdown = ctx.lifecycle().shutdown_requested();
        }
        if shutdown.is_some() {
            // Drain existing logical-close owners under their original budgets
            // before leaving this scope. Other phases remain owned throughout;
            // their existing shutdown destructors are a separate boundary.
            if closes.occupied_slots() == 0 {
                return shutdown.take().unwrap();
            }
            if !closes.advance() {
                catten_services::reactor::wait(closes.remaining_ms(), 0);
            }
            continue;
        }
        if retry.is_some_and(|deadline| deadline.expired()) {
            retry = None;
        }
        replenish(
            tcp.as_ref(),
            &mut accepting,
            inputs.available_slots().saturating_sub(
                controls.occupied_slots() + outputs.occupied_slots() + closes.occupied_slots(),
            ),
            &mut retry,
            trace,
        );
        if !announced && accepting.iter().any(Option::is_some) {
            catten_rt::logln!("[deployd] listening port={}", clusterctl::NOTIFY_PORT);
            announced = true;
        }
        let mut accepted = false;
        for slot in &mut accepting {
            let Some(owner) = slot.as_mut() else {
                continue;
            };
            match owner.pending.poll() {
                Ok(None) => {}
                Ok(Some(reply))
                    if reply.result == 0
                        && reply.memory.is_none()
                        && reply.connection.is_none() =>
                {
                    let Accepting {
                        pending,
                        socket,
                    } = slot.take().unwrap();
                    drop(pending);
                    if let Err((socket, _)) = inputs.admit(socket) {
                        closes.start(socket);
                    }
                    accepted = true;
                }
                _ => {
                    let Accepting {
                        pending,
                        socket,
                    } = slot.take().unwrap();
                    drop(pending);
                    closes.start(socket);
                    retry = Some(catten_services::deadline::Deadline::after(ACCEPT_RETRY_MS));
                }
            }
        }
        let mut progressed = accepted;
        if let Some((socket, reply)) = controls.advance() {
            progressed = true;
            if let Some((socket, result)) = outputs.start(socket, reply) {
                retire_output(socket, result, &mut closes, trace);
            }
        }
        let (input_progress, finished) = inputs.advance_reserved(controls.reserved_bytes());
        progressed |= input_progress;
        if let Some((socket, request)) = finished {
            let completed = match request {
                Ok(request) => {
                    let available =
                        inputs.available_bytes().saturating_sub(controls.reserved_bytes());
                    controls.start(socket, request, available)
                }
                Err(error) => Some((
                    socket,
                    alloc::borrow::Cow::Borrowed(match error {
                        RequestError::AllocationFailed => ALLOCATION_FAILURE_RESPONSE,
                        _ => BAD_REQUEST_RESPONSE,
                    }),
                )),
            };
            if let Some((socket, reply)) = completed
                && let Some((socket, result)) = outputs.start(socket, reply)
            {
                retire_output(socket, result, &mut closes, trace);
            }
        }
        let (output_progress, finished) = outputs.advance();
        progressed |= output_progress;
        if let Some((socket, result)) = finished {
            retire_output(socket, result, &mut closes, trace);
        }
        progressed |= closes.advance();
        replenish(
            tcp.as_ref(),
            &mut accepting,
            inputs.available_slots().saturating_sub(
                controls.occupied_slots() + outputs.occupied_slots() + closes.occupied_slots(),
            ),
            &mut retry,
            trace,
        );
        if progressed {
            continue;
        }
        let timeout = [
            inputs.remaining_ms(),
            controls.remaining_ms(),
            outputs.remaining_ms(),
            closes.remaining_ms(),
            retry.map(|d| d.remaining_ms()),
        ]
        .into_iter()
        .flatten()
        .min();
        catten_services::reactor::wait(timeout, 0);
    }
}

fn retire_output<'a>(
    socket: socket::OwnedSocket<'a>,
    result: Result<(), catten_services::deployment_output::Error>,
    closes: &mut catten_services::deployment_close::Closes<'a>,
    trace: bool,
) {
    if trace && let Err(error) = result {
        catten_rt::logln!("[deployd] response ended: {error:?}");
    }
    // A send receipt confirms enqueue only. The next owning phase observes one
    // logical close without blocking other clients or resending accepted work.
    closes.start(socket);
}

fn replenish<'a>(
    tcp: catten_rt::owned::ConnectionRef<'a>,
    accepting: &mut [Option<Accepting<'a>>; catten_services::deployment_input::MAX_CLIENTS],
    available: usize,
    retry: &mut Option<catten_services::deadline::Deadline>,
    trace: bool,
) {
    if retry.is_some() {
        return;
    }
    let mut count = accepting.iter().flatten().count();
    for slot in accepting.iter_mut().filter(|slot| slot.is_none()) {
        if count >= available {
            break;
        }
        match begin_accept(tcp) {
            Ok(owner) => {
                *slot = Some(owner);
                count += 1;
            }
            Err(error) => {
                if trace {
                    catten_rt::logln!("[deployd] listener admission rejected: {error:?}");
                }
                // Rejected setup has no readiness producer. Accepted listener
                // and receive operations retain their own CQ notifications.
                *retry = Some(catten_services::deadline::Deadline::after(ACCEPT_RETRY_MS));
                break;
            }
        }
    }
}

struct Accepting<'a> {
    pending: catten_rt::owned::PendingCall<'static>,
    socket: socket::OwnedSocket<'a>,
}

fn begin_accept(
    tcp: catten_rt::owned::ConnectionRef<'_>,
) -> Result<Accepting<'_>, socket::SocketError> {
    let socket = new_listener(tcp)?;
    let pending = socket.accept_call().map_err(socket::SocketError::Ipc)?;
    let mut owner = Accepting {
        pending,
        socket,
    };
    if owner.pending.bind_cq(0).is_err() {
        // Accepted work and its socket stay together through terminal abort.
        catten_rt::domain_abort();
    }
    Ok(owner)
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
