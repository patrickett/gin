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
            ("/virtual/main.gin", "first:\nreturn"),
            ("/virtual/main.gin", "second:\nreturn"),
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
            ("/virtual/main.gin", "main:\nreturn"),
            ("/virtual/helper.gin", "first:\nreturn"),
            ("/virtual/helper.gin", "second:\nreturn"),
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
        vec![("/virtual/unknown.gin", "main:\nreturn missing")],
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
    let root = std::env::temp_dir().join(format!(
        "gin_driver_cache_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    let core = root.join("core");
    fs::create_dir_all(&core).unwrap();
    fs::write(core.join("flask.jsonc"), "{}").unwrap();
    let main_path = root.join("main.gin");
    let int_path = core.join("int.gin");
    let main_source = "use core.Int\n";
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
                ("/virtual/provider.gin", "provider:\nreturn"),
                ("/virtual/consumer.gin", "consumer:\nreturn"),
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
