mod common;
use common::compile_exe;
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn main_without_a_value_succeeds_and_explicit_status_is_preserved() {
    for (source, expected) in [
        ("#entry\nmain:\n    return\n", 0),
        ("work:\n    return\n\n#entry\nmain: work\n", 0),
        ("#entry\nmain:\n    return 7\n", 7),
        ("#entry\nlaunch:\n    return 11\n", 11),
    ] {
        let package = TempPackage::new("main_exit_status");
        let input = package.write("main.gin", source);
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), Profile::Debug);
        assert!(result.is_success(), "compilation failed: {result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        let status = Command::new(output).status().expect("run main");
        assert_eq!(status.code(), Some(expected), "source: {source}");
    }
}
