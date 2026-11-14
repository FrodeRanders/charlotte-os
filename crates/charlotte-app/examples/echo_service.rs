//! Minimal signed application service; publish authority comes from its descriptor.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
use charlotte_app::{
    AppExit,
    Application,
    Context,
    LaunchError,
    deadline::Deadline,
    grant_client,
    owned::{
        Endpoint,
        IpcError,
    },
    service::{
        ServeError,
        Server,
    },
};

#[derive(Debug)]
pub enum Error {
    Launch(LaunchError),
    Grant(grant_client::Error),
    Ipc(IpcError),
    Serve(ServeError<IpcError>),
}

pub fn run(ctx: &Context) -> Result<AppExit, Error> {
    let app = Application::from_context(ctx).map_err(Error::Launch)?;
    let endpoint = Endpoint::create(charlotte_app::name(b"ECHO"), 1, 8).map_err(Error::Ipc)?;
    let mut server = Server::new(ctx, endpoint, 16).map_err(|(_, error)| Error::Ipc(error))?;
    let name = app.artifact_name().map_err(Error::Grant)?;
    server.publish(&app, &name, &Deadline::after(5_000)).map_err(Error::Grant)?;
    let shutdown = server
        .serve(|mut message| {
            if let Some(reply) = message.reply.take() {
                let result = if message.opcode == 1 {
                    message.arg0 as i64
                } else {
                    -1
                };
                reply.reply(result)?;
            }
            Ok(())
        })
        .map_err(Error::Serve)?;
    // The endpoint and all handler resources leave scope before acknowledgement.
    Ok(AppExit::Shutdown(shutdown))
}
#[cfg(not(test))]
charlotte_app::entry!(run);
