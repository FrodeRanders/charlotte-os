//! Minimal lifecycle verifier used by the isolated kernel shutdown test.
#![no_std]
#![no_main]

use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    config,
    owned::Endpoint,
};
use catten_services::sleep_ms;

const INTERFACE: u64 = 0x5348_5554_444f_574e; // "SHUTDOWN"
const VERSION: u32 = 1;
const MODE_KEY: u64 = 0x7368_7574_6d6f_6465; // "shutmode"
const MODE_STUBBORN: u64 = 1;
const STATUS_STARTED: usize = 0;

fn serve(ctx: &Context) -> ShutdownRequest {
    let stubborn =
        matches!(ctx.manifest_value(MODE_KEY), Some(ManifestValue::Unsigned(MODE_STUBBORN)));
    let endpoint =
        Endpoint::create(INTERFACE, VERSION, 1).unwrap_or_else(|_| catten_rt::domain_abort());
    config::write::<u32>(STATUS_STARTED, 1);

    loop {
        if !stubborn && let Some(request) = ctx.lifecycle().shutdown_requested() {
            drop(endpoint);
            return request;
        }
        sleep_ms(10);
    }
}

fn main(ctx: Context) -> ! {
    let device = matches!(ctx.manifest_value(MODE_KEY), Some(ManifestValue::Unsigned(2)));
    let request = serve(&ctx);
    if device {
        request.complete_device_quiesced()
    } else {
        request.complete()
    }
}

catten_rt::entry!(main);
