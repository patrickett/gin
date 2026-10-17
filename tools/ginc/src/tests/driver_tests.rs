use super::*;
use std::fs;

fn source_package(primary: &str, sources: Vec<(&str, &str)>) -> SourcePackage {
    SourcePackage::new(
        primary,
        sources
            .into_iter()
            .map(|(path, source)| SourceFile::new(path, source))
            .collect(),
    )
}

fn check(driver: &mut CompilerDriver, primary: &str, sources: Vec<(&str, &str)>) -> CheckedPackage {
    let parsed = driver
        .parse(source_package(primary, sources))
        .expect("valid primary source");
    driver.check(CompilePackage::new(parsed, CompileTarget::Library))
}

#[test]
fn todo_holes_block_codegen_even_in_unselected_constant_branches() {
    let source = concat!(
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\n",
        "selected Word := when #target.arch is 'x86_64' then 42 else #todo\n",
    );
    let mut driver = CompilerDriver::default();
    let parsed = driver
        .parse(source_package(
            "/virtual/main.gin",
            vec![("/virtual/main.gin", source)],
        ))
        .unwrap();
    let checked = driver.check(CompilePackage::new(
        parsed,
        CompileTarget::Concrete(flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap()),
    ));
    assert!(!checked.has_front_end_flaws());
    assert!(
        !checked.has_typecheck_flaws(),
        "{:?}",
        checked.primary_file().typecheck_diagnostics()
    );
    assert!(checked.has_holes());
    assert!(
        checked
            .primary_file()
            .typed()
            .exprs
            .kind
            .iter()
            .all(|kind| !matches!(kind, typecheck::TypedExprKind::Todo))
    );
    let context = codegen::NativeCompiler::create_context();
    let lowered = driver.lower_package(&context, &checked);
    assert!(lowered.module.is_none());
    let diagnostic = &lowered.diagnostics[0].diagnostics[0];
    assert_eq!(diagnostic.code.slug().as_str(), "unfilled-hole");
    assert_eq!(diagnostic.category, Category::Flaw);
    assert_eq!(&source[diagnostic.span.to_range()], "#todo");
}

#[test]
fn parse_rejects_a_missing_primary_source() {
    let error = match CompilerDriver::default().parse(source_package(
        "/virtual/missing.gin",
        vec![("/virtual/other.gin", "Other has Value")],
    )) {
        Ok(_) => panic!("missing primary source should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        CompilerDriverError::PrimarySourceNotFound { path }
            if path == Path::new("/virtual/missing.gin")
    ));
}

#[test]
fn parse_rejects_a_duplicate_primary_source_path() {
    let error = match CompilerDriver::default().parse(source_package(
        "/virtual/main.gin",
        vec![
            ("/virtual/main.gin", "first:\n    return"),
            ("/virtual/main.gin", "second:\n    return"),
        ],
    )) {
        Ok(_) => panic!("duplicate primary source path should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        CompilerDriverError::DuplicateSourcePath { path }
            if path == Path::new("/virtual/main.gin")
    ));
}

#[test]
fn parse_rejects_a_duplicate_non_primary_source_path() {
    let error = match CompilerDriver::default().parse(source_package(
        "/virtual/main.gin",
        vec![
            ("/virtual/main.gin", "main:\n    return"),
            ("/virtual/helper.gin", "first:\n    return"),
            ("/virtual/helper.gin", "second:\n    return"),
        ],
    )) {
        Ok(_) => panic!("duplicate non-primary source path should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        CompilerDriverError::DuplicateSourcePath { path }
            if path == Path::new("/virtual/helper.gin")
    ));
}

#[test]
fn parse_reports_flaws_before_semantic_checking() {
    let parsed = CompilerDriver::default()
        .parse(source_package(
            "/virtual/broken.gin",
            vec![("/virtual/broken.gin", "broken := @")],
        ))
        .expect("valid primary source");

    assert!(parsed.has_flaws());
    assert!(
        parsed.files()[0]
            .output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.error_code().starts_with("parse-"))
    );
}

#[test]
fn checked_files_keep_their_typed_ast_mapping() {
    let mut driver = CompilerDriver::default();
    let checked = check(
        &mut driver,
        "/virtual/right.gin",
        vec![
            ("/virtual/left.gin", "Left has Value"),
            ("/virtual/right.gin", "Right has Value"),
        ],
    );

    let left = checked.file(Path::new("/virtual/left.gin")).unwrap();
    let right = checked.file(Path::new("/virtual/right.gin")).unwrap();
    assert!(left.typed().tags.keys().any(|tag| tag.0.as_str() == "Left"));
    assert!(
        right
            .typed()
            .tags
            .keys()
            .any(|tag| tag.0.as_str() == "Right")
    );
    assert_eq!(checked.primary_file().path(), right.path());
}

#[test]
fn front_end_diagnostics_are_attached_to_their_source() {
    let mut driver = CompilerDriver::default();
    let checked = check(
        &mut driver,
        "/virtual/broken.gin",
        vec![("/virtual/broken.gin", "broken := @")],
    );

    let file = checked.primary_file();
    assert!(file.has_front_end_flaws());
    assert!(
        file.front_end_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.error_code().starts_with("parse-"))
    );
}

#[test]
fn typecheck_diagnostics_are_attached_to_their_source() {
    let mut driver = CompilerDriver::default();
    let checked = check(
        &mut driver,
        "/virtual/unknown.gin",
        vec![("/virtual/unknown.gin", "main:\n    return missing")],
    );

    let file = checked.primary_file();
    assert!(file.has_typecheck_flaws());
    assert!(
        file.typecheck_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.error_code().starts_with("type-"))
    );
}

#[test]
fn driver_reuses_cached_imports_between_checks() {
    let root = test_fixtures::unique_temp_dir("driver_cache");
    let core = root.join("core");
    fs::create_dir_all(&core).unwrap();
    fs::write(
        core.join("mod.flask"),
        "#settings\nmodule: 'core'\nversion: 0.0.0\n",
    )
    .unwrap();
    let main_path = root.join("main.gin");
    let int_path = core.join("int.gin");
    let main_source = "use core Int\n";
    fs::write(&main_path, main_source).unwrap();
    fs::write(&int_path, "Int is Unit\n").unwrap();
    let dependencies = HashMap::from([("core".to_string(), core)]);
    let mut driver = CompilerDriver::default();

    let parsed = driver
        .parse(SourcePackage::new(
            main_path.clone(),
            vec![SourceFile::new(&main_path, main_source)],
        ))
        .unwrap();
    let first = driver.check(
        CompilePackage::new(parsed, CompileTarget::Library)
            .with_dependencies(dependencies.clone())
            .with_import_resolution(true),
    );
    assert!(first.file(&int_path).is_some());

    fs::remove_file(&int_path).unwrap();
    let parsed = driver
        .parse(SourcePackage::new(
            main_path.clone(),
            vec![SourceFile::new(&main_path, main_source)],
        ))
        .unwrap();
    let second = driver.check(
        CompilePackage::new(parsed, CompileTarget::Library)
            .with_dependencies(dependencies)
            .with_import_resolution(true),
    );

    assert!(second.file(&int_path).is_some());
    assert!(!second.has_front_end_flaws());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lowering_builds_one_module_from_every_checked_file() {
    let mut driver = CompilerDriver::default();
    let parsed = driver
        .parse(source_package(
            "/virtual/consumer.gin",
            vec![
                ("/virtual/provider.gin", "provider:\n    return"),
                ("/virtual/consumer.gin", "consumer:\n    return"),
            ],
        ))
        .expect("valid primary source");
    let target = flask::TargetTriple::parse("x86_64-unknown-linux-gnu").expect("test target");
    let checked = driver.check(CompilePackage::new(parsed, CompileTarget::Concrete(target)));
    let context = codegen::NativeCompiler::create_context();
    let lowered = driver.lower_package(&context, &checked);
    let mlir = lowered.module.unwrap().as_operation().to_string();

    assert!(mlir.contains("@provider"));
    assert!(mlir.contains("@consumer"));
    assert_eq!(lowered.diagnostics.len(), 2);
}

#[test]
fn unresolved_builder_chain_is_lowered_once_and_preserves_independent_errors() {
    for length in [2, 8, 16] {
        let chain = ".missing_step('argument')".repeat(length);
        let source = format!(
            "{}Builder has\n    value Word\n    finish(self):\n        return\nstart() Builder: return Builder(value: 1)\nresult: start(){chain}\nother: independent_missing\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        let package = check(
            &mut CompilerDriver::default(),
            "/virtual/chain.gin",
            vec![("/virtual/chain.gin", &source)],
        );
        let file = package.primary_file();
        assert!(
            !package.has_front_end_flaws(),
            "{:?}",
            file.front_end_diagnostics()
        );
        let errors = file.typecheck_diagnostics();
        assert_eq!(
            errors
                .iter()
                .filter(|d| d.arg("name") == Some("missing_step"))
                .count(),
            length,
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|d| d.arg("name") == Some("independent_missing")),
            "{errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|d| d.code.slug() == "type-text-literal-requires-context"),
            "{errors:?}"
        );
        assert!(file.typed().exprs.kind.len() < length * 4 + 20);
        for error in errors
            .iter()
            .filter(|d| d.arg("name") == Some("missing_step"))
        {
            assert_eq!(
                &source[error.span.start()..error.span.end()],
                "missing_step"
            );
        }
    }
}

#[test]
fn valid_builder_chain_retains_receiver_types_and_argument_diagnostics() {
    let source = format!(
        "{}Builder has\n    value Word\n    finish(self):\n        return\nBuilder.new() Builder: return Builder(value: 1)\nBuilder.step(self) Builder: return Builder(value: self.value)\nresult: Builder.new().step().step()\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let package = check(
        &mut CompilerDriver::default(),
        "/virtual/valid.gin",
        vec![("/virtual/valid.gin", source.as_str())],
    );
    assert!(
        !package.has_typecheck_flaws(),
        "{:?}",
        package.primary_file().typecheck_diagnostics()
    );
    let source = "result: missing_function(missing_argument, 'text')\nother: 'needs context'\n";
    let package = check(
        &mut CompilerDriver::default(),
        "/virtual/invalid.gin",
        vec![("/virtual/invalid.gin", source)],
    );
    let errors = package.primary_file().typecheck_diagnostics();
    for name in ["missing_function", "missing_argument"] {
        assert!(
            errors.iter().any(|d| d.arg("name") == Some(name)),
            "{errors:?}"
        );
    }
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn checked_primary_and_file_lookup_survive_import_path_normalization() {
    let package = test_fixtures::TempPackage::new("primary_path_normalization");
    package.write_flask("fixture");
    package.write("main.gin", "main: 42\n");
    let original = package.join("./main.gin");
    let canonical = original.canonicalize().unwrap();
    let mut driver = CompilerDriver::default();
    let parsed = driver
        .parse(SourcePackage::new(
            &original,
            vec![SourceFile::new(&original, "main: 42\n")],
        ))
        .unwrap();
    let checked = driver
        .check(CompilePackage::new(parsed, CompileTarget::Library).with_import_resolution(true));
    assert_eq!(checked.primary_path(), canonical);
    assert_eq!(checked.file(&original).unwrap().path(), canonical);
    assert!(!checked.has_front_end_flaws());
    assert!(!checked.has_typecheck_flaws());
}
