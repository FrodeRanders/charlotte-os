//! Launch policy for diagnostic telemetry persistence, independent of sampling.

pub const ARCHIVE_INTERVAL_MS_KEY: u64 = crate::manifest_key(b"arch-ms");
pub const DEFAULT_ARCHIVE_INTERVAL_MS: u64 = 0;
pub const MIN_ARCHIVE_INTERVAL_MS: u64 = 1_000;
pub const MAX_ARCHIVE_INTERVAL_MS: u64 = 86_400_000;

pub const fn valid_archive_interval(milliseconds: u64) -> bool {
    milliseconds == 0
        || (milliseconds >= MIN_ARCHIVE_INTERVAL_MS && milliseconds <= MAX_ARCHIVE_INTERVAL_MS)
}

/// Zero disables archiving; an enabled interval is between one second and a day.
pub fn parse_archive_interval(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let milliseconds = value.parse().ok()?;
    valid_archive_interval(milliseconds).then_some(milliseconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_policy_is_explicit_and_bounded() {
        for value in ["0", "1000", "10000", "60000", "86400000"] {
            assert!(parse_archive_interval(value).is_some(), "{value}");
        }
        for value in [
            "",
            "on",
            "-1",
            "+1000",
            " 1000",
            "1000 ",
            "1",
            "999",
            "86400001",
            "18446744073709551616",
        ] {
            assert_eq!(parse_archive_interval(value), None, "{value}");
        }
        assert_eq!(DEFAULT_ARCHIVE_INTERVAL_MS, 0);
    }
}
