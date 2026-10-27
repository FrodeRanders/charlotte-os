//! Complete limits files reject before credentials are opened or output created.
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use charlotte_launch::management::{
    limit_values,
    Limits,
    LIMIT_NAMES,
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "charlotte-management-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cluster-sign"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn defaults_are_complete_and_invalid_limit_files_reject_before_private_input() {
    let fixture = Fixture::new();
    assert!(fixture.run(&["management-limits-default", "limits"]).status.success());
    let defaults = fs::read_to_string(fixture.0.join("limits")).unwrap();
    assert_eq!(
        defaults,
        LIMIT_NAMES
            .iter()
            .zip(limit_values(Limits::DEFAULT))
            .map(|(name, value)| format!("{name}={value}\n"))
            .collect::<String>()
    );
    let cases = [
        defaults.lines().skip(1).map(|line| format!("{line}\n")).collect::<String>(),
        format!("{defaults}max_roots=8\n"),
        format!("{defaults}unknown=1\n"),
        defaults.replace("max_roots=8", "max_roots=0"),
        defaults.replace("max_roots=8", "max_roots=+8"),
        defaults.replace("max_roots=8", "max_roots=18446744073709551616"),
        defaults.replace("handshake_ms=5000", "handshake_ms=60001"),
        defaults.replace("max_output_bytes=65536", "max_output_bytes=10"),
    ];
    for text in cases {
        fs::write(fixture.0.join("bad"), text).unwrap();
        let result = fixture.run(&[
            "management-profile-create",
            "output",
            "bad",
            "missing-private",
            "chain=missing",
            "root=missing",
            "reader=missing",
        ]);
        assert!(!result.status.success());
        let message = String::from_utf8(result.stderr).unwrap();
        assert!(message.contains("limit"), "{message}");
        assert!(!message.contains("read management key"), "{message}");
        assert!(!fixture.0.join("output").exists());
    }
}
