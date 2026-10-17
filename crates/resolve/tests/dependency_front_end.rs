use std::collections::HashMap;

use ast::ty::PackageInstanceKey;
use flask::CompileTarget;
use resolve::{PackageFrontEndOptions, PackageFrontEndOutput, ParsedFile, ParsedModuleCache};
use test_fixtures::TempPackage;
use typecheck::transform::{BodyReuse, PackageCheckSession, PackageTransformOptions};

fn options() -> PackageFrontEndOptions {
    PackageFrontEndOptions {
        gin_home: None,
        compile_target: CompileTarget::Library,
        transform: PackageTransformOptions::FULL,
        resolve_imports: true,
        package_fallback: PackageInstanceKey::workspace("fixture", "0"),
        transform_paths: None,
    }
}

fn equivalent(left: &PackageFrontEndOutput, right: &PackageFrontEndOutput) {
    assert_eq!(left.public_interface, right.public_interface);
    assert_eq!(left.files.len(), right.files.len());
    for (left, right) in left.files.iter().zip(&right.files) {
        assert_eq!(left.parsed.path, right.parsed.path);
        assert_eq!(left.parsed.output.symptoms, right.parsed.output.symptoms);
        assert_eq!(left.import_diagnostics, right.import_diagnostics);
        assert_eq!(left.prepare_diagnostics, right.prepare_diagnostics);
        assert_eq!(left.typed, right.typed);
        let left = left.typed.as_ref().unwrap();
        let right = right.typed.as_ref().unwrap();
        assert_eq!(left.type_registry, right.type_registry);
        assert_eq!(left.places, right.places);
        assert_eq!(left.place_versions, right.place_versions);
        assert_eq!(
            left.place_version_components,
            right.place_version_components
        );
        assert_eq!(left.constant_objects, right.constant_objects);
        assert_eq!(left.all_flaws(), right.all_flaws());
    }
}

#[test]
fn imports_and_file_identity_are_independent_of_entry_processing_order() {
    let package = TempPackage::new("dependency-front-end-order");
    package.write_flask("fixture");
    package.write("values/values.gin", test_fixtures::preludes::WORD_PRELUDE);
    package.write(
        "math/math.gin",
        "use '../values' Word\nidentity(value Word) Word: return value\n",
    );
    package.write(
        "main.gin",
        "use './math' identity\nuse './values' Word\nmain(value Word) Word: return identity(value)\n",
    );
    let root = package.path().canonicalize().unwrap();
    let files = ["main.gin", "math/math.gin", "values/values.gin"]
        .map(|path| ParsedFile::read(&root.join(path)).unwrap());
    let expected = resolve::run_package_front_end(
        files.to_vec(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
    );
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for workers in [1, 2, 4, 8] {
            let mut session =
                PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
                    .unwrap();
            let mut cache = ParsedModuleCache::default();
            for _ in 0..2 {
                let output = resolve::run_package_front_end_with_session(
                    order.map(|index| files[index].clone()).to_vec(),
                    &HashMap::new(),
                    cache,
                    options(),
                    &mut session,
                );
                equivalent(&expected, &output);
                cache = output.cache;
            }
        }
    }
}

#[test]
fn import_edits_and_compile_time_implementation_changes_match_clean_resolution() {
    let package = TempPackage::new("dependency-front-end-edits");
    package.write_flask("fixture");
    package.write(
        "math/math.gin",
        &format!(
            "{}compute(value is in Word) is in Word: return 1\n",
            test_fixtures::preludes::WORD_PRELUDE
        ),
    );
    package.write("main.gin", "use './math' compute\nanswer: compute(0)\n");
    let path = package.path().canonicalize().unwrap().join("main.gin");
    let mut session =
        PackageCheckSession::new(4.try_into().unwrap(), BodyReuse::ExactInputs).unwrap();
    let files = || vec![ParsedFile::read(&path).unwrap()];
    let before = resolve::run_package_front_end_with_session(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
        &mut session,
    );
    package.write(
        "math/math.gin",
        &format!(
            "{}compute(value is in Word) is in Word: return 2\n",
            test_fixtures::preludes::WORD_PRELUDE
        ),
    );
    let expected = resolve::run_package_front_end(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
    );
    let after = resolve::run_package_front_end_with_session(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
        &mut session,
    );
    equivalent(&expected, &after);
    assert_ne!(before.public_interface, after.public_interface);
    assert_eq!(
        before.public_interface.resolution_fingerprint,
        after.public_interface.resolution_fingerprint
    );
    let consumer = |output: &PackageFrontEndOutput| {
        output
            .files
            .iter()
            .find(|file| file.parsed.path == path)
            .unwrap()
            .typed
            .as_ref()
            .unwrap()
            .exprs
            .clone()
    };
    assert_ne!(consumer(&before), consumer(&after));
    assert_eq!(session.stats().reused_files, 1);
    assert_eq!(session.stats().lowered_files, 1);
    package.write("main.gin", "use './math' absent\nanswer: absent(0)\n");
    let unresolved = resolve::run_package_front_end_with_session(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
        &mut session,
    );
    assert!(
        unresolved
            .files
            .iter()
            .any(|file| !file.import_diagnostics.is_empty())
    );
    package.write("math/math.gin", &format!("{}compute(value is in Word) is in Word: return 2\nabsent(value is in Word) is in Word: return value\n", test_fixtures::preludes::WORD_PRELUDE));
    let expected = resolve::run_package_front_end(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
    );
    let resolved = resolve::run_package_front_end_with_session(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
        &mut session,
    );
    equivalent(&expected, &resolved);
    assert!(
        resolved
            .files
            .iter()
            .all(|file| file.import_diagnostics.is_empty())
    );
    assert!(
        resolved
            .files
            .iter()
            .all(|file| file.typed.as_ref().unwrap().all_flaws().is_empty())
    );
}

#[test]
fn same_module_aliases_resolve_before_file_declaration_order() {
    let package = TempPackage::new("dependency-front-end-aliases");
    package.write_flask("fixture");
    package.write("a.gin", "A is B\n");
    package.write("b.gin", "B is C\n");
    package.write("c.gin", "C is in 0 to 255\n");
    let root = package.path().canonicalize().unwrap();
    let files = ["a.gin", "b.gin", "c.gin"].map(|path| ParsedFile::read(&root.join(path)).unwrap());
    let expected = resolve::run_package_front_end(
        files.to_vec(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
    );
    let tag_type = |name: &str| {
        let id = typecheck::TagId(ast::prelude::Intern::from_ref(name));
        expected
            .files
            .iter()
            .find_map(|file| {
                file.typed
                    .as_ref()?
                    .tags
                    .get(&id)
                    .map(|tag| tag.resolved_ty.clone())
            })
            .unwrap()
    };
    assert_eq!(tag_type("A"), tag_type("C"));
    assert_eq!(tag_type("B"), tag_type("C"));
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for workers in [1, 2, 4, 8] {
            let mut session =
                PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
                    .unwrap();
            let actual = resolve::run_package_front_end_with_session(
                order.map(|index| files[index].clone()).to_vec(),
                &HashMap::new(),
                ParsedModuleCache::default(),
                options(),
                &mut session,
            );
            equivalent(&expected, &actual);
        }
    }
}

#[test]
fn declaration_catalog_preserves_private_import_diagnostics() {
    let package = TempPackage::new("dependency-front-end-private-alias");
    package.write_flask("fixture");
    package.write("values/types.gin", "private\nSecret is in 0 to 255\n");
    package.write("main.gin", "use './values' Secret\nAlias is Secret\n");
    let path = package.path().canonicalize().unwrap().join("main.gin");
    let files = || vec![ParsedFile::read(&path).unwrap()];
    let expected = resolve::run_package_front_end(
        files(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
    );
    assert!(
        expected
            .files
            .iter()
            .any(|file| !file.import_diagnostics.is_empty())
    );
    for workers in [1, 2, 4, 8] {
        let mut session =
            PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs).unwrap();
        let actual = resolve::run_package_front_end_with_session(
            files(),
            &HashMap::new(),
            ParsedModuleCache::default(),
            options(),
            &mut session,
        );
        equivalent(&expected, &actual);
    }
}

#[test]
fn signatures_can_project_from_later_same_module_record_definitions() {
    let package = TempPackage::new("dependency-front-end-signature-registry");
    package.write_flask("fixture");
    package.write(
        "a.gin",
        "write(cell ref{storage} Cell(Word), slot mut{storage.value} Word):\n    return\n",
    );
    package.write(
        "b.gin",
        "Cell(item) has\n    value item\n    finish(self):\n        return\n",
    );
    package.write("c.gin", test_fixtures::preludes::WORD_PRELUDE);
    let root = package.path().canonicalize().unwrap();
    let files = ["a.gin", "b.gin", "c.gin"].map(|path| ParsedFile::read(&root.join(path)).unwrap());
    let expected = resolve::run_package_front_end(
        files.to_vec(),
        &HashMap::new(),
        ParsedModuleCache::default(),
        options(),
    );
    assert!(
        expected
            .files
            .iter()
            .all(|file| file.import_diagnostics.is_empty()
                && file.prepare_diagnostics.is_empty()
                && file.typed.as_ref().unwrap().all_flaws().is_empty())
    );
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for workers in [1, 2, 4, 8] {
            let mut session =
                PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
                    .unwrap();
            let mut cache = ParsedModuleCache::default();
            for _ in 0..2 {
                let actual = resolve::run_package_front_end_with_session(
                    order.map(|index| files[index].clone()).to_vec(),
                    &HashMap::new(),
                    cache,
                    options(),
                    &mut session,
                );
                equivalent(&expected, &actual);
                cache = actual.cache;
            }
        }
    }
}
