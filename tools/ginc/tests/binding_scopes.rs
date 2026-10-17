mod common;
use common::compile_exe;
use std::process::Command;

use ginc::cli::Profile;
use ginc::compile::{CompileFailure, CompileResult};
use test_fixtures::TempPackage;

#[test]
fn shadowing_stops_compilation_before_emitting_an_artifact() {
    for source in [
        "#entry\nmain:\n    value: 1\n    value: 2\n    return value\n",
        "choose(value is in 0 to 255):\n    value := 2\n    return value\n#entry\nmain: choose(1)\n",
        "#entry\nmain:\n    value := 1\n    for item in []\n        value: 2\n    return value\n",
        "identity(value):\n    value: 2\n    return value\n#entry\nmain: 0\n",
    ] {
        for profile in [Profile::Debug, Profile::Release] {
            let package = TempPackage::new("rejected_shadowing");
            package.write_flask("rejected_shadowing");
            let input = package.write("main.gin", source);
            let output = package.join("program");
            let result = compile_exe(input, &output.clone(), profile);
            assert!(
                matches!(
                    result,
                    CompileResult::Failed(CompileFailure::TypecheckFailed)
                ),
                "{profile:?}: {result:?}"
            );
            assert!(!output.exists());
        }
    }
}

#[test]
fn disjoint_scopes_and_explicit_updates_execute_in_both_profiles() {
    let source = "choose(flag is in 0 to 1):\n    when flag = 0 then\n        value := 7\n        return value\n    value := 9\n    return value\n#entry\nmain:\n    total is in 0 to 255: 0\n    for value in 0 to 1\n        total :: choose(value)\n    for value in [2, 3]\n        observed: value\n    value := 33\n    return total + value\n";
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("disjoint_binding_scopes");
        package.write_flask("disjoint_binding_scopes");
        let input = package.write("main.gin", source);
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), profile);
        assert!(
            result.is_success() && !result.has_typecheck_diagnostics(),
            "{profile:?}: {result:?}"
        );
        assert_eq!(Command::new(output).status().unwrap().code(), Some(42));
    }
}
