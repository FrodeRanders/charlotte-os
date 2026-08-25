//! Wire protocol for the CharlotteOS UTC time service.
//!
//! The service registers the short name `time`. `OP_NOW` returns a copied
//! [`SNAPSHOT_LEN`]-byte memory object so applications can inspect UTC,
//! calibration quality, and the monotonic clock used to derive it. All
//! integers in the wire representation are little-endian.
#![no_std]

pub const INTERFACE: u64 = u64::from_le_bytes(*b"TIME\0\0\0\0");
pub const VERSION: u32 = 1;
pub const NAME: u64 = u64::from_le_bytes(*b"time\0\0\0\0");

/// Return a [`TimeSnapshot`] in a moved memory object. The scalar result is
/// [`SNAPSHOT_LEN`] on success.
pub const OP_NOW: u32 = 1;
/// Return whole Unix seconds as the scalar result. This call fails until at
/// least persisted holdover state or a network sample is available.
pub const OP_UNIX_SECONDS: u32 = 2;
/// Return a fixed-width ISO 8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ`)
/// in a moved memory object. The scalar result is [`ISO8601_LEN`].
pub const OP_ISO8601: u32 = 3;

pub const ERR_UNSYNCHRONIZED: i64 = -1;
pub const ERR_UNAVAILABLE: i64 = -2;
pub const ERR_BAD_OPCODE: i64 = -3;

pub const STATE_UNSYNCHRONIZED: u8 = 0;
/// Time is advancing from the last persisted calibration, but elapsed
/// powered-off time is unknowable without a fresh network sample.
pub const STATE_HOLDOVER: u8 = 1;
pub const STATE_SYNCHRONIZED: u8 = 2;

pub const SNAPSHOT_MAGIC: u32 = 0x314d_4954; // "TIM1" LE
pub const SNAPSHOT_LEN: usize = 80;
pub const ISO8601_LEN: usize = 30;
pub const NEVER_SYNCED_AGE_MS: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UtcDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeSnapshot {
    pub state: u8,
    pub stratum: u8,
    pub unix_seconds: i64,
    pub nanosecond: u32,
    pub uncertainty_ms: u32,
    /// Estimated correction applied to nominal monotonic-counter time.
    pub drift_ppb: i64,
    pub last_sync_age_ms: u64,
    pub monotonic_ticks: u64,
    pub counter_frequency_hz: u64,
    pub utc: UtcDateTime,
    pub leap_indicator: u8,
    pub server_ipv4: [u8; 4],
    pub sample_count: u32,
}

impl TimeSnapshot {
    pub fn encode(self) -> [u8; SNAPSHOT_LEN] {
        let mut bytes = [0u8; SNAPSHOT_LEN];
        put_u32(&mut bytes, 0, SNAPSHOT_MAGIC);
        put_u16(&mut bytes, 4, VERSION as u16);
        bytes[6] = self.state;
        bytes[7] = self.stratum;
        put_u64(&mut bytes, 8, self.unix_seconds as u64);
        put_u32(&mut bytes, 16, self.nanosecond);
        put_u32(&mut bytes, 20, self.uncertainty_ms);
        put_u64(&mut bytes, 24, self.drift_ppb as u64);
        put_u64(&mut bytes, 32, self.last_sync_age_ms);
        put_u64(&mut bytes, 40, self.monotonic_ticks);
        put_u64(&mut bytes, 48, self.counter_frequency_hz);
        put_u32(&mut bytes, 56, self.utc.year as u32);
        bytes[60] = self.utc.month;
        bytes[61] = self.utc.day;
        bytes[62] = self.utc.hour;
        bytes[63] = self.utc.minute;
        bytes[64] = self.utc.second;
        bytes[65] = self.leap_indicator;
        bytes[68..72].copy_from_slice(&self.server_ipv4);
        put_u32(&mut bytes, 72, self.sample_count);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < SNAPSHOT_LEN
            || get_u32(bytes, 0)? != SNAPSHOT_MAGIC
            || get_u16(bytes, 4)? != VERSION as u16
        {
            return None;
        }
        Some(Self {
            state: bytes[6],
            stratum: bytes[7],
            unix_seconds: get_u64(bytes, 8)? as i64,
            nanosecond: get_u32(bytes, 16)?,
            uncertainty_ms: get_u32(bytes, 20)?,
            drift_ppb: get_u64(bytes, 24)? as i64,
            last_sync_age_ms: get_u64(bytes, 32)?,
            monotonic_ticks: get_u64(bytes, 40)?,
            counter_frequency_hz: get_u64(bytes, 48)?,
            utc: UtcDateTime {
                year: get_u32(bytes, 56)? as i32,
                month: bytes[60],
                day: bytes[61],
                hour: bytes[62],
                minute: bytes[63],
                second: bytes[64],
            },
            leap_indicator: bytes[65],
            server_ipv4: bytes[68..72].try_into().ok()?,
            sample_count: get_u32(bytes, 72)?,
        })
    }
}

/// Convert Unix seconds to a proleptic-Gregorian UTC calendar value.
pub fn utc_from_unix(unix_seconds: i64) -> UtcDateTime {
    let days = unix_seconds.div_euclid(86_400);
    let seconds = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    UtcDateTime {
        year,
        month,
        day,
        hour: (seconds / 3_600) as u8,
        minute: ((seconds % 3_600) / 60) as u8,
        second: (seconds % 60) as u8,
    }
}

/// Format a snapshot as fixed-width ISO 8601 UTC. Years outside 0000..=9999
/// cannot be represented by this compact application ABI.
pub fn iso8601(snapshot: &TimeSnapshot) -> Option<[u8; ISO8601_LEN]> {
    let utc = snapshot.utc;
    if !(0..=9_999).contains(&utc.year)
        || !(1..=12).contains(&utc.month)
        || !(1..=31).contains(&utc.day)
        || utc.hour > 23
        || utc.minute > 59
        || utc.second > 60
        || snapshot.nanosecond >= 1_000_000_000
    {
        return None;
    }
    let mut out = *b"0000-00-00T00:00:00.000000000Z";
    decimal(&mut out[0..4], utc.year as u32);
    decimal(&mut out[5..7], utc.month as u32);
    decimal(&mut out[8..10], utc.day as u32);
    decimal(&mut out[11..13], utc.hour as u32);
    decimal(&mut out[14..16], utc.minute as u32);
    decimal(&mut out[17..19], utc.second as u32);
    decimal(&mut out[20..29], snapshot.nanosecond);
    Some(out)
}

fn decimal(bytes: &mut [u8], mut value: u32) {
    for byte in bytes.iter_mut().rev() {
        *byte = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

// Howard Hinnant's civil-from-days transform, with day zero at 1970-01-01.
fn civil_from_days(days_since_epoch: i64) -> (i32, u8, u8) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 {
        z
    } else {
        z - 146_096
    }
    .div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime
        + if month_prime < 10 {
            3
        } else {
            -9
        };
    year += i64::from(month <= 2);
    (year as i32, month as u8, day as u8)
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn get_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn get_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_calendar_boundaries() {
        assert_eq!(
            utc_from_unix(0),
            UtcDateTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            }
        );
        assert_eq!(utc_from_unix(-1).year, 1969);
        assert_eq!(utc_from_unix(-1).month, 12);
        assert_eq!(utc_from_unix(-1).day, 31);
        assert_eq!(utc_from_unix(951_782_400).day, 29); // 2000-02-29
        assert_eq!(utc_from_unix(1_704_067_199).year, 2023);
        assert_eq!(utc_from_unix(1_704_067_200).year, 2024);
    }

    #[test]
    fn snapshot_round_trips() {
        let snapshot = TimeSnapshot {
            state: STATE_SYNCHRONIZED,
            stratum: 3,
            unix_seconds: 1_777_000_123,
            nanosecond: 456_789_000,
            uncertainty_ms: 12,
            drift_ppb: -19_500,
            last_sync_age_ms: 42,
            monotonic_ticks: 998_877,
            counter_frequency_hz: 1_000_000_000,
            utc: utc_from_unix(1_777_000_123),
            leap_indicator: 0,
            server_ipv4: [162, 159, 200, 1],
            sample_count: 7,
        };
        assert_eq!(TimeSnapshot::decode(&snapshot.encode()), Some(snapshot));
        assert_eq!(iso8601(&snapshot), Some(*b"2026-04-24T03:08:43.456789000Z"));
    }
}
