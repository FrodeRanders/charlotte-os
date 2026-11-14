//! Application launch, typed service clients and resource-owning serving scopes.
#![no_std]
extern crate alloc;

pub use catten_rt::{
    Context,
    ShutdownRequest,
    deadline,
    logln,
    owned,
};
pub use charlotte_protocol_grant as grant;
pub mod client;
pub mod grant_client;
#[cfg(feature = "kafka")]
pub mod kafka_client;
#[cfg(feature = "s3")]
pub mod s3_client;
pub mod service;
#[cfg(feature = "socket")]
pub mod socket;
#[cfg(feature = "socket")]
mod socket_close;
#[cfg(feature = "socket")]
pub mod socket_send;

pub const fn name(bytes: &[u8]) -> u64 {
    catten_rt::manifest_key(bytes)
}

fn sleep_ms(milliseconds: u64) {
    let result = owned::Completion::timer(milliseconds).and_then(owned::Completion::wait);
    if result != Ok(0) {
        catten_rt::logln!("[app] backoff failed: {:?}", result);
        catten_rt::domain_abort();
    }
}

/// Borrowed launch authority; the grant controller still enforces every grant.
pub struct Application<'ctx> {
    context: &'ctx Context,
    controller: owned::ConnectionRef<'ctx>,
    descriptor: owned::LaunchMemoryRef<'ctx>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchError {
    MissingController,
    MissingDescriptor,
}

impl<'ctx> Application<'ctx> {
    pub fn from_context(context: &'ctx Context) -> Result<Self, LaunchError> {
        Ok(Self {
            context,
            controller: context.bootstrap_connection().ok_or(LaunchError::MissingController)?,
            descriptor: context.profile_memory().ok_or(LaunchError::MissingDescriptor)?,
        })
    }

    pub fn context(&self) -> &'ctx Context {
        self.context
    }

    pub fn acquire(
        &self,
        service: &[u8],
        rights: u16,
        deadline: &deadline::Deadline,
    ) -> Result<owned::Connection, grant_client::Error> {
        grant_client::acquire_with_context(
            self.context,
            self.controller,
            &self.descriptor,
            service,
            rights,
            deadline,
        )
    }

    pub fn publish(
        &self,
        service: &[u8],
        endpoint: &owned::Endpoint,
        deadline: &deadline::Deadline,
    ) -> Result<i64, grant_client::Error> {
        if let Some(request) = self.context.lifecycle().shutdown_requested() {
            return Err(grant_client::Error::Shutdown(request));
        }
        grant_client::publish_until(self.controller, &self.descriptor, service, endpoint, deadline)
    }

    pub fn artifact_name(&self) -> Result<alloc::vec::Vec<u8>, grant_client::Error> {
        let mapping = self.descriptor.map_read_only().map_err(grant_client::Error::Memory)?;
        let descriptor = charlotte_launch::deployment::decode(mapping.as_slice())
            .ok_or(grant_client::Error::InvalidRequest)?;
        Ok(alloc::vec::Vec::from(descriptor.artifact_name))
    }
}

/// A returning application scope has disposed its resources before this value
/// selects a terminal action. Worker groups must be drained before returning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppExit {
    Completed,
    Shutdown(ShutdownRequest),
}

pub fn run<E: core::fmt::Debug>(context: Context, app: fn(&Context) -> Result<AppExit, E>) -> ! {
    match app(&context) {
        Ok(AppExit::Completed) => unsafe { catten_rt::thread_exit() },
        Ok(AppExit::Shutdown(request)) => request.complete(),
        Err(error) => {
            catten_rt::logln!("[app] stopped: {:?}", error);
            catten_rt::domain_abort()
        }
    }
}

/// Returning business code, using the existing crt0/allocator/panic contract.
#[macro_export]
macro_rules! entry {
    ($app:path) => {
        fn __charlotte_main(context: $crate::Context) -> ! {
            $crate::run(context, $app)
        }
        $crate::__runtime::entry!(__charlotte_main);
    };
}
#[doc(hidden)]
pub use catten_rt as __runtime;

pub mod prelude {
    pub use catten_rt::call::{
        Admission,
        AdmissionError,
        CallObservation,
        ObservationError,
    };

    pub use crate::{
        AppExit,
        Application,
        Context,
        LaunchError,
        ShutdownRequest,
        deadline::Deadline,
        owned::{
            CallResult,
            Connection,
            ConnectionRef,
            Endpoint,
            IpcError,
            OwnedMemory,
            OwnedPayload,
            PendingCall,
        },
    };
}
