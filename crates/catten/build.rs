use std::env;

fn main() {
    management_resource();
    telemetry_archive();
    otlp_export();
    // Compile-time policy is trusted Rust build input. Require an absolute path
    // so invocation cwd cannot silently select a different operator profile.
    println!("cargo:rerun-if-env-changed=CATTEN_RESOURCE_POLICY");
    let policy = match env::var_os("CATTEN_RESOURCE_POLICY") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            assert!(path.is_absolute(), "CATTEN_RESOURCE_POLICY must be an absolute file path");
            path
        }
        None => std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("src/resource_policy/defaults.rs"),
    };
    let policy = policy.canonicalize().expect("resource policy file must exist");
    assert!(policy.is_file(), "resource policy must be a file");
    let policy = policy.to_str().expect("resource policy path must be UTF-8");
    assert!(!policy.contains(['\n', '\r']), "resource policy path cannot contain newlines");
    println!("cargo:rerun-if-changed={policy}");
    println!("cargo:rustc-env=CATTEN_RESOURCE_POLICY_FILE={policy}");
    for retired in ["CATTEN_CLUSTER_VIP", "CATTEN_CLUSTER_TCP_PORT", "CATTEN_CLUSTER_SERVICE_NAME"]
    {
        println!("cargo:rerun-if-env-changed={retired}");
        assert!(
            env::var_os(retired).is_none(),
            "{retired} is retired; use CATTEN_CLUSTER_SERVICES=[NAME=]IPv4:PORT[;...]"
        );
    }
    println!("cargo:rerun-if-env-changed=CATTEN_TRUST_MODE");
    match env::var("CATTEN_TRUST_MODE") {
        Err(env::VarError::NotPresent) => {}
        Ok(mode) if mode == "development" => {}
        Ok(mode) if mode == "production" => panic!(
            "production images are disabled: protected trust and recipient-key provisioning is \
             not implemented; development fixtures must not protect real credentials"
        ),
        _ => panic!("CATTEN_TRUST_MODE must be development or production"),
    }
    println!("cargo:rustc-env=CATTEN_TRUST_MODE=development");
    println!("cargo:warning=DEVELOPMENT image: public fixture trust; do not deploy real secrets");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    if env::var_os("CARGO_FEATURE_BOOT_TRUST_TEST").is_some() {
        assert_eq!(arch, "x86_64", "boot_trust_test supports only the x86 QEMU fixture");
        println!(
            "cargo:warning=boot_trust_test uses public test enrollment/state/recipient fixtures"
        );
    }
    boot_test_installed_policy();

    println!("cargo:rerun-if-env-changed=CATTEN_LOCAL_DEPLOYMENT_FIXTURE");
    if env::var_os("CARGO_FEATURE_DEPLOY_NET_TEST").is_some() {
        let path = env::var("CATTEN_LOCAL_DEPLOYMENT_FIXTURE")
            .expect("deploy fixture requires signed descriptor; use the boot runner");
        println!("cargo:rerun-if-changed={path}");
    }

    match arch.as_str() {
        "x86_64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten/linker/x86_64.ld");
            // Re-run when the file changes. Cargo resolves these paths from
            // the manifest directory, so omit the crate prefix.
            println!("cargo:rerun-if-changed=linker/x86_64.ld");
        }
        "aarch64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten/linker/aarch64.ld");
            // Re-run when the file changes. Cargo resolves these paths from
            // the manifest directory, so omit the crate prefix.
            println!("cargo:rerun-if-changed=linker/aarch64.ld");
        }
        "riscv64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten/linker/riscv64.ld");
            // Re-run when the file changes. Cargo resolves these paths from
            // the manifest directory, so omit the crate prefix.
            println!("cargo:rerun-if-changed=linker/riscv64.ld");
        }
        _ => panic!("Invalid ISA"),
    }
}

fn telemetry_archive() {
    const INPUT: &str = "CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS";
    println!("cargo:rerun-if-env-changed={INPUT}");
    let interval = match env::var(INPUT) {
        Err(env::VarError::NotPresent) => charlotte_launch::telemetry::DEFAULT_ARCHIVE_INTERVAL_MS,
        Ok(value) => charlotte_launch::telemetry::parse_archive_interval(&value)
            .expect("CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS must be 0 or 1000..=86400000"),
        Err(_) => panic!("CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS must be ASCII decimal"),
    };
    println!("cargo:rustc-env={INPUT}={interval}");
}

fn otlp_export() {
    use charlotte_launch::otlp;
    for name in ["CATTEN_OTLP_ENDPOINT", "CATTEN_OTLP_INTERVAL_MS", "CATTEN_OTLP_TIMEOUT_MS"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let endpoint = env::var("CATTEN_OTLP_ENDPOINT");
    let bytes = match endpoint {
        Err(env::VarError::NotPresent) => {
            assert!(
                env::var_os("CATTEN_OTLP_INTERVAL_MS").is_none()
                    && env::var_os("CATTEN_OTLP_TIMEOUT_MS").is_none(),
                "OTLP interval/timeout require CATTEN_OTLP_ENDPOINT"
            );
            Vec::new()
        }
        Ok(value) => {
            let number = |key, default| match env::var(key) {
                Err(env::VarError::NotPresent) => default,
                Ok(v) => otlp::decimal(&v).expect("OTLP budgets must be ASCII decimal"),
                Err(_) => panic!("OTLP budgets must be ASCII decimal"),
            };
            otlp::Profile::from_endpoint(
                &value,
                number("CATTEN_OTLP_INTERVAL_MS", otlp::DEFAULT_INTERVAL_MS),
                number("CATTEN_OTLP_TIMEOUT_MS", otlp::DEFAULT_TIMEOUT_MS),
            )
            .expect(
                "CATTEN_OTLP_ENDPOINT requires http://IPv4[:port]/path and bounded \
                 interval/timeout",
            )
            .encode()
            .expect("validated OTLP profile")
        }
        Err(_) => panic!("CATTEN_OTLP_ENDPOINT must be ASCII HTTP/IPv4"),
    };
    let path = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("otlp-profile.bin");
    std::fs::write(&path, &bytes).expect("stage outbound OTLP profile");
    let hash = charlotte_launch::sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    println!("cargo:rustc-env=CATTEN_OTLP_PROFILE_FILE={}", path.display());
    println!("cargo:rustc-env=CATTEN_OTLP_PROFILE_SHA256={hash}");
}

/// The public Secure Boot fixture may compile a different exact installed
/// snapshot. Its candidate module stays separate; this never enables production
/// enrollment or treats a verified successor as an installed boot policy.
fn boot_test_installed_policy() {
    use std::io::Read;

    use charlotte_launch::trust::signed_policy;
    const INPUT: &str = "CATTEN_BOOT_TEST_INSTALLED_POLICY";
    println!("cargo:rerun-if-env-changed={INPUT}");
    let selected = env::var_os(INPUT);
    let fixture = env::var_os("CARGO_FEATURE_BOOT_TRUST_TEST").is_some();
    assert!(selected.is_none() || fixture, "{INPUT} requires the boot_trust_test fixture");
    if !fixture {
        return;
    }
    let path = selected.map_or_else(
        || {
            std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join("src/service/admission/test-policy.bin")
        },
        std::path::PathBuf::from,
    );
    assert!(path.is_absolute(), "{INPUT} must be an absolute file path");
    let path = path.canonicalize().expect("installed fixture snapshot must exist");
    assert!(path.is_file(), "installed fixture snapshot must be a regular file");
    let name = path.to_str().expect("installed fixture snapshot path must be UTF-8");
    assert!(!name.contains(['\n', '\r']), "invalid installed snapshot path");
    let file = std::fs::File::open(&path).expect("open installed fixture snapshot");
    let mut bytes = Vec::new();
    file.take((signed_policy::ENCODED_LEN + 1) as u64)
        .read_to_end(&mut bytes)
        .expect("read bounded installed fixture snapshot");
    signed_policy::signature_digest(&bytes).expect("malformed installed fixture snapshot");
    let staged = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap())
        .join("boot-test-installed-policy.bin");
    std::fs::write(&staged, bytes).expect("stage installed fixture snapshot");
    println!("cargo:rerun-if-changed={name}");
    println!("cargo:rustc-env=CATTEN_BOOT_TEST_INSTALLED_POLICY_FILE={}", staged.display());
}

fn management_resource() {
    use std::io::Read;

    use charlotte_launch::management::sealed;
    println!("cargo:rustc-check-cfg=cfg(catten_management_resource)");
    println!("cargo:rerun-if-env-changed=CATTEN_MANAGEMENT_TLS_RESOURCE");
    let Some(path) = env::var_os("CATTEN_MANAGEMENT_TLS_RESOURCE") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    assert!(path.is_absolute(), "CATTEN_MANAGEMENT_TLS_RESOURCE must be an absolute file path");
    let path = path.canonicalize().expect("management TLS resource must exist");
    let name = path.to_str().expect("management resource path must be UTF-8");
    assert!(!name.contains(['\n', '\r']), "management resource path cannot contain newlines");
    let file = std::fs::File::open(&path).expect("open management TLS resource");
    assert!(
        file.metadata().expect("management resource metadata").is_file(),
        "management TLS resource must be a regular file"
    );
    let mut bytes = Vec::new();
    file.take((sealed::MAX_RESOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .expect("read bounded management TLS resource");
    // A canonical sealed shape is required before embedding; signature/context
    // verification uses the installed boot policy at runtime, never this file.
    sealed::signature_digest(&bytes).expect("malformed management TLS sealed resource");
    println!("cargo:rerun-if-changed={name}");
    let staged = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap())
        .join("management-tls-resource.bin");
    std::fs::write(&staged, bytes).expect("stage sealed management TLS resource");
    println!("cargo:rustc-env=CATTEN_MANAGEMENT_TLS_RESOURCE_FILE={}", staged.display());
    println!("cargo:rustc-cfg=catten_management_resource");
}
