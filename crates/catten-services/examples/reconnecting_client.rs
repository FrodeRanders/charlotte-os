//APPBEGIN
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

use catten_rt::{
    Context,
    ShutdownRequest,
    owned::{
        CallResult,
        Connection,
        IpcError,
        PendingCall,
    },
};
use catten_services::{
    deadline::Deadline,
    deploy,
    grant_client,
};
use catten_syscall::{
    IPC_REPLY_CANCELLED,
    IPC_REPLY_ENDPOINT_CLOSED,
    ipc_status,
};
use charlotte_launch::deployment::RIGHT_CALL;

const READS: usize = 3;
const MAX_SESSIONS: usize = 3;
const JOB_BUDGET_MS: u64 = 5_000;

#[derive(Debug)]
pub enum Failure {
    LaunchContract,
    Discovery(grant_client::Error),
    Ipc(IpcError),
    WaitAdmissionRejected,
    OutcomeUnknown,
    ServiceRestarted, // Application policy for a closed endpoint.
    Rejected(i64),
    InvalidReply,
    RecoveryLimit,
    Shutdown(ShutdownRequest),
}

#[cfg(not(test))]
fn main(ctx: Context) -> ! {
    // run() releases its owned resources before any terminal action here.
    match run(&ctx) {
        Ok(()) => unsafe { catten_syscall::thread_exit() },
        Err(Failure::Shutdown(request)) => request.complete(),
        Err(error) => {
            catten_rt::logln!("client stopped: {:?}", error);
            catten_rt::domain_abort()
        }
    }
}

#[cfg(not(test))]
catten_rt::entry!(main);
//APPEND

//DISCOVERYBEGIN
struct Work {
    completed: usize,
    deadline: Deadline,
}

pub fn run(ctx: &Context) -> Result<(), Failure> {
    // These are borrowed launch grants, not newly adopted capabilities.
    let controller = ctx.bootstrap_connection().ok_or(Failure::LaunchContract)?;
    let descriptor = ctx.profile_memory().ok_or(Failure::LaunchContract)?;
    let mut work = Work {
        completed: 0,
        deadline: Deadline::after(JOB_BUDGET_MS),
    };

    for _ in 0..MAX_SESSIONS {
        check_shutdown(ctx)?;
        let connection = grant_client::acquire_until(
            controller,
            &descriptor,
            b"greet",
            RIGHT_CALL,
            &work.deadline,
        )
        .map_err(Failure::Discovery)?;

        // The session borrows this owner. Its pending call ends first.
        match session(ctx, &connection, &mut work) {
            Err(Failure::ServiceRestarted) => continue,
            result => return result,
        }
        // continue drops the old connection before the next acquisition.
    }
    Err(Failure::RecoveryLimit)
}

fn check_shutdown(ctx: &Context) -> Result<(), Failure> {
    match ctx.lifecycle().shutdown_requested() {
        Some(request) => Err(Failure::Shutdown(request)),
        None => Ok(()),
    }
}
//DISCOVERYEND

//SESSIONBEGIN
fn session(ctx: &Context, connection: &Connection, work: &mut Work) -> Result<(), Failure> {
    while work.completed < READS {
        check_shutdown(ctx)?;
        if work.deadline.expired() {
            return Err(Failure::RecoveryLimit);
        }
        let pending = connection.call(deploy::OP_GET, 0).map_err(admission_error)?;
        let value = decode(observe(pending, &work.deadline)?)?;

        // Business use: a confirmed, read-only greeting observation.
        catten_rt::logln!("observation {}: {}", work.completed, value);
        work.completed += 1;
        // Only a new business operation receives a new deadline.
        work.deadline = Deadline::after(JOB_BUDGET_MS);
    }
    Ok(())
}

fn admission_error(error: IpcError) -> Failure {
    match error {
        IpcError::Status(ipc_status::ENDPOINT_CLOSED) => Failure::ServiceRestarted,
        other => Failure::Ipc(other),
    }
}

fn decode(reply: CallResult) -> Result<u64, Failure> {
    // All unexpected returned owners are still disposed on this error path.
    if reply.connection.is_some() || reply.memory.is_some() {
        return Err(Failure::InvalidReply);
    }
    match reply.result {
        IPC_REPLY_ENDPOINT_CLOSED => Err(Failure::ServiceRestarted),
        IPC_REPLY_CANCELLED => Err(Failure::OutcomeUnknown),
        value if value < 0 => Err(Failure::Rejected(value)),
        value if value as u64 == deploy::GREET_VALUE => Ok(value as u64),
        _ => Err(Failure::InvalidReply),
    }
}
//SESSIONEND

//CALLBEGIN
fn observe(mut pending: PendingCall<'_>, deadline: &Deadline) -> Result<CallResult, Failure> {
    match pending.wait_timeout(deadline.remaining_ms()) {
        Ok(Some(reply)) => Ok(reply),
        Ok(None) => {
            cancel(pending);
            Err(Failure::OutcomeUnknown)
        }
        Err(IpcError::Status(ipc_status::RESOURCE_LIMIT)) => {
            // Wait admission failed; the accepted call still has an owner.
            cancel(pending);
            Err(Failure::WaitAdmissionRejected)
        }
        Err(error) => Err(Failure::Ipc(error)),
    }
}

fn cancel(pending: PendingCall<'_>) {
    if let Err((retained, error)) = pending.close() {
        // Rejected cleanup returns the owner, including any Rust memory loan.
        // Keep it and the enclosing session live through terminal abort.
        let _retained = retained;
        catten_rt::logln!("call cleanup rejected: {:?}", error);
        catten_rt::domain_abort();
    }
}
//CALLEND

#[cfg(test)]
#[path = "reconnecting_client/tests.rs"]
mod tests;
