#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{TempPackage, host_target_triple};
fn source(target: &str) -> Option<String> {
    if !matches!(
        target,
        "aarch64-apple-darwin" | "aarch64-unknown-linux-gnu" | "x86_64-unknown-linux-gnu"
    ) {
        return None;
    }
    Some(test_fixtures::with_runtime(include_str!(
        "../../../crates/test-fixtures/fixtures/machine_memory_binary_copy.gin"
    )))
}

fn llvm_nm() -> PathBuf {
    for prefix in ["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"] {
        let path = Path::new(prefix).join("llvm-nm");
        if path.exists() {
            return path;
        }
    }
    "llvm-nm".into()
}

#[test]
fn machine_memory_allocates_copies_binary_bytes_and_releases_without_runtime() {
    let target = host_target_triple();
    let Some(source) = source(&target) else {
        return;
    };
    assert!(!source.contains("extern"));
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("machine_memory");
        let executable = package.join("program");
        let mut args = Args {
            input: package.write("main.gin", &source),
            output: Some(executable.clone()),
            target: Some(target.clone()),
            profile,
            no_cache: true,
            gin_home: Some(flask::GinHome::new(package.join("home"))),
            ..Default::default()
        };
        let compilation = GinCompiler::compile(&mut args);
        assert!(compilation.is_success(), "{compilation:?}");
        assert!(!compilation.has_typecheck_diagnostics(), "{compilation:?}");
        let execution = Command::new(&executable).output().unwrap();
        assert!(execution.status.success(), "{execution:?}");
        assert_eq!(execution.stdout, [65, 0, 66, 255, 10]);
        let symbols = Command::new(llvm_nm())
            .arg("--just-symbol-name")
            .arg("--undefined-only")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(symbols.status.success(), "{symbols:?}");
        assert!(
            String::from_utf8_lossy(&symbols.stdout).trim().is_empty(),
            "{symbols:?}"
        );
    }
}
