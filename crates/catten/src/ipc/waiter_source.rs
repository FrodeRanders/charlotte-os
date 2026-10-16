//! Exact source capture and allocation-free waiter publication under IPC.

use super::*;
use crate::klib::observer::PreparedWaitRegistration;

pub(super) enum Source {
    Endpoint(EndpointId),
    Reply(PendingCallId),
}

impl Source {
    fn state<'a>(&self, ipc: &'a IpcRegistry) -> Result<(&'a WaiterList, bool), RegistrationError> {
        match self {
            Self::Endpoint(id) => {
                let endpoint = ipc.endpoints.get(id).ok_or(RegistrationError::Closed)?;
                Ok((&endpoint.readiness_observers, endpoint_available(ipc, endpoint)))
            }
            Self::Reply(id) => {
                let call = ipc.pending_calls.get(id).ok_or(RegistrationError::Closed)?;
                Ok((&call.observers, call.result.is_some()))
            }
        }
    }

    fn capture(&self) -> Result<(WaiterList, bool), RegistrationError> {
        let ipc = IPC.read();
        let (list, ready) = self.state(&ipc)?;
        Ok((list.clone(), ready))
    }

    pub(super) fn register(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        let (source, ready) = self.capture()?;
        if ready {
            return Ok(WaitRegistration::ready());
        }
        // Entry owns the captured source, observer and original charge through
        // revalidation. No allocation or losing-storage destruction under IPC.
        let mut prepared = sponsor.prepare(&source, observer)?;
        drop(source);
        let result = self.publish(&mut prepared);
        prepared.finish();
        result
    }

    fn publish(
        &self,
        prepared: &mut PreparedWaitRegistration,
    ) -> Result<WaitRegistration, RegistrationError> {
        let ipc = IPC.read();
        let (source, ready) = self.state(&ipc)?;
        if !prepared.matches(source) {
            return Err(RegistrationError::Closed);
        }
        if ready {
            Ok(WaitRegistration::ready())
        } else {
            prepared.publish()
        }
    }
}

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
