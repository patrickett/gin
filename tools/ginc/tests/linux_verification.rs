//! Runs the machine-code system-call blocks of `modules/core` and `modules/terminal` for real:
//! darwin on this host, Linux (aarch64 and x86_64) in an `alpine:3` container, through
//! `scripts/verify-linux/verify.py`.
//!
//! Opt-in (it takes about two minutes and needs docker): set `GIN_VERIFY_LINUX=1`. Without it, or
//! without docker, the toolchain or the image, this test prints why and passes. It never pulls.
use std::path::Path;
use std::process::Command;

#[test]
fn syscall_blocks_do_what_gin_says() {
    if std::env::var_os("GIN_VERIFY_LINUX").is_none() {
        eprintln!(
            "skipped: set GIN_VERIFY_LINUX=1 to run scripts/verify-linux/verify.py (needs docker)"
        );
        return;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/verify-linux/verify.py");
    let output = Command::new("python3")
        .arg("-I")
        .arg(&script)
        .env("GINC", env!("CARGO_BIN_EXE_ginc"))
        .output()
        .expect("python3 is needed to run the verification script");
    let text = String::from_utf8_lossy(&output.stdout);
    eprintln!("{text}{}", String::from_utf8_lossy(&output.stderr));
    match output.status.code() {
        Some(0) => {}
        Some(77) => eprintln!("skipped: nothing could run (see the SKIP lines above)"),
        other => panic!(
            "a syscall block is wrong or a negative control is invalid: exit {other:?}\n{text}"
        ),
    }
}
