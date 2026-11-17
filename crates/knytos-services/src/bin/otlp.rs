//! Optional native OTLP/HTTP exporter. One lossy batch; no local spool.
#![no_std]
#![no_main]
extern crate alloc;

use alloc::{
    format,
    vec::Vec,
};

use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    owned::{
        CallResult,
        Connection,
        ConnectionRef,
        PendingCall,
    },
};
use catten_services::{
    deadline::Deadline,
    net,
    ns,
    observability,
    otlp,
    otlp_transport,
    socket,
    time,
};
catten_rt::entry!(main);

struct Call {
    pending: PendingCall<'static>,
    deadline: Deadline,
}
impl Call {
    fn invoke(
        ctx: &Context,
        connection: ConnectionRef<'_>,
        opcode: u32,
        arg: u64,
        deadline: Deadline,
    ) -> Result<CallResult, ()> {
        if ctx.lifecycle().shutdown_requested().is_some() || deadline.expired() {
            return Err(());
        }
        Self::wait(ctx, connection.call(opcode, arg).map_err(|_| ())?, deadline)
    }

    fn wait(
        ctx: &Context,
        mut pending: PendingCall<'static>,
        deadline: Deadline,
    ) -> Result<CallResult, ()> {
        pending.bind_cq(0).map_err(|_| ())?;
        let mut call = Self {
            pending,
            deadline,
        };
        loop {
            // SAFETY: this service has one thread and no other CQ reader.
            unsafe { otlp_transport::drain_hints(ctx) };
            if let Some(reply) = call.pending.poll().map_err(|_| ())? {
                return Ok(reply);
            }
            if ctx.lifecycle().shutdown_requested().is_some() || call.deadline.expired() {
                return Err(());
            }
            catten_services::reactor::wait(Some(call.deadline.remaining_ms()), 0);
        }
    }
}

struct Inputs {
    observe: Connection,
    tcpip: Connection,
    time: Connection,
    net: Connection,
}
impl Inputs {
    fn open(ctx: &Context, deadline: Deadline) -> Result<Self, ()> {
        let names = ctx.bootstrap_connection().ok_or(())?;
        let lookup = |name| -> Result<Connection, ()> {
            let mut result = Call::invoke(ctx, names, ns::OP_LOOKUP, name, deadline)?;
            if result.result < 1 || result.memory.is_some() {
                return Err(());
            }
            result.connection.take().ok_or(())
        };
        Ok(Self {
            observe: lookup(observability::NAME)?,
            tcpip: lookup(socket::NAME)?,
            time: lookup(time::NAME)?,
            net: lookup(net::NAME)?,
        })
    }

    fn body(&self, ctx: &Context, deadline: Deadline) -> Result<Vec<u8>, ()> {
        let history =
            Call::invoke(ctx, self.observe.as_ref(), observability::OP_HISTORY, 0, deadline)?;
        if history.connection.is_some() || history.result <= 0 {
            return Err(());
        }
        let memory = history.memory.ok_or(())?;
        let mapping = memory.map_read_only().map_err(|_| ())?;
        let len = usize::try_from(history.result).map_err(|_| ())?;
        let (frequency, sample, previous) =
            otlp::latest_history(mapping.as_slice().get(..len).ok_or(())?).ok_or(())?;
        drop(mapping);
        let now = Call::invoke(ctx, self.time.as_ref(), time::OP_NOW, 0, deadline)?;
        if now.connection.is_some() || now.result != time::SNAPSHOT_LEN as i64 {
            return Err(());
        }
        let memory = now.memory.ok_or(())?;
        let mapping = memory.map_read_only().map_err(|_| ())?;
        let now = time::TimeSnapshot::decode(mapping.as_slice()).ok_or(())?;
        drop(mapping);
        if now.state != time::STATE_SYNCHRONIZED
            || now.unix_seconds <= 0
            || now.nanosecond >= 1_000_000_000
            || now.counter_frequency_hz != frequency
            || now.monotonic_ticks < sample.ticks
        {
            return Err(());
        }
        let age =
            u128::from(now.monotonic_ticks - sample.ticks) * 1_000_000_000 / u128::from(frequency);
        if age > 5_000_000_000 {
            return Err(());
        }
        let unix = (now.unix_seconds as u64)
            .checked_mul(1_000_000_000)
            .and_then(|n| n.checked_add(u64::from(now.nanosecond)))
            .and_then(|n| n.checked_sub(age as u64))
            .ok_or(())?;
        let status = Call::invoke(ctx, self.net.as_ref(), net::OP_STATUS, 0, deadline)?;
        if status.memory.is_some() || status.connection.is_some() {
            return Err(());
        }
        let (link, mac) = charlotte_protocol_net::decode_status(status.result);
        if link == 0 {
            return Err(());
        }
        let [a, b, c, d, e, f] = mac;
        let instance = format!("{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}");
        otlp::metrics(&instance, unix, frequency, sample, previous).ok_or(())
    }
}

fn serve(ctx: &Context) -> ShutdownRequest {
    let profile = match ctx.manifest_value(charlotte_launch::otlp::KEY) {
        Some(ManifestValue::Bytes(bytes)) => charlotte_launch::otlp::Profile::decode(bytes),
        _ => None,
    }
    .unwrap_or_else(|| catten_rt::domain_abort());
    let [a, b, c, d] = profile.address;
    catten_rt::logln!(
        "[otlp] collector=http://{}.{}.{}.{}:{}{} interval_ms={} timeout_ms={} \
         archive_independent=true",
        a,
        b,
        c,
        d,
        profile.port,
        profile.path,
        profile.interval_ms,
        profile.timeout_ms
    );
    let mut failures = 0u32;
    let mut accepted = 0u64;
    let mut dropped = 0u64;
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return request;
        }
        // All preparation calls and transport waits share this one deadline.
        let deadline = Deadline::after(profile.timeout_ms);
        let result = (|| {
            let inputs =
                Inputs::open(ctx, deadline).map_err(|_| otlp_transport::Error::Admission)?;
            let body = inputs.body(ctx, deadline).map_err(|_| otlp_transport::Error::Protocol)?;
            let request = otlp::request(&profile, &body).ok_or(otlp_transport::Error::Protocol)?;
            // SAFETY: the exporter has one thread and owns the default CQ's
            // sole reader role through all preparation and transport phases.
            unsafe { otlp_transport::post(ctx, inputs.tcpip.as_ref(), &profile, request, deadline) }
        })();
        let retry_after = match result {
            Ok(otlp_transport::Outcome::Rejected {
                status: 429 | 503,
                retry_after_ms,
            }) => retry_after_ms.unwrap_or(0),
            _ => 0,
        };
        match result {
            Ok(otlp_transport::Outcome::Accepted(otlp::Acknowledgement::Full)) => {
                accepted = accepted.saturating_add(1);
                failures = 0;
                catten_rt::logln!("[otlp] accepted batches={} dropped={}", accepted, dropped);
            }
            Ok(otlp_transport::Outcome::Accepted(otlp::Acknowledgement::Partial {
                rejected,
            })) => {
                dropped = dropped.saturating_add(1);
                failures = 0;
                catten_rt::logln!(
                    "[otlp] partial success rejected={} batch terminal; no replay",
                    rejected
                );
            }
            result => {
                dropped = dropped.saturating_add(1);
                failures = failures.saturating_add(1);
                catten_rt::logln!(
                    "[otlp] batch dropped={} failure={:?}; sampling remains independent",
                    dropped,
                    result
                );
            }
        }
        // No persistent queue and no replay of ambiguous or partial requests.
        // Backoff drops stale samples; the next attempt collects fresh data.
        let delay = if failures == 0 {
            profile.interval_ms
        } else {
            profile
                .interval_ms
                .saturating_mul(1u64 << failures.min(6))
                .min(profile.interval_ms.max(300_000))
        };
        let delay = delay.max(retry_after);
        let jitter = if failures == 0 {
            0
        } else {
            catten_syscall::monotonic_clock().0 % 1_000
        };
        if let Err(request) =
            catten_services::sleep_ms_or_shutdown(ctx, delay.saturating_add(jitter))
        {
            return request;
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}
