//! Scalar protocol boundary shared by native clients and host-side scripts.
use catten_rt::{
    call::{
        Admission,
        AdmissionError,
        ObservationError,
    },
    deadline::Deadline,
    owned::ConnectionRef,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Admission(AdmissionError),
    Observation(ObservationError),
    InvalidReply,
    EndpointClosed,
    Cancelled,
    Service(i64),
    UnexpectedCall,
}

/// Business logic can depend on its service operation, not a native syscall
/// backend. Mutability also permits an ordered host script without globals.
pub trait ScalarService {
    fn call(&mut self, opcode: u32, argument: u64) -> Result<i64, Error>;
}

pub struct ScalarClient<'connection, 'deadline> {
    connection: ConnectionRef<'connection>,
    deadline: &'deadline Deadline,
}
impl<'connection, 'deadline> ScalarClient<'connection, 'deadline> {
    pub const fn new(
        connection: ConnectionRef<'connection>,
        deadline: &'deadline Deadline,
    ) -> Self {
        Self {
            connection,
            deadline,
        }
    }
}
impl ScalarService for ScalarClient<'_, '_> {
    fn call(&mut self, opcode: u32, argument: u64) -> Result<i64, Error> {
        let pending = Admission::new(self.deadline)
            .call(self.connection, opcode, argument)
            .map_err(Error::Admission)?;
        let reply =
            pending.observe_until(self.deadline).cancel_on_failure().map_err(Error::Observation)?;
        if reply.connection.is_some() || reply.memory.is_some() {
            return Err(Error::InvalidReply);
        }
        decode(reply.result)
    }
}

fn decode(result: i64) -> Result<i64, Error> {
    match result {
        catten_rt::abi::IPC_REPLY_ENDPOINT_CLOSED => Err(Error::EndpointClosed),
        catten_rt::abi::IPC_REPLY_CANCELLED => Err(Error::Cancelled),
        value if value < 0 => Err(Error::Service(value)),
        value => Ok(value),
    }
}

/// Host scripts never fabricate kernel capabilities or execute the ABI backend.
#[cfg(feature = "test-support")]
pub mod testing {
    use alloc::{
        collections::VecDeque,
        vec::Vec,
    };

    use super::{
        Error,
        ScalarService,
    };
    pub struct Step {
        pub opcode: u32,
        pub argument: u64,
        pub result: Result<i64, Error>,
    }
    pub struct ScriptedScalar {
        steps: VecDeque<Step>,
    }
    impl ScriptedScalar {
        pub fn new(steps: Vec<Step>) -> Self {
            Self {
                steps: steps.into(),
            }
        }

        pub fn is_complete(&self) -> bool {
            self.steps.is_empty()
        }
    }
    impl ScalarService for ScriptedScalar {
        fn call(&mut self, opcode: u32, argument: u64) -> Result<i64, Error> {
            let Some(step) = self.steps.front() else {
                return Err(Error::UnexpectedCall);
            };
            if step.opcode != opcode || step.argument != argument {
                return Err(Error::UnexpectedCall);
            }
            self.steps.pop_front().expect("matched script").result.and_then(super::decode)
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use alloc::vec;

    use super::{
        testing::*,
        *,
    };
    #[test]
    fn host_script_checks_request_and_preserves_uncertain_outcome_without_retry() {
        let mut service = ScriptedScalar::new(vec![Step {
            opcode: 1,
            argument: 9,
            result: Ok(catten_rt::abi::IPC_REPLY_CANCELLED),
        }]);
        assert_eq!(service.call(1, 8), Err(Error::UnexpectedCall));
        assert!(!service.is_complete());
        assert_eq!(service.call(1, 9), Err(Error::Cancelled));
        assert!(service.is_complete());
        assert_eq!(service.call(1, 9), Err(Error::UnexpectedCall));
    }
}
