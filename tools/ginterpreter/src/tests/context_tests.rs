use super::*;
use crate::{InterpreterSession, RuntimeStorage, Submission};
use test_fixtures::TempPackage;

#[test]
fn discovery_without_a_manifest_starts_an_empty_usable_session() {
    let directory = TempPackage::new("interpreter_without_manifest");
    directory.write("unrelated.gin", "broken := @");
    let context = InterpreterContext::discover(directory.path()).unwrap();

    assert!(context.is_isolated());
    assert_eq!(
        context.package().root(),
        directory.path().canonicalize().unwrap()
    );
    assert!(context.package().sources().is_empty());
    assert!(context.package().dependencies().is_empty());

    let mut session = InterpreterSession::with_context(context);
    let Submission::Evaluated { value, .. } = session.submit("40 + 2") else {
        panic!("an isolated session should evaluate known inputs");
    };
    assert_eq!(value.storage, RuntimeStorage::Int64(42));
}

#[test]
fn discovery_loads_a_manifest_from_a_parent_directory() {
    let package = TempPackage::new("interpreter_parent_manifest");
    package.write_flask("parent_manifest");
    let source = package.write("nested/definitions.gin", "PackageTag is Unit\n");
    let context = InterpreterContext::discover(source.parent().unwrap()).unwrap();

    assert!(!context.is_isolated());
    assert_eq!(
        context.package().root(),
        package.path().canonicalize().unwrap()
    );
    assert_eq!(context.package().sources().len(), 1);
    assert_eq!(
        context.package().sources()[0].path,
        source.canonicalize().unwrap()
    );
}

#[test]
fn discovery_supports_mod_flask() {
    let package = TempPackage::new("interpreter_mod_manifest");
    package.write(
        "mod.flask",
        "#settings\nmodule: 'interpreter_mod_manifest'\nversion: 0.1.0\ntarget: 'library'\n",
    );
    let context = InterpreterContext::discover(package.path()).unwrap();

    assert!(!context.is_isolated());
    assert!(matches!(context.target(), CompileTarget::Concrete(_)));
}

#[test]
fn discovery_preserves_path_dependencies() {
    let package = TempPackage::new("interpreter_dependency_manifest");
    package.write_flask_with_deps(
        "app",
        r#"core: "./dependencies/core"
"#,
    );
    package.write_nested_flask("dependencies/core", "core");
    let context = InterpreterContext::discover(package.path()).unwrap();

    assert!(!context.is_isolated());
    assert_eq!(
        context.package().dependencies()["core"]
            .canonicalize()
            .unwrap(),
        package.join("dependencies/core").canonicalize().unwrap(),
    );
}

#[test]
fn discovery_rejects_a_malformed_manifest_before_a_valid_parent() {
    let package = TempPackage::new("interpreter_invalid_child_manifest");
    package.write_flask("valid_parent");
    let manifest = package.write("nested/mod.flask", "invalid");

    for result in [
        InterpreterContext::discover(manifest.parent().unwrap()),
        InterpreterContext::load(manifest.parent().unwrap()),
    ] {
        let Err(InterpreterContextError::InvalidPackage(message)) = result else {
            panic!("invalid child manifest must reject package loading");
        };
        assert!(message.contains("nested/mod.flask"), "{message}");
    }
}

#[test]
fn discovery_ignores_legacy_manifests() {
    let package = TempPackage::new("interpreter_legacy_manifests");
    package.write_flask("current");
    package.write("flask.jsonc", "invalid");
    let context = InterpreterContext::discover(package.path()).unwrap();
    assert!(context.reload().is_ok());
}

#[test]
fn isolated_context_ignores_existing_manifests_and_stays_isolated_on_reload() {
    let package = TempPackage::new("interpreter_explicit_isolation");
    package.write("mod.flask", "invalid");
    package.write("broken.gin", "broken := @");
    let context = InterpreterContext::isolated_at(package.path()).unwrap();
    let reloaded = context.reload().unwrap();

    assert!(context.is_isolated());
    assert!(reloaded.is_isolated());
    assert_eq!(
        reloaded.package().root(),
        package.path().canonicalize().unwrap()
    );
    assert!(reloaded.package().sources().is_empty());
}

#[test]
fn discovery_of_a_file_without_a_manifest_uses_its_directory() {
    let directory = TempPackage::new("interpreter_file_without_manifest");
    let source = directory.write("example.gin", "broken := @");
    let context = InterpreterContext::discover(&source).unwrap();

    assert!(context.is_isolated());
    assert_eq!(
        context.package().root(),
        directory.path().canonicalize().unwrap()
    );
    assert!(context.package().sources().is_empty());
}

#[test]
fn strict_loading_still_requires_a_manifest() {
    let directory = TempPackage::new("interpreter_strict_loading");

    assert!(matches!(
        InterpreterContext::load(directory.path()),
        Err(InterpreterContextError::PackageNotFound(_)),
    ));
}

#[test]
fn nonexistent_paths_do_not_silently_start_isolated_sessions() {
    let directory = TempPackage::new("interpreter_nonexistent_path");
    let path = directory.join("missing");

    assert!(matches!(
        InterpreterContext::discover(&path),
        Err(InterpreterContextError::InvalidPath { .. }),
    ));
    assert!(matches!(
        InterpreterContext::isolated_at(&path),
        Err(InterpreterContextError::InvalidPath { .. }),
    ));
}
