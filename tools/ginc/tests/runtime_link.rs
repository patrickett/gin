#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use test_fixtures::{TempPackage, host_target_triple};

fn llvm_tool(name: &str) -> PathBuf {
    for prefix in ["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"] {
        let path = Path::new(prefix).join(name);
        if path.exists() {
            return path;
        }
    }
    PathBuf::from(name)
}

#[test]
fn assembly_helpers_and_local_definitions_link_without_runtime_c() {
    let package = TempPackage::new("runtime_free_link");
    let wrapper = package.write(
        "cc-wrapper",
        r#"#!/bin/sh
previous=''
for argument in "$@"; do
    if [ "$previous" = '-x' ] && [ "$argument" = 'c' ]; then
        echo 'unexpected runtime C compilation' >&2
        exit 91
    fi
    previous="$argument"
done
exec "$GIN_TEST_REAL_CC" "$@"
"#,
    );
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    for (name, source, expected) in [
        (
            "unused_assembly",
            r#"Word is in -9223372036854775808 to 9223372036854775807
Word has
    finish(self):
        return
runtime_print_int(value Word) Word: return runtime_print(value as RuntimeInt) as Word
#entry
main: 0
"#,
            0,
        ),
        (
            "local_definition",
            "Word is in -9223372036854775808 to 9223372036854775807\nWord has\n    finish(self):\n        return\nruntime_print_int(value Word) Word: return value\n#entry\nmain: runtime_print_int(23)\n",
            23,
        ),
    ] {
        let input = package.write(&format!("{name}.gin"), &test_fixtures::with_runtime(source));
        for profile in ["debug", "release"] {
            let executable = package.join(format!("{name}-{profile}"));
            let compilation = Command::new(env!("CARGO_BIN_EXE_ginc"))
                .arg(&input)
                .args(["--target", &host_target_triple(), "--profile", profile])
                .args(["--no-cache", "-o"])
                .arg(&executable)
                .env("GIN_HOME", package.join("home"))
                .env("CC", &wrapper)
                .env("GIN_TEST_REAL_CC", llvm_tool("clang"))
                .output()
                .unwrap();
            assert!(
                compilation.status.success(),
                "{name}/{profile}: {}",
                String::from_utf8_lossy(&compilation.stderr)
            );
            let process = Command::new(&executable).output().unwrap();
            assert_eq!(process.status.code(), Some(expected));
            assert!(process.stdout.is_empty());
            let symbols = Command::new(llvm_tool("llvm-nm"))
                .args(["--undefined-only", "--just-symbol-name"])
                .arg(&executable)
                .output()
                .unwrap();
            assert!(symbols.status.success(), "{symbols:?}");
            assert!(
                !String::from_utf8_lossy(&symbols.stdout).contains("runtime_"),
                "{symbols:?}"
            );
        }
    }
}
