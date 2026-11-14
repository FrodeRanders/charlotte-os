//APPBEGIN
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

use catten_rt::abi::{
    IPC_REPLY_CANCELLED,
    IPC_REPLY_ENDPOINT_CLOSED,
    ipc_status,
};
use charlotte_app::{
    grant_client,
    prelude::*,
};
use charlotte_launch::deployment::RIGHT_CALL;

// Protocol of the repository's deployed greeting test service.
const OP_GET: u32 = 1;
const GREET_VALUE: u64 = 0x0001_0102_464c_457f;
const READS: usize = 3;
const MAX_SESSIONS: usize = 3;
const JOB_BUDGET_MS: u64 = 5_000;

#[derive(Debug)]
pub enum Failure {
    LaunchContract(LaunchError),
    Discovery(grant_client::Error),
    Ipc(IpcError),
    Admission(AdmissionError),
    WaitAdmissionRejected(IpcError),
    OutcomeUnknown,
    EndpointLost, // A replacement is not implied.
    Rejected(i64),
    InvalidReply,
    DeadlineExpired,
    SessionLimit,
    Shutdown(ShutdownRequest),
}

#[cfg(not(test))]
fn main(ctx: &Context) -> Result<AppExit, Failure> {
    match run(ctx) {
        Ok(()) => Ok(AppExit::Completed),
        Err(Failure::Shutdown(request)) => Ok(AppExit::Shutdown(request)),
        Err(error) => Err(error),
    }
}

#[cfg(not(test))]
charlotte_app::entry!(main);
//APPEND

//DISCOVERYBEGIN
struct Work {
    completed: usize,
    deadline: Deadline,
}

pub fn run(ctx: &Context) -> Result<(), Failure> {
    // These are borrowed launch grants, not newly adopted capabilities.
    let app = Application::from_context(ctx).map_err(Failure::LaunchContract)?;
    let mut work = Work {
        completed: 0,
        deadline: Deadline::after(JOB_BUDGET_MS),
    };

    for _ in 0..MAX_SESSIONS {
        check_shutdown(ctx)?;
        let connection =
            app.acquire(b"greet", RIGHT_CALL, &work.deadline).map_err(discovery_error)?;

        // The session borrows this owner. Its pending call ends first.
        match session(ctx, &connection, &mut work) {
            Err(Failure::EndpointLost) => continue,
            result => return result,
        }
        // continue drops the old connection before the next acquisition.
    }
    Err(Failure::SessionLimit)
}

fn discovery_error(error: grant_client::Error) -> Failure {
    match error {
        grant_client::Error::Shutdown(request) => Failure::Shutdown(request),
        grant_client::Error::Admission(AdmissionError::Shutdown(request)) => {
            Failure::Shutdown(request)
        }
        error => Failure::Discovery(error),
    }
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
        let pending = Admission::with_lifecycle(&work.deadline, ctx.lifecycle())
            .call(connection.as_ref(), OP_GET, 0)
            .map_err(admission_error)?;
        let value = decode(observe(pending, &work.deadline)?)?;

        // Business use: a confirmed, read-only greeting observation.
        catten_rt::logln!("observation {}: {}", work.completed, value);
        work.completed += 1;
        // Only a new business operation receives a new deadline.
        work.deadline = Deadline::after(JOB_BUDGET_MS);
    }
    Ok(())
}

fn admission_error(error: AdmissionError) -> Failure {
    match error {
        AdmissionError::Rejected(IpcError::Status(status))
            if status == ipc_status::ENDPOINT_CLOSED =>
        {
            Failure::EndpointLost
        }
        AdmissionError::Shutdown(request) => Failure::Shutdown(request),
        AdmissionError::Deadline => Failure::DeadlineExpired,
        other => Failure::Admission(other),
    }
}

fn decode(reply: CallResult) -> Result<u64, Failure> {
    // Unexpected returned owners are disposed on this error path.
    if reply.connection.is_some() || reply.memory.is_some() {
        return Err(Failure::InvalidReply);
    }
    match reply.result {
        IPC_REPLY_ENDPOINT_CLOSED => Err(Failure::EndpointLost),
        IPC_REPLY_CANCELLED => Err(Failure::OutcomeUnknown),
        value if value < 0 => Err(Failure::Rejected(value)),
        value if value as u64 == GREET_VALUE => Ok(value as u64),
        _ => Err(Failure::InvalidReply),
    }
}
//SESSIONEND

//CALLBEGIN
fn observe(pending: PendingCall<'_>, deadline: &Deadline) -> Result<CallResult, Failure> {
    match pending.observe_until(deadline) {
        CallObservation::Reply(reply) => Ok(reply),
        CallObservation::Deadline(pending) => {
            cancel(pending);
            Err(Failure::OutcomeUnknown)
        }
        CallObservation::WaitRejected {
            pending,
            error,
        } => {
            // Wait admission failed; the accepted call still has an owner.
            cancel(pending);
            Err(Failure::WaitAdmissionRejected(error))
        }
        CallObservation::Failed(error) => Err(Failure::Ipc(error)),
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
