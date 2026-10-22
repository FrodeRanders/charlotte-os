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
    let mut listeners = catten_services::deployment_listener::Listeners::new(tcp.as_ref(), 0);
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
            // Finish accepted setup under its original budget. Cancel idle
            // accepts into the same close pool while all other phases stay live.
            let (listener_progress, finished) = listeners.advance(true);
            if let Some((socket, _)) = finished {
                closes.start(socket);
            }
            let close_progress = closes.advance();
            if listeners.occupied_slots() == 0 && closes.occupied_slots() == 0 {
                return shutdown.take().unwrap();
            }
            if !listener_progress && !close_progress {
                catten_services::reactor::wait(
                    [listeners.remaining_ms(false), closes.remaining_ms()]
                        .into_iter()
                        .flatten()
                        .min(),
                    0,
                );
            }
            continue;
        }
        let available = inputs.available_slots().saturating_sub(
            controls.occupied_slots() + outputs.occupied_slots() + closes.occupied_slots(),
        );
        listeners.replenish(available);
        let (mut progressed, finished) = listeners.advance(false);
        if !announced && listeners.accepting() {
            catten_rt::logln!("[deployd] listening port={}", clusterctl::NOTIFY_PORT);
            announced = true;
        }
        if let Some((socket, accepted)) = finished {
            if accepted {
                if let Err((socket, _)) = inputs.admit(socket) {
                    closes.start(socket);
                }
            } else {
                closes.start(socket);
            }
        }
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
        let available = inputs.available_slots().saturating_sub(
            controls.occupied_slots() + outputs.occupied_slots() + closes.occupied_slots(),
        );
        listeners.replenish(available);
        if progressed {
            continue;
        }
        let timeout = [
            inputs.remaining_ms(),
            controls.remaining_ms(),
            outputs.remaining_ms(),
            closes.remaining_ms(),
            listeners.remaining_ms(available > listeners.occupied_slots()),
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

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
