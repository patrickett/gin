//! Integration tests for dependency member imports (`use core.Int`).

use std::collections::HashMap;

use parser::query::SourceParseExt;
use resolve::{GinPackageExt, ParsedFile, resolve_import_symptoms, resolve_imports};
use test_fixtures::TempPackage;

#[test]
fn resolve_package_member_import_core_int() {
    let pkg = TempPackage::new("member");
    pkg.write_flask_with_deps("app", r#"{"core":{"path":"core"}}"#);
    pkg.write(
        "core/flask.jsonc",
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    pkg.write("core/int.gin", "Int is Unit\n");
    const MAIN: &str = "use core.Int\n\nmain:\nreturn\n";
    let main_path = pkg.write("main.gin", MAIN);

    let source = MAIN.to_string();
    let output = source.parse_source_full();
    let entry = ParsedFile {
        path: main_path.clone(),
        source,
        output,
    };

    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.join("core"));

    let symptoms = resolve_import_symptoms(vec![entry], &deps);
    let main_diags = symptoms.get(&main_path).cloned().unwrap_or_default();
    assert!(
        main_diags.is_empty(),
        "expected no import errors for `use core.Int`, got: {:?}",
        main_diags
    );
}

#[test]
fn resolve_self_dep_from_nested_file_like_gin_core() {
    let pkg = TempPackage::new("gin_core_like");
    pkg.write_flask_with_deps("core", r#"{"self":{"path":"."},"arch":{"path":"arch"}}"#);
    pkg.write("primitive/bool.gin", "Bool is Unit\n");
    pkg.write("primitive/list.gin", "List is Unit\n");
    const COPY: &str = "use self.primitive.(Bool, List)\n\nCopy has \n";
    let copy_path = pkg.write("marker/copy.gin", COPY);

    let deps = copy_path.flask_path_dependencies();

    let source = COPY.to_string();
    let output = source.parse_source_full();
    let entry = ParsedFile {
        path: copy_path.clone(),
        source,
        output,
    };
    let symptoms = resolve::resolve_import_symptoms(vec![entry], &deps);
    let diags = symptoms.get(&copy_path).cloned().unwrap_or_default();
    assert!(
        diags.is_empty(),
        "self dep must resolve from nested file; got {:?}",
        diags
    );
}

#[test]
fn resolve_core_bundle_dotted_members_like_target_gin() {
    let pkg = TempPackage::new("core_target_bundle");
    pkg.write_flask_with_deps("core", r#"{"core":{"path":"."}}"#);
    pkg.write("default/default.gin", "Default(value) has default value\n");
    pkg.write("target/arch/arch.gin", "Architecture is 'x86_64'\n");
    pkg.write("target/os/os.gin", "OperatingSystem is 'linux'\n");
    pkg.write("target/vendor/vendor.gin", "Vendor is 'unknown'\n");
    const TARGET: &str = "use core.(default.Default, target.arch.Architecture, target.os.OperatingSystem, target.vendor.Vendor)\n\nTarget has arch Architecture, vendor Vendor, os OperatingSystem\n";
    let target_path = pkg.write("target/target.gin", TARGET);

    let source = TARGET.to_string();
    let output = source.parse_source_full();
    let deps = target_path.flask_path_dependencies();
    let symptoms = resolve::resolve_import_symptoms(
        vec![ParsedFile {
            path: target_path.clone(),
            source,
            output,
        }],
        &deps,
    );
    let diags = symptoms.get(&target_path).cloned().unwrap_or_default();
    assert!(
        diags.is_empty(),
        "expected `use core.(folder.Symbol, …)` to resolve, got: {:?}",
        diags
    );
}

#[test]
fn resolve_dep_bundle_with_folder_path() {
    let pkg = TempPackage::new("dep_folder_bundle");
    pkg.write_flask_with_deps("app", r#"{"self":{"path":"."}}"#);
    pkg.write("primitive/bool.gin", "Bool is Unit\n");
    pkg.write("primitive/list.gin", "List is Unit\n");
    const MAIN: &str = "use self.primitive.(Bool, List)\n\nmain:\nreturn\n";
    let main_path = pkg.write("main.gin", MAIN);

    let source = MAIN.to_string();
    let output = source.parse_source_full();
    let entry = ParsedFile {
        path: main_path.clone(),
        source,
        output,
    };

    let mut deps = HashMap::new();
    deps.insert("self".to_string(), pkg.path().to_path_buf());

    let symptoms = resolve_import_symptoms(vec![entry], &deps);
    let main_diags = symptoms.get(&main_path).cloned().unwrap_or_default();
    assert!(
        main_diags.is_empty(),
        "expected no import errors for `use self.primitive.(Bool, List)`, got: {:?}",
        main_diags
    );
}

#[test]
fn resolve_imports_excludes_unreferenced_dependency_files() {
    let pkg = TempPackage::new("all_dependency_files");
    pkg.write_flask_with_deps("app", r#"{"core":{"path":"core"}}"#);
    pkg.write(
        "core/flask.jsonc",
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    pkg.write("core/int.gin", "Int is Unit\n");
    let extra_path = pkg.write("core/extra.gin", "Extra is Unit\n");
    let main_path = pkg.write("main.gin", "use core.Int\n\nmain:\nreturn\n");

    let source = std::fs::read_to_string(&main_path).unwrap();
    let entry = ParsedFile {
        path: main_path,
        output: source.parse_source_full(),
        source,
    };
    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.join("core"));

    let resolved = resolve_imports(vec![entry], &deps);
    assert!(
        resolved.iter().all(|file| file.path != extra_path),
        "unreferenced dependency source should not be compiled"
    );
}

#[test]
fn resolve_imports_loads_transitive_modules_without_unrelated_siblings() {
    let pkg = TempPackage::new("lazy_transitive_import");
    pkg.write_flask_with_deps("app", r#"{"core":{"path":"core"}}"#);
    pkg.write(
        "core/flask.jsonc",
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    let root_path = pkg.write("core/root.gin", "use core.primitive.Bool\nRoot is Bool\n");
    let bool_path = pkg.write("core/primitive/bool.gin", "Bool is True or False\n");
    let unrelated_path = pkg.write("core/target/arch.gin", "Architecture is 'x86_64'\n");
    let main_path = pkg.write("main.gin", "use core.Root\n\nmain:\nreturn\n");

    let source = std::fs::read_to_string(&main_path).unwrap();
    let entry = ParsedFile {
        path: main_path,
        output: source.parse_source_full(),
        source,
    };
    let mut deps = HashMap::new();
    deps.insert("core".to_string(), pkg.join("core"));

    let resolved = resolve_imports(vec![entry], &deps);
    let paths: Vec<_> = resolved.iter().map(|file| &file.path).collect();

    assert!(paths.contains(&&root_path));
    assert!(paths.contains(&&bool_path));
    assert!(!paths.contains(&&unrelated_path));
}

#[test]
fn parse_warns_single_member_bundle() {
    let pkg = TempPackage::new("warn");
    const MAIN: &str = "use core.(Int)\n";
    pkg.write("main.gin", MAIN);
    let output = MAIN.parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|d| d.code.slug() == "use-prefer-member-import"),
        "expected PreferMemberImport diagnostic"
    );
}
