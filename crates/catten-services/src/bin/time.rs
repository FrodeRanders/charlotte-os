//! UTC time service with an SNTP client and persistent holdover calibration.
//!
//! The service uses the observe service's hardware-backed monotonic counter as
//! its oscillator. It samples an NTPv4 server over connected UDP, estimates
//! network uncertainty and counter frequency error, and persists the latest
//! calibration in the local object store. Persistence prevents the clock from
//! returning to 1970 after a warm restart, but cannot account for powered-off
//! time; that state is explicitly reported as `STATE_HOLDOVER` until a fresh
//! network sample arrives.
#![no_std]
#![no_main]

use core::cmp;

use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    config,
    owned::{
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        PendingCall,
        ReplyToken,
    },
};
use catten_services::{
    ns,
    objstore,
    observability,
    socket,
    time,
    wait_for_local_ready_owned,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    IpcRights,
    THREAD_STATISTICS_HEADER_U64S,
    THREAD_STATISTICS_MAGIC,
    THREAD_STATISTICS_VERSION,
    cq_read,
    cq_wait_timeout,
    thread_exit,
    thread_statistics_header as thread_header,
};
use charlotte_launch::time_status as status;

catten_rt::entry!(main);

const NTP_PORT: u16 = 123;
const NTP_PACKET_LEN: usize = 48;
const NTP_UNIX_EPOCH_SECONDS: u64 = 2_208_988_800;
const NTP_ERA_SECONDS: u64 = 1u64 << 32;
const DEFAULT_NTP_SERVER: [u8; 4] = [162, 159, 200, 1];
const LOOP_WAIT_MS: u64 = 1_000;
const REQUEST_TIMEOUT_MS: u64 = 5_000;
const RETRY_INTERVAL_SECONDS: u64 = 64;
const SYNC_INTERVAL_SECONDS: u64 = 900;
const MAX_DRIFT_PPB: i64 = 500_000;
const UNCERTAINTY_FLOOR_PPB: u64 = 50_000;

/// Reserved local-system object namespace, separate from fffe artifacts and
/// ffff executable fixtures.
const CALIBRATION_OBJECT_ID: u64 = 0xfffd_0000_0000_0001;
const CALIBRATION_MAGIC: u64 = 0x314c_4143_454d_4954; // "TIMECAL1"
const CALIBRATION_VERSION: u32 = 1;
const CALIBRATION_LEN: usize = 64;

#[derive(Clone, Copy)]
struct MonoClock {
    ticks: u64,
    frequency_hz: u64,
}

struct ClockModel {
    anchor_utc_ns: u64,
    anchor_mono_ticks: u64,
    frequency_hz: u64,
    drift_ppb: i64,
    uncertainty_ns: u64,
    state: u8,
    stratum: u8,
    leap_indicator: u8,
    sample_count: u32,
    last_sync_mono_ticks: Option<u64>,
    last_sample_utc_ns: Option<u64>,
    last_sample_mono_ticks: Option<u64>,
    last_returned_ns: u64,
}

impl ClockModel {
    fn from_persisted(record: Calibration, mono: MonoClock) -> Self {
        Self {
            anchor_utc_ns: record.utc_ns,
            anchor_mono_ticks: mono.ticks,
            frequency_hz: mono.frequency_hz,
            drift_ppb: record.drift_ppb.clamp(-MAX_DRIFT_PPB, MAX_DRIFT_PPB),
            uncertainty_ns: u64::MAX,
            state: time::STATE_HOLDOVER,
            stratum: record.stratum,
            leap_indicator: record.leap_indicator,
            sample_count: record.sample_count,
            last_sync_mono_ticks: None,
            // A monotonic counter does not span power loss, so persisted and
            // current anchors must never be used for a drift measurement.
            last_sample_utc_ns: None,
            last_sample_mono_ticks: None,
            last_returned_ns: record.utc_ns,
        }
    }

    fn elapsed_ns(&self, mono_ticks: u64) -> u64 {
        if self.frequency_hz == 0 || mono_ticks <= self.anchor_mono_ticks {
            return 0;
        }
        let nominal = u128::from(mono_ticks - self.anchor_mono_ticks).saturating_mul(1_000_000_000)
            / u128::from(self.frequency_hz);
        let scale = (1_000_000_000i64 + self.drift_ppb).max(1) as u128;
        nominal.saturating_mul(scale).checked_div(1_000_000_000).unwrap_or(0).min(u64::MAX as u128)
            as u64
    }

    fn utc_ns(&mut self, mono_ticks: u64) -> u64 {
        let estimated = self.anchor_utc_ns.saturating_add(self.elapsed_ns(mono_ticks));
        // An accepted correction may move the model backwards. Never expose a
        // regression to applications; hold at the prior value until real UTC
        // catches up.
        self.last_returned_ns = cmp::max(self.last_returned_ns, estimated);
        self.last_returned_ns
    }

    fn snapshot(&mut self, mono: MonoClock, server_ipv4: [u8; 4]) -> time::TimeSnapshot {
        let utc_ns = self.utc_ns(mono.ticks);
        let unix_seconds = (utc_ns / 1_000_000_000) as i64;
        let age_ms = self.last_sync_mono_ticks.map_or(time::NEVER_SYNCED_AGE_MS, |last| {
            ticks_to_ns(mono.ticks.saturating_sub(last), mono.frequency_hz) / 1_000_000
        });
        let uncertainty_ms = if self.uncertainty_ns == u64::MAX {
            u32::MAX
        } else {
            let growth_ppb = cmp::max(self.drift_ppb.unsigned_abs(), UNCERTAINTY_FLOOR_PPB);
            let growth_ns = age_ms.saturating_mul(growth_ppb) / 1_000;
            self.uncertainty_ns.saturating_add(growth_ns).div_ceil(1_000_000).min(u32::MAX as u64)
                as u32
        };
        time::TimeSnapshot {
            state: self.state,
            stratum: self.stratum,
            unix_seconds,
            nanosecond: (utc_ns % 1_000_000_000) as u32,
            uncertainty_ms,
            drift_ppb: self.drift_ppb,
            last_sync_age_ms: age_ms,
            monotonic_ticks: mono.ticks,
            counter_frequency_hz: mono.frequency_hz,
            utc: time::utc_from_unix(unix_seconds),
            leap_indicator: self.leap_indicator,
            server_ipv4,
            sample_count: self.sample_count,
        }
    }

    fn accept_sample(&mut self, sample: NtpSample, mono: MonoClock) {
        let mut drift = self.drift_ppb;
        if let (Some(old_utc), Some(old_mono)) =
            (self.last_sample_utc_ns, self.last_sample_mono_ticks)
            && sample.mono_ticks > old_mono
            && sample.utc_ns > old_utc
        {
            let nominal = ticks_to_ns(sample.mono_ticks - old_mono, mono.frequency_hz);
            let actual = sample.utc_ns - old_utc;
            if nominal > 0 {
                let measured = ((actual as i128 - nominal as i128) * 1_000_000_000i128
                    / nominal as i128)
                    .clamp(-(MAX_DRIFT_PPB as i128), MAX_DRIFT_PPB as i128)
                    as i64;
                // Damp network-path jitter: a new measurement contributes one
                // quarter to the retained oscillator estimate.
                drift = (drift.saturating_mul(3).saturating_add(measured)) / 4;
            }
        }
        self.anchor_utc_ns = sample.utc_ns;
        self.anchor_mono_ticks = sample.mono_ticks;
        self.frequency_hz = mono.frequency_hz;
        self.drift_ppb = drift;
        self.uncertainty_ns = sample.uncertainty_ns;
        self.state = time::STATE_SYNCHRONIZED;
        self.stratum = sample.stratum;
        self.leap_indicator = sample.leap_indicator;
        self.sample_count = self.sample_count.saturating_add(1);
        self.last_sync_mono_ticks = Some(sample.mono_ticks);
        self.last_sample_utc_ns = Some(sample.utc_ns);
        self.last_sample_mono_ticks = Some(sample.mono_ticks);
    }
}

#[derive(Clone, Copy)]
struct Calibration {
    utc_ns: u64,
    drift_ppb: i64,
    uncertainty_ns: u64,
    sample_count: u32,
    stratum: u8,
    leap_indicator: u8,
    server_ipv4: [u8; 4],
}

impl Calibration {
    fn encode(self) -> [u8; CALIBRATION_LEN] {
        let mut bytes = [0u8; CALIBRATION_LEN];
        put_u64_le(&mut bytes, 0, CALIBRATION_MAGIC);
        put_u32_le(&mut bytes, 8, CALIBRATION_VERSION);
        put_u32_le(&mut bytes, 12, CALIBRATION_LEN as u32);
        put_u64_le(&mut bytes, 16, self.utc_ns);
        put_u64_le(&mut bytes, 24, self.drift_ppb as u64);
        put_u64_le(&mut bytes, 32, self.uncertainty_ns);
        put_u32_le(&mut bytes, 40, self.sample_count);
        bytes[44] = self.stratum;
        bytes[45] = self.leap_indicator;
        bytes[48..52].copy_from_slice(&self.server_ipv4);
        let checksum = fnv1a64(&bytes[..56]);
        put_u64_le(&mut bytes, 56, checksum);
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CALIBRATION_LEN
            || get_u64_le(bytes, 0)? != CALIBRATION_MAGIC
            || get_u32_le(bytes, 8)? != CALIBRATION_VERSION
            || get_u32_le(bytes, 12)? as usize != CALIBRATION_LEN
            || get_u64_le(bytes, 56)? != fnv1a64(&bytes[..56])
        {
            return None;
        }
        Some(Self {
            utc_ns: get_u64_le(bytes, 16)?,
            drift_ppb: get_u64_le(bytes, 24)? as i64,
            uncertainty_ns: get_u64_le(bytes, 32)?,
            sample_count: get_u32_le(bytes, 40)?,
            stratum: bytes[44],
            leap_indicator: bytes[45],
            server_ipv4: bytes[48..52].try_into().ok()?,
        })
    }
}

struct Attempt<'connection> {
    recv_call: PendingCall<'static>,
    _socket: socket::OwnedSocket<'connection>,
    request_transmit: u64,
    sent_mono_ticks: u64,
    deadline_mono_ticks: u64,
}

#[derive(Clone, Copy)]
struct NtpSample {
    utc_ns: u64,
    mono_ticks: u64,
    uncertainty_ns: u64,
    stratum: u8,
    leap_indicator: u8,
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    catten_syscall::el0_log(0x454d_4954, code as u64); // "TIME"
    unsafe { thread_exit() }
}

fn wait_scalar(call: PendingCall<'_>) -> Option<(i64, Option<Connection>)> {
    let result = call.wait().ok()?;
    Some((result.result, result.connection))
}

fn read_monotonic(observe_conn: ConnectionRef<'_>) -> Option<MonoClock> {
    let result = observe_conn.call(observability::OP_THREAD_SNAPSHOT, 0).ok()?.wait().ok()?;
    if result.result < (THREAD_STATISTICS_HEADER_U64S * core::mem::size_of::<u64>()) as i64 {
        return None;
    }
    let memory = result.memory?;
    let mapping = memory.map_read_only().ok()?;
    if mapping.len() < THREAD_STATISTICS_HEADER_U64S * core::mem::size_of::<u64>() {
        return None;
    }
    let header = unsafe {
        core::slice::from_raw_parts(mapping.as_ptr().cast::<u64>(), THREAD_STATISTICS_HEADER_U64S)
    };
    (header[thread_header::MAGIC] == THREAD_STATISTICS_MAGIC
        && header[thread_header::VERSION] == THREAD_STATISTICS_VERSION
        && header[thread_header::COUNTER_FREQUENCY_HZ] > 0)
        .then_some(MonoClock {
            ticks: header[thread_header::MONOTONIC_TICKS],
            frequency_hz: header[thread_header::COUNTER_FREQUENCY_HZ],
        })
}

fn ticks_to_ns(ticks: u64, frequency_hz: u64) -> u64 {
    if frequency_hz == 0 {
        return 0;
    }
    (u128::from(ticks).saturating_mul(1_000_000_000) / u128::from(frequency_hz))
        .min(u64::MAX as u128) as u64
}

fn ticks_after(clock: MonoClock, seconds: u64) -> u64 {
    clock.ticks.saturating_add(clock.frequency_hz.saturating_mul(seconds))
}

fn tcpip_has_ipv4(tcp_conn: ConnectionRef<'_>) -> bool {
    let Ok(call) = tcp_conn.call(socket::OP_STATUS, 0) else {
        return false;
    };
    let Ok(result) = call.wait() else {
        return false;
    };
    if result.result < core::mem::size_of::<u32>() as i64 {
        return false;
    }
    result
        .memory
        .and_then(|memory| memory.map_read_only().ok())
        .filter(|mapping| mapping.len() >= core::mem::size_of::<u32>())
        .is_some_and(|mapping| {
            (unsafe { core::ptr::read_unaligned(mapping.as_ptr().cast::<u32>()) }) != 0
        })
}

fn reply_bytes(reply: ReplyToken, bytes: &[u8]) -> bool {
    let Ok(memory) = OwnedMemory::allocate(1) else {
        let _ = reply.reply(time::ERR_UNAVAILABLE);
        return false;
    };
    let Ok(mut mapping) = memory.map_writable() else {
        let _ = reply.reply(time::ERR_UNAVAILABLE);
        return false;
    };
    if bytes.len() > mapping.len() {
        let _ = reply.reply(time::ERR_UNAVAILABLE);
        return false;
    }
    mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
    let Ok(memory) = mapping.unmap() else {
        let _ = reply.reply(time::ERR_UNAVAILABLE);
        return false;
    };
    reply.reply_move(memory, bytes.len() as i64).is_ok()
}

fn ntp_request_token(model: &mut Option<ClockModel>, mono: MonoClock) -> u64 {
    if let Some(model) = model {
        let unix_ns = model.utc_ns(mono.ticks);
        let seconds = unix_ns / 1_000_000_000;
        let fraction_ns = unix_ns % 1_000_000_000;
        let ntp_seconds = seconds.saturating_add(NTP_UNIX_EPOCH_SECONDS) as u32;
        let fraction = ((u128::from(fraction_ns) << 32) / 1_000_000_000) as u32;
        return (u64::from(ntp_seconds) << 32) | u64::from(fraction);
    }
    // The server treats this as an opaque nonce and echoes it in Originate.
    // Keep it non-zero even before wall-clock synchronization.
    mono.ticks.rotate_left(17) ^ 0xa5c3_7d19_0000_0001
}

fn start_attempt<'connection>(
    tcp_conn: ConnectionRef<'connection>,
    server_ipv4: [u8; 4],
    mono: MonoClock,
    model: &mut Option<ClockModel>,
) -> Option<Attempt<'connection>> {
    let socket = socket::OwnedSocket::open(tcp_conn, socket::DOMAIN_UDP).ok()?;

    let address_memory = OwnedMemory::allocate(1).ok()?;
    let mut address_mapping = address_memory.map_writable().ok()?;
    address_mapping.as_mut_slice()[..4].copy_from_slice(&server_ipv4);
    address_mapping.as_mut_slice()[4..6].copy_from_slice(&NTP_PORT.to_le_bytes());
    let address_memory = address_mapping.unmap().ok()?;
    let connect = tcp_conn.call_move(socket::OP_CONNECT, socket.id(), address_memory).ok()?;
    if wait_scalar(connect).is_none_or(|(result, _)| result != 0) {
        return None;
    }

    let request_transmit = ntp_request_token(model, mono);
    let packet_memory = OwnedMemory::allocate(1).ok()?;
    let mut packet_mapping = packet_memory.map_writable().ok()?;
    packet_mapping.as_mut_slice()[..NTP_PACKET_LEN].fill(0);
    packet_mapping.as_mut_slice()[0] = 0x23; // LI=0, VN=4, mode=client
    packet_mapping.as_mut_slice()[40..48].copy_from_slice(&request_transmit.to_be_bytes());
    let packet_memory = packet_mapping.unmap().ok()?;
    let packed_send = ((NTP_PACKET_LEN as u64) << 32) | (socket.id() & 0xffff_ffff);
    let send = tcp_conn.call_move(socket::OP_SEND, packed_send, packet_memory).ok()?;
    if wait_scalar(send).is_none_or(|(result, _)| result != NTP_PACKET_LEN as i64) {
        return None;
    }
    let recv_call = tcp_conn.call(socket::OP_RECV, socket.id()).ok()?;
    Some(Attempt {
        recv_call,
        _socket: socket,
        request_transmit,
        sent_mono_ticks: mono.ticks,
        deadline_mono_ticks: mono
            .ticks
            .saturating_add(mono.frequency_hz.saturating_mul(REQUEST_TIMEOUT_MS) / 1_000),
    })
}

fn ntp_to_unix_ns(timestamp: u64) -> Option<u64> {
    let seconds32 = timestamp >> 32;
    let fraction = timestamp as u32;
    // Era zero covers 1970..2036. After the 2036 wrap, a small seconds field
    // denotes era one; dates before 1970 are deliberately outside this ABI.
    let ntp_seconds = if seconds32 < NTP_UNIX_EPOCH_SECONDS {
        seconds32.checked_add(NTP_ERA_SECONDS)?
    } else {
        seconds32
    };
    let unix_seconds = ntp_seconds.checked_sub(NTP_UNIX_EPOCH_SECONDS)?;
    let nanos = (u128::from(fraction) * 1_000_000_000u128) >> 32;
    unix_seconds.checked_mul(1_000_000_000)?.checked_add(nanos as u64)
}

fn parse_ntp_reply(bytes: &[u8], attempt: &Attempt<'_>, received: MonoClock) -> Option<NtpSample> {
    if bytes.len() < NTP_PACKET_LEN {
        return None;
    }
    let leap = bytes[0] >> 6;
    let version = (bytes[0] >> 3) & 0x07;
    let mode = bytes[0] & 0x07;
    let stratum = bytes[1];
    let originate = u64::from_be_bytes(bytes[24..32].try_into().ok()?);
    let receive = u64::from_be_bytes(bytes[32..40].try_into().ok()?);
    let transmit = u64::from_be_bytes(bytes[40..48].try_into().ok()?);
    if leap == 3
        || version < 3
        || mode != 4
        || !(1..=15).contains(&stratum)
        || originate != attempt.request_transmit
        || receive == 0
        || transmit == 0
    {
        return None;
    }
    let receive_ns = ntp_to_unix_ns(receive)?;
    let transmit_ns = ntp_to_unix_ns(transmit)?;
    if transmit_ns < receive_ns || received.ticks < attempt.sent_mono_ticks {
        return None;
    }
    let local_round_trip_ns =
        ticks_to_ns(received.ticks - attempt.sent_mono_ticks, received.frequency_hz);
    let server_processing_ns = transmit_ns - receive_ns;
    let network_round_trip_ns = local_round_trip_ns.saturating_sub(server_processing_ns);
    let root_dispersion = u32::from_be_bytes(bytes[8..12].try_into().ok()?);
    let root_dispersion_ns =
        ((u128::from(root_dispersion) * 1_000_000_000u128) >> 16).min(u64::MAX as u128) as u64;
    Some(NtpSample {
        utc_ns: ((u128::from(receive_ns) + u128::from(transmit_ns)) / 2) as u64,
        mono_ticks: attempt.sent_mono_ticks + (received.ticks - attempt.sent_mono_ticks) / 2,
        uncertainty_ns: network_round_trip_ns / 2 + root_dispersion_ns,
        stratum,
        leap_indicator: leap,
    })
}

fn poll_attempt(attempt: &mut Attempt<'_>, received: MonoClock) -> Option<Result<NtpSample, ()>> {
    let result = match attempt.recv_call.poll() {
        Ok(Some(result)) => result,
        Ok(None) => {
            if received.ticks >= attempt.deadline_mono_ticks {
                catten_rt::logln!("[time] NTP response timed out");
                return Some(Err(()));
            }
            return None;
        }
        Err(_) => return Some(Err(())),
    };
    if result.result < NTP_PACKET_LEN as i64 {
        return Some(Err(()));
    }
    let Some(memory) = result.memory else {
        return Some(Err(()));
    };
    let Ok(mapping) = memory.map_read_only() else {
        return Some(Err(()));
    };
    let length = result.result as usize;
    if length > mapping.len() {
        return Some(Err(()));
    }
    let bytes = &mapping.as_slice()[..length];
    let sample = parse_ntp_reply(bytes, attempt, received).ok_or(());
    if sample.is_err() {
        catten_rt::logln!(
            "[time] rejected NTP reply: len={} header={:#x} stratum={}",
            result.result,
            bytes[0],
            bytes[1]
        );
    }
    Some(sample)
}

fn load_calibration(obj_conn: ConnectionRef<'_>) -> Option<Calibration> {
    let result = obj_conn.call(objstore::OP_READ, CALIBRATION_OBJECT_ID).ok()?.wait().ok()?;
    if result.result < CALIBRATION_LEN as i64 {
        return None;
    }
    let memory = result.memory?;
    let mapping = memory.map_read_only().ok()?;
    if mapping.len() < CALIBRATION_LEN {
        return None;
    }
    Calibration::decode(&mapping.as_slice()[..CALIBRATION_LEN])
}

fn save_calibration(obj_conn: ConnectionRef<'_>, record: Calibration) -> bool {
    let Ok(create) = obj_conn.call(objstore::OP_CREATE_AT, CALIBRATION_OBJECT_ID) else {
        return false;
    };
    let Some((created, _)) = wait_scalar(create) else {
        return false;
    };
    if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
        return false;
    }
    let Ok(size_memory) = OwnedMemory::allocate(1) else {
        return false;
    };
    let Ok(mut size_mapping) = size_memory.map_writable() else {
        return false;
    };
    size_mapping.as_mut_slice()[..8].copy_from_slice(&(CALIBRATION_LEN as u64).to_le_bytes());
    let Ok(size_memory) = size_mapping.unmap() else {
        return false;
    };
    let sized = obj_conn
        .call_borrow_read(objstore::OP_SET_SIZE, CALIBRATION_OBJECT_ID, &size_memory)
        .ok()
        .and_then(wait_scalar)
        .is_some_and(|(result, _)| result == objstore::ERR_OK);
    if !sized {
        return false;
    }
    let Ok(data_memory) = OwnedMemory::allocate(1) else {
        return false;
    };
    let Ok(mut data_mapping) = data_memory.map_writable() else {
        return false;
    };
    let bytes = record.encode();
    data_mapping.as_mut_slice()[..bytes.len()].copy_from_slice(&bytes);
    let Ok(data_memory) = data_mapping.unmap() else {
        return false;
    };
    let Ok(write) = obj_conn.call_move(objstore::OP_WRITE, CALIBRATION_OBJECT_ID, data_memory)
    else {
        return false;
    };
    if wait_scalar(write).is_none_or(|(result, _)| result != objstore::ERR_OK) {
        return false;
    }
    obj_conn
        .call(objstore::OP_FLUSH, 0)
        .ok()
        .and_then(wait_scalar)
        .is_some_and(|(result, _)| result == objstore::ERR_OK)
}

fn calibration_from_model(model: &ClockModel, server_ipv4: [u8; 4]) -> Calibration {
    Calibration {
        utc_ns: model.anchor_utc_ns,
        drift_ppb: model.drift_ppb,
        uncertainty_ns: model.uncertainty_ns,
        sample_count: model.sample_count,
        stratum: model.stratum,
        leap_indicator: model.leap_indicator,
        server_ipv4,
    }
}

fn handle_requests(
    endpoint: &Endpoint,
    observe_conn: ConnectionRef<'_>,
    model: &mut Option<ClockModel>,
    server_ipv4: [u8; 4],
) {
    loop {
        let message = match endpoint.try_receive() {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(catten_rt::owned::ReceiveError::EndpointClosed) => unsafe { thread_exit() },
            Err(_) => continue,
        };
        let Some(reply) = message.reply else {
            continue;
        };
        match message.opcode {
            time::OP_NOW | time::OP_ISO8601 => {
                let Some(clock) = read_monotonic(observe_conn) else {
                    let _ = reply.reply(time::ERR_UNAVAILABLE);
                    continue;
                };
                let Some(model) = model.as_mut() else {
                    let _ = reply.reply(time::ERR_UNSYNCHRONIZED);
                    continue;
                };
                let snapshot = model.snapshot(clock, server_ipv4);
                if message.opcode == time::OP_NOW {
                    reply_bytes(reply, &snapshot.encode());
                } else if let Some(iso) = time::iso8601(&snapshot) {
                    reply_bytes(reply, &iso);
                } else {
                    let _ = reply.reply(time::ERR_UNAVAILABLE);
                }
            }
            time::OP_UNIX_SECONDS => {
                let Some(clock) = read_monotonic(observe_conn) else {
                    let _ = reply.reply(time::ERR_UNAVAILABLE);
                    continue;
                };
                let Some(model) = model.as_mut() else {
                    let _ = reply.reply(time::ERR_UNSYNCHRONIZED);
                    continue;
                };
                let _ = reply.reply((model.utc_ns(clock.ticks) / 1_000_000_000) as i64);
            }
            _ => {
                let _ = reply.reply(time::ERR_BAD_OPCODE);
            }
        }
    }
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);

    // Local name service
    let ns_conn = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xe001));

    // The 'observe' service is used as local timepiece
    let (_, observe_conn) = wait_for_registered_name_owned(ns_conn, observability::NAME)
        .unwrap_or_else(|| fail(0xe002));

    let (_, tcp_conn) =
        wait_for_registered_name_owned(ns_conn, socket::NAME).unwrap_or_else(|| fail(0xe003));
    let persistence_requested =
        ctx.manifest_value(charlotte_launch::manifest_key(b"persist")).is_some();
    let obj_conn = if persistence_requested {
        Some(
            wait_for_registered_name_owned(ns_conn, objstore::NAME)
                .unwrap_or_else(|| fail(0xe004))
                .1,
        )
    } else {
        None
    };
    let server_ipv4 = match ctx.manifest_value(charlotte_launch::manifest_key(b"ntp_ip")) {
        Some(ManifestValue::Bytes(bytes)) if bytes.len() == 4 => {
            [bytes[0], bytes[1], bytes[2], bytes[3]]
        }
        _ => DEFAULT_NTP_SERVER,
    };
    let initial_mono = read_monotonic(observe_conn.as_ref()).unwrap_or_else(|| fail(0xe005));
    let mut model = obj_conn
        .as_ref()
        .and_then(|connection| load_calibration(connection.as_ref()))
        .filter(|record| record.server_ipv4 == server_ipv4)
        .map(|record| ClockModel::from_persisted(record, initial_mono));
    config::write::<u32>(
        status::SYNC_STATE,
        model.as_ref().map_or(time::STATE_UNSYNCHRONIZED, |clock| clock.state) as u32,
    );

    // Publish 'time' service (through its endpoint) with the 'name' service
    let endpoint =
        Endpoint::create(time::INTERFACE, time::VERSION, 32).unwrap_or_else(|_| fail(0xe006));
    let registration = ns_conn
        .call_connection(
            ns::OP_REGISTER,
            time::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| fail(0xe007));
    if wait_scalar(registration).is_none_or(|(generation, _)| generation < 1)
        || endpoint.bind_completion_queue(0).is_err()
    {
        fail(0xe007);
    }
    catten_rt::logln!(
        "[time] serving; NTP {}.{}.{}.{}:{} persistence={}",
        server_ipv4[0],
        server_ipv4[1],
        server_ipv4[2],
        server_ipv4[3],
        NTP_PORT,
        obj_conn.is_some()
    );
    config::write::<u32>(status::STAGE, 2);

    // Avoid firing the first packet into the boot storm. Failure still leaves
    // the 'time' endpoint available in unsynchronized/holdover state.
    let network_ready = wait_for_local_ready_owned(ns_conn);
    let mut next_sync_ticks = initial_mono.ticks;
    if !network_ready {
        next_sync_ticks = ticks_after(initial_mono, RETRY_INTERVAL_SECONDS);
    }
    let mut attempt: Option<Attempt> = None;
    let mut ntp_failures: u32 = 0;
    let cq = ctx.completion_queue_layout();

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            // Dropping the endpoint closes admission. Any active NTP attempt,
            // its UDP socket, and all owned service connections then unwind
            // together before the lifecycle acknowledgement is published.
            return request;
        }
        handle_requests(&endpoint, observe_conn.as_ref(), &mut model, server_ipv4);
        let Some(now_mono) = read_monotonic(observe_conn.as_ref()) else {
            config::write::<u32>(status::ERROR, 0xe008);
            cq_wait_timeout(1, LOOP_WAIT_MS, 0);
            continue;
        };

        if let Some(active) = attempt.as_mut()
            && let Some(result) = poll_attempt(active, now_mono)
        {
            drop(attempt.take());
            match result {
                Ok(sample) => {
                    if let Some(clock) = model.as_mut() {
                        clock.accept_sample(sample, now_mono);
                    } else {
                        let mut clock = ClockModel {
                            anchor_utc_ns: sample.utc_ns,
                            anchor_mono_ticks: sample.mono_ticks,
                            frequency_hz: now_mono.frequency_hz,
                            drift_ppb: 0,
                            uncertainty_ns: sample.uncertainty_ns,
                            state: time::STATE_SYNCHRONIZED,
                            stratum: sample.stratum,
                            leap_indicator: sample.leap_indicator,
                            sample_count: 0,
                            last_sync_mono_ticks: None,
                            last_sample_utc_ns: None,
                            last_sample_mono_ticks: None,
                            last_returned_ns: sample.utc_ns,
                        };
                        clock.accept_sample(sample, now_mono);
                        model = Some(clock);
                    }
                    let clock = model.as_ref().unwrap();
                    config::write::<u32>(status::SYNC_STATE, clock.state as u32);
                    config::write::<u32>(status::SAMPLES, clock.sample_count);
                    config::write::<i64>(status::DRIFT_PPB, clock.drift_ppb);
                    let unix_seconds = clock.anchor_utc_ns / 1_000_000_000;
                    let utc = time::utc_from_unix(unix_seconds as i64);
                    catten_rt::logln!(
                        "[time] synchronized unix_s={} \
                         utc={:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z stratum={} \
                         uncertainty_ms={} drift_ppb={}",
                        unix_seconds,
                        utc.year,
                        utc.month,
                        utc.day,
                        utc.hour,
                        utc.minute,
                        utc.second,
                        clock.anchor_utc_ns % 1_000_000_000,
                        clock.stratum,
                        clock.uncertainty_ns.div_ceil(1_000_000),
                        clock.drift_ppb
                    );
                    if let Some(obj_conn) = obj_conn.as_ref()
                        && !save_calibration(
                            obj_conn.as_ref(),
                            calibration_from_model(clock, server_ipv4),
                        )
                    {
                        config::write::<u32>(status::PERSIST_ERROR, 1);
                    }
                    next_sync_ticks = ticks_after(now_mono, SYNC_INTERVAL_SECONDS);
                }
                Err(()) => {
                    ntp_failures = ntp_failures.wrapping_add(1);
                    config::write::<u32>(status::NTP_FAILURES, ntp_failures);
                    catten_rt::logln!("[time] NTP request failed; failures={}", ntp_failures);
                    next_sync_ticks = ticks_after(now_mono, RETRY_INTERVAL_SECONDS);
                }
            }
        }

        if attempt.is_none() && now_mono.ticks >= next_sync_ticks {
            let has_ipv4 = tcpip_has_ipv4(tcp_conn.as_ref());
            if !has_ipv4 {
                // DHCP can complete just after the boot-ready marker. Do not
                // consume an NTP attempt (and its 64-second retry interval)
                // until the stack has installed a source address and route.
                next_sync_ticks = ticks_after(now_mono, 1);
            } else {
                catten_rt::logln!("[time] requesting NTP sample");
                attempt = start_attempt(tcp_conn.as_ref(), server_ipv4, now_mono, &mut model);
                if attempt.is_none() {
                    ntp_failures = ntp_failures.wrapping_add(1);
                    config::write::<u32>(status::NTP_FAILURES, ntp_failures);
                    catten_rt::logln!("[time] NTP request setup failed; failures={}", ntp_failures);
                    next_sync_ticks = ticks_after(now_mono, RETRY_INTERVAL_SECONDS);
                }
            }
        }

        cq_wait_timeout(1, LOOP_WAIT_MS, 0);
        while unsafe { cq_read(cq.base, cq.entries) }.is_some() {}
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn put_u32_le(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64_le(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn get_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}
