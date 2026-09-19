use ginc::cli::{Args, Emit, Profile};
use ginc::compile::{CompileResult, GinCompiler};

mod common;
use common::cra;

#[test]
fn concrete_target_cli_preserves_wide_compile_time_integer_constants() {
    let package = test_fixtures::TempPackage::new("wide_integer_constants");
    let count = test_fixtures::semantic_fixtures::COUNT_GIN
        .lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n");
    let input = package.write("main.gin", &format!("Byte is in 0 to 255\n{count}"));
    let output = std::env::temp_dir().join(format!(
        "gin-count-{}-{}.ginif",
        std::process::id(),
        std::thread::current().name().unwrap_or("target")
    ));
    let mut args = Args {
        input,
        output: Some(output.clone()),
        target: Some(test_fixtures::host_target_triple()),
        emit: Emit::Interface,
        profile: Profile::Debug,
        no_cache: true,
        gin_home: None,
        timings: false,
        timings_json: None,
        quiet: false,
        fast_llvm: false,
        verbose: None,
        dependencies: Default::default(),
    };

    let result = GinCompiler::compile(&mut args);
    let _ = std::fs::remove_file(&output);

    assert!(
        matches!(result, CompileResult::Success { .. }),
        "{result:?}"
    );
}

#[test]
fn alignment_runtime_constructor_rejects_zero() {
    assert_alignment_invalid("0", "alignment_zero_invalid");
}

#[test]
fn alignment_runtime_constructor_rejects_non_power_of_two() {
    assert_alignment_invalid("3", "alignment_non_power_invalid");
}

fn assert_alignment_invalid(candidate: &str, test_name: &str) {
    let count = test_fixtures::semantic_fixtures::COUNT_GIN;
    let count = count.lines().skip(1).collect::<Vec<_>>().join("\n");
    let source = format!(
        "Byte is in 0 to 255\n{count}\n\nmain:\n    if Alignment.try_new({candidate}) is Invalid\n        0\n    return\nreturn 1\n"
    );

    cra(test_name, &source)
        .assert_compiled()
        .assert_exit_code(0);
}
