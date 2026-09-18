use ginc::cli::Args;
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn main_without_a_value_succeeds_and_explicit_status_is_preserved() {
    for (source, expected) in [
        ("main:\nreturn\n", 0),
        ("work:\nreturn\n\nmain: work\n", 0),
        ("main:\nreturn 7\n", 7),
    ] {
        let package = TempPackage::new("main_exit_status");
        let input = package.write("main.gin", source);
        let output = package.join("program");
        let mut args = Args {
            input,
            output: Some(output.clone()),
            target: Some(host_target_triple()),
            no_cache: true,
            ..Default::default()
        };
        let result = GinCompiler::compile(&mut args);
        assert!(result.is_success(), "compilation failed: {result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        let status = Command::new(output).status().expect("run main");
        assert_eq!(status.code(), Some(expected), "source: {source}");
    }
}
