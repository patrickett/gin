//! The compiler's new checking path validates both entered and skipped loop paths.
use std::process::{Command, Output};

fn check(source: &str) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.gin");
    std::fs::write(&path, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "check", "--message-format=json"])
        .arg(path)
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .output()
        .unwrap()
}

#[test]
fn a_possibly_skipped_loop_does_not_hide_an_invalid_result() {
    for (parameters, head) in [
        ("flag in 0 to 1", "while flag = 1 do"),
        ("n in 0 to 100", "for i in 1 to n do"),
    ] {
        let source =
            format!("f({parameters}) in 0 to 0:\n    {head}\n        return 0\n    return 1\n");
        let invalid = check(&source);
        assert_eq!(invalid.status.code(), Some(1), "{invalid:?}");
        assert!(
            String::from_utf8_lossy(&invalid.stdout).contains("source-check-"),
            "{invalid:?}"
        );
        let valid = check(&source.replace("return 1", "return 0"));
        assert!(valid.status.success(), "{valid:?}");
    }
}

#[test]
fn a_possibly_skipped_loop_does_not_hide_a_linear_obligation() {
    for (parameters, head) in [
        ("flag in 0 to 1", "while flag = 1 do"),
        ("n in 0 to 100", "for i in 1 to n do"),
    ] {
        let source = format!(
            "Token has\nf({parameters}, t Token):\n    {head}\n        Token := t\n\
             \x20       return\n    return\n"
        );
        let invalid = check(&source);
        assert_eq!(invalid.status.code(), Some(1), "{invalid:?}");
        assert!(
            String::from_utf8_lossy(&invalid.stdout).contains("source-check-unconsumed"),
            "{invalid:?}"
        );
        let valid = check(&source.replace("\n    return\n", "\n    Token := t\n    return\n"));
        assert!(valid.status.success(), "{valid:?}");
    }
}
