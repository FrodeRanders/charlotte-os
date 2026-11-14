//! A single-endpoint serving scope and explicitly joined native workers.
//! Handlers are synchronous; multiplexed calls use owned reactor operations.

use catten_rt::{
    Context,
    ShutdownRequest,
    cq,
    owned::{
        Endpoint,
        IncomingMessage,
        IpcError,
        ReceiveError,
    },
};

pub struct Server<'ctx> {
    endpoint: Endpoint,
    context: &'ctx Context,
    batch: usize,
    publication: Option<i64>,
    publication_attempted: bool,
}

#[derive(Debug)]
pub enum ServeError<E> {
    Receive(ReceiveError),
    Wait(cq::WaitError),
    Handler(E),
}

impl<'ctx> Server<'ctx> {
    /// Queue zero observes authenticated lifecycle publication as well as IPC.
    /// The server owns its endpoint even when readiness binding rejects.
    pub fn new(
        context: &'ctx Context,
        endpoint: Endpoint,
        batch: usize,
    ) -> Result<Self, (Endpoint, IpcError)> {
        if let Err(error) = endpoint.bind_completion_queue(0) {
            return Err((endpoint, error));
        }
        Ok(Self {
            endpoint,
            context,
            batch: batch.max(1),
            publication: None,
            publication_attempted: false,
        })
    }

    /// Publish only after the caller has initialized its dependencies/state.
    pub fn publish(
        &mut self,
        app: &crate::Application<'_>,
        name: &[u8],
        deadline: &crate::deadline::Deadline,
    ) -> Result<i64, crate::grant_client::Error> {
        if self.publication_attempted {
            return Err(crate::grant_client::Error::InvalidRequest);
        }
        // An uncertain registration must not be replayed by reusing this scope.
        self.publication_attempted = true;
        let generation = app.publish(name, &self.endpoint, deadline)?;
        self.publication = Some(generation);
        Ok(generation)
    }

    pub fn publication_generation(&self) -> Option<i64> {
        self.publication
    }

    /// Drain before parking, rechecking shutdown between requests and after
    /// every wake. `self` stays borrowed on error, retaining the endpoint.
    pub fn serve<E>(
        &mut self,
        mut handle: impl FnMut(IncomingMessage) -> Result<(), E>,
    ) -> Result<ShutdownRequest, ServeError<E>> {
        loop {
            let mut drained = false;
            for _ in 0..self.batch {
                if let Some(request) = self.context.lifecycle().shutdown_requested() {
                    return Ok(request);
                }
                match self.endpoint.try_receive().map_err(ServeError::Receive)? {
                    Some(message) => handle(message).map_err(ServeError::Handler)?,
                    None => {
                        drained = true;
                        break;
                    }
                }
            }
            if drained {
                cq::wait(None, 0).map_err(ServeError::Wait)?;
            }
        }
    }
}

pub use catten_rt::owned::{
    WorkerCapacity,
    Workers,
};
