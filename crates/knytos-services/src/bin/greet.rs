//! The greeting service: the cluster's deployed artifact.
//!
//! This is the binary the cluster-deployment demo actually ships. The
//! pipeline signs it (`tools/cluster-sign elf-sign`) with the cluster's
//! private key and stages the note-signed ELF in the service bundle as
//! `greet.elf`; that signed ELF is the artifact stored in the object store,
//! fetched by the deploy agent, verified against the cluster public key, and
//! served across the network. The signature note is also what lets the EL0
//! loader accept it as a loadable domain.
//!
//! When run, it registers itself under the `deploy` interface and answers
//! `OP_GET` with the greeting value.
#![no_std]
#![no_main]

extern crate alloc;

use knytos_launch::greet_status as status;
use knytos_rt::{
    Context,
    ShutdownRequest,
    config,
    owned::Endpoint,
};
use knytos_services::{
    deploy,
    grant_client,
    ns,
    sleep_ms,
};
use knytos_syscall::IpcRights;

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1); // stage: started
    let bootstrap = ctx.bootstrap_connection().unwrap_or_else(|| knytos_rt::domain_abort());
    config::write::<u32>(status::STAGE, 2); // stage: bootstrap connection received

    let endpoint = Endpoint::create(deploy::INTERFACE, deploy::VERSION, 8)
        .unwrap_or_else(|_| knytos_rt::domain_abort());
    config::write::<u32>(status::STAGE, 3); // stage: endpoint created

    let generation = if let Some(descriptor) = ctx.profile_memory() {
        grant_client::publish(bootstrap, &descriptor, b"greet", &endpoint).unwrap_or_else(|error| {
            knytos_rt::logln!("[greet] publish grant failed: {:?}", error);
            knytos_rt::domain_abort()
        })
    } else {
        bootstrap
            .call_connection(
                ns::OP_REGISTER,
                deploy::NAME,
                &endpoint,
                IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
            )
            .unwrap_or_else(|_| knytos_rt::domain_abort())
            .wait()
            .unwrap_or_else(|_| knytos_rt::domain_abort())
            .result
    };
    config::write::<u32>(status::STAGE, 4); // stage: short register call sent
    if generation < 1 {
        knytos_rt::domain_abort();
    }
    config::write::<u32>(status::GENERATION, generation as u32);
    config::write::<u32>(status::STAGE, 6); // stage: serving

    loop {
        // This service owns no completion queue work other than endpoint
        // readiness. Poll the endpoint with a bounded sleep so every request
        // is observed while the lifecycle request remains responsive.
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            drop(endpoint);
            return request;
        }
        let Some(mut message) =
            endpoint.try_receive().unwrap_or_else(|_| knytos_rt::domain_abort())
        else {
            sleep_ms(10);
            continue;
        };
        match message.opcode {
            deploy::OP_GET => {
                // The deployed artifact's greeting: the leading eight bytes
                // of the artifact (its ELF header), little-endian.
                if let Some(reply) = message.reply.take() {
                    let _ = reply.reply(deploy::GREET_VALUE as i64);
                }
            }
            _ => {
                if let Some(reply) = message.reply.take() {
                    let _ = reply.reply(0);
                }
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

knytos_rt::entry!(main);
