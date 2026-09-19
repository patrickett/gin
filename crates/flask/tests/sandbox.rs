#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sandbox_denies_ungranted_reads_and_writes_and_isolates_home() {
    use flask::ExecutionSandbox;
    use std::path::Path;
    let private = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let secret = private.path().join("credential");
    let attack = private.path().join("injected");
    std::fs::write(&secret, "credential-value\n").unwrap();
    let mut command = ExecutionSandbox::default()
        .command(Path::new("/bin/bash"), scratch.path())
        .unwrap();
    let result = command.arg("-c").arg("if IFS= read -r value < \"$1\"; then exit 31; fi; if (echo injected > \"$2\"); then exit 32; fi; printf '%s' \"$HOME\"; echo ok > local-result").arg("sh").arg(&secret).arg(&attack).output().unwrap();
    assert!(
        result.status.success(),
        "{:?}: {}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        scratch.path().canonicalize().unwrap().to_string_lossy()
    );
    assert!(!attack.exists());
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("local-result")).unwrap(),
        "ok\n"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sandbox_honors_explicit_filesystem_grants() {
    use flask::ExecutionSandbox;
    use std::path::Path;
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let source = input.path().join("data");
    let destination = output.path().join("result");
    std::fs::write(&source, "allowed\n").unwrap();
    let sandbox = ExecutionSandbox::default()
        .with_read(input.path().to_path_buf())
        .with_write(output.path().to_path_buf());
    let result = sandbox
        .command(Path::new("/bin/bash"), scratch.path())
        .unwrap()
        .arg("-c")
        .arg("IFS= read -r value < \"$1\" && printf '%s' \"$value\" > \"$2\"")
        .arg("sh")
        .arg(&source)
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{:?}: {}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read_to_string(destination).unwrap(), "allowed");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sandbox_network_probe() {
    let Ok(address) = std::env::var("GIN_SANDBOX_NETWORK_PROBE") else {
        return;
    };
    let connected = std::net::TcpStream::connect_timeout(
        &address.parse().unwrap(),
        std::time::Duration::from_secs(2),
    )
    .is_ok();
    assert_eq!(
        connected,
        std::env::var("GIN_SANDBOX_EXPECT_NETWORK").unwrap() == "yes"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sandbox_network_requires_an_explicit_grant() {
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap().to_string();
    let scratch = tempfile::tempdir().unwrap();
    for allowed in [false, true] {
        let result = flask::ExecutionSandbox::default()
            .with_network(allowed)
            .command(&std::env::current_exe().unwrap(), scratch.path())
            .unwrap()
            .args(["--exact", "sandbox_network_probe", "--nocapture"])
            .env("GIN_SANDBOX_NETWORK_PROBE", &address)
            .env(
                "GIN_SANDBOX_EXPECT_NETWORK",
                if allowed { "yes" } else { "no" },
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
