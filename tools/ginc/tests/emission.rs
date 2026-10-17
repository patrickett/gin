use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn library_generated_code_executes_in_debug_and_release() {
    let target = host_target_triple();
    let library = TempPackage::new("instruction_library");
    library.write_flask("circuit");
    library.write(
        "circuit.gin",
        &test_fixtures::emission::buffered_increment_source(&target),
    );
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("emission_consumer");
        let cache = TempPackage::new("emission_cache");
        let input = package.write("main.gin", "use circuit increment, Word\n#entry\nmain:\n    result: increment(41 as Word)\n    numeric is in 0 to 18446744073709551615: result\n    return numeric - 42\n");
        let executable = package.join("program");
        let mut args = Args {
            input,
            output: Some(executable.clone()),
            target: Some(target.clone()),
            profile,
            no_cache: true,
            dependencies: [("circuit".to_string(), library.path().to_path_buf())].into(),
            gin_home: Some(flask::GinHome::new(cache.path().to_path_buf())),
            ..Default::default()
        };
        let result = GinCompiler::compile(&mut args);
        assert!(result.is_success(), "{result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        let output = Command::new(&executable).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
}
