use std::collections::HashMap;
use std::path::PathBuf;

use ast::ty::PackageInstanceKey;
use flask::CompileTarget;
use parser::query::SourceParseExt;
use resolve::{PackageFrontEndOptions, ParsedFile, ParsedModuleCache, run_package_front_end};
use test_fixtures::TempPackage;
use typecheck::transform::PackageTransformOptions;

fn options() -> PackageFrontEndOptions {
    PackageFrontEndOptions {
        gin_home: None,
        compile_target: CompileTarget::Library,
        transform: PackageTransformOptions::IDE,
        resolve_imports: false,
        package_fallback: PackageInstanceKey::workspace("fixture", "0"),
        transform_paths: None,
    }
}

#[test]
fn selected_transform_results_stay_with_their_files() {
    for selection in [None, Some(vec![PathBuf::from("b.gin")]), Some(Vec::new())] {
        let files = [("z.gin", "zebra"), ("a.gin", "alpha"), ("b.gin", "beta")]
            .into_iter()
            .map(|(path, name)| {
                let source = format!("{name} := 1\n");
                ParsedFile {
                    path: path.into(),
                    output: source.parse_source_full(),
                    source,
                }
            })
            .collect();
        let mut options = options();
        options.transform_paths = selection.clone();
        let output = run_package_front_end(
            files,
            &HashMap::new(),
            ParsedModuleCache::default(),
            options,
        );
        assert_eq!(
            output
                .files
                .iter()
                .map(|file| file.parsed.path.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["a.gin", "b.gin", "z.gin"]
        );
        for (file, name) in output.files.iter().zip(["alpha", "beta", "zebra"]) {
            let selected = selection
                .as_ref()
                .is_none_or(|paths| paths.contains(&file.parsed.path));
            assert_eq!(file.typed.is_some(), selected);
            if let Some(typed) = &file.typed {
                assert!(typed.defs.keys().any(|id| id.0.as_str() == name));
                assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
            }
        }
    }
}

#[test]
fn imported_files_precede_importers_and_keep_diagnostic_provenance_with_cached_imports() {
    let package = TempPackage::new("front_end");
    package.write_flask("fixture");
    package.write("math/values.gin", "badName := 1\n");
    package.write(
        "main.gin",
        "use './math' badName\nuse './missing' absent\nmainValue := 2\n",
    );
    let root = package.path().canonicalize().unwrap();
    let path = root.join("main.gin");
    let imported = root.join("math/values.gin");
    let mut cache = ParsedModuleCache::default();
    for _ in 0..2 {
        let mut options = options();
        options.resolve_imports = true;
        let output = run_package_front_end(
            vec![ParsedFile::read(&path).unwrap()],
            &HashMap::new(),
            cache,
            options,
        );
        assert_eq!(
            output
                .files
                .iter()
                .map(|file| &file.parsed.path)
                .collect::<Vec<_>>(),
            [&imported, &path]
        );
        for file in &output.files {
            let original = ParsedFile::read(&file.parsed.path).unwrap();
            assert!(!original.output.symptoms.is_empty());
            assert_eq!(file.parsed.output.symptoms, original.output.symptoms);
            assert!(file.typed.is_some());
        }
        assert!(output.files[0].import_diagnostics.is_empty());
        assert!(!output.files[1].import_diagnostics.is_empty());
        assert!(
            output.files[1]
                .import_diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code.slug().starts_with("use-"))
        );
        cache = output.cache;
    }
}

#[test]
fn relative_transform_selection_survives_import_path_normalization() {
    let package = TempPackage::new("relative_transform_selection");
    package.write_flask("fixture");
    package.write("math/values.gin", "answer: 42\n");
    package.write("main.gin", "use './math' answer\nmain: 42\n");
    let root = package.path().canonicalize().unwrap();
    let cwd = std::env::current_dir().unwrap();
    let common = cwd
        .ancestors()
        .find(|ancestor| root.starts_with(ancestor))
        .unwrap();
    let relative = cwd
        .strip_prefix(common)
        .unwrap()
        .components()
        .fold(PathBuf::new(), |path, _| path.join(".."))
        .join(root.strip_prefix(common).unwrap())
        .join("main.gin");
    assert!(relative.is_relative());
    let mut options = options();
    options.resolve_imports = true;
    options.transform_paths = Some(vec![relative.clone()]);
    let output = run_package_front_end(
        vec![ParsedFile::read(&relative).unwrap()],
        &HashMap::new(),
        ParsedModuleCache::default(),
        options,
    );
    let selected = output
        .files
        .iter()
        .filter(|file| file.typed.is_some())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].parsed.path, root.join("main.gin"));
    assert!(selected[0].typed.as_ref().unwrap().all_flaws().is_empty());
}
