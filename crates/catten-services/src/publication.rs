//! Endpoint publication with bounded retries before call acceptance.

use catten_rt::owned::{
    ConnectionRef,
    Endpoint,
    IpcError,
};

use crate::{
    deadline::Deadline,
    ns,
    sleep_ms,
};

pub const RETRY_BUDGET_MS: u64 = 1_000;
const RETRY_PAUSE_MS: u64 = 10;

#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    Admission(IpcError),
    Observation(IpcError),
    Rejected(i64),
    InvalidReply,
}

fn submit<T>(
    mut attempt: impl FnMut() -> Result<T, IpcError>,
    mut expired: impl FnMut() -> bool,
    mut pause: impl FnMut(),
) -> Result<T, IpcError> {
    loop {
        match attempt() {
            Ok(call) => return Ok(call),
            Err(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL)) if !expired() => pause(),
            Err(error) => return Err(error),
        }
    }
}

/// Borrow the same endpoint throughout admission and observation. Only an
/// exact queue-full rejection retries; an accepted call is observed once.
pub fn register(names: ConnectionRef<'_>, name: u64, endpoint: &Endpoint) -> Result<i64, Error> {
    let deadline = Deadline::after(RETRY_BUDGET_MS);
    let mut announced = false;
    let call = submit(
        || {
            names.call_connection(
                ns::OP_REGISTER,
                name,
                endpoint,
                catten_syscall::IpcRights::SEND
                    | catten_syscall::IpcRights::CALL
                    | catten_syscall::IpcRights::MINT_CONNECTION,
            )
        },
        || deadline.expired(),
        || {
            if !announced {
                catten_rt::logln!(
                    "[publication] queue-full name={name:#x} endpoint={} retaining source",
                    endpoint.as_raw()
                );
                announced = true;
            }
            sleep_ms(RETRY_PAUSE_MS);
        },
    )
    .map_err(Error::Admission)?;
    let reply = call.wait().map_err(Error::Observation)?;
    if reply.connection.is_some() || reply.memory.is_some() {
        return Err(Error::InvalidReply);
    }
    if reply.result < 1 {
        return Err(Error::Rejected(reply.result));
    }
    Ok(reply.result)
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    #[test]
    fn queue_full_retries_until_one_acceptance() {
        let attempts = Cell::new(0);
        let pauses = Cell::new(0);
        let result = submit(
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 3 {
                    Err(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL))
                } else {
                    Ok(7)
                }
            },
            || false,
            || pauses.set(pauses.get() + 1),
        );
        assert_eq!(result, Ok(7));
        assert_eq!((attempts.get(), pauses.get()), (3, 2));
    }

    #[test]
    fn expired_and_permanent_rejections_are_not_replayed() {
        for (error, expired) in [
            (IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL), true),
            (IpcError::Status(catten_syscall::ipc_status::RESOURCE_LIMIT), false),
            (IpcError::CreationFailed, false),
        ] {
            let attempts = Cell::new(0);
            assert_eq!(
                submit::<()>(
                    || {
                        attempts.set(attempts.get() + 1);
                        Err(error)
                    },
                    || expired,
                    || panic!("must not pause")
                ),
                Err(error)
            );
            assert_eq!(attempts.get(), 1);
        }
    }
}
