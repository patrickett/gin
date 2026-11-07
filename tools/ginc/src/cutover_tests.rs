use crate::cli::Args;
use crate::compile::{CompileFailure, CompileResult, GinCompiler};

#[test]
fn ordinary_compilation_checks_a_loose_new_source_file_without_an_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.gin");
    std::fs::write(
        &path,
        "#unrestricted\nDigit is in 0 to 9\nidentity(value Digit) Digit: return value\n",
    )
    .unwrap();
    let mut args = Args {
        input: path,
        ..Args::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(
        matches!(result, CompileResult::Success { output: None }),
        "{result:?}"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn automatic_checking_does_not_silently_ignore_a_requested_output() {
    for package in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.gin");
        std::fs::write(&path, "value() in 0 to 9: return 1\n").unwrap();
        let input = if package {
            std::fs::write(
                directory.path().join("mod.flask"),
                "#settings\nmodule: 'app'\nversion: 1.0.0\n",
            )
            .unwrap();
            directory.path().to_owned()
        } else {
            path
        };
        let output = directory.path().join("artifact");
        let result = GinCompiler::compile(&mut Args {
            input,
            output: Some(output.clone()),
            ..Args::default()
        });
        assert!(
            matches!(
                result,
                CompileResult::Failed(CompileFailure::InvalidOptions(_))
            ),
            "{result:?}"
        );
        assert!(!output.exists());
    }
}
