//! `use core folder.Symbol, …` must typecheck like quoted sibling imports in target/.

use std::collections::HashMap;
use std::path::PathBuf;

use internment::Intern;
use parser::query::SourceParseExt;
use resolve::{GinPackageExt, ParsedFile, resolve_imports};
use test_fixtures::TempPackage;

use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

mod support;
use support::unknown_symbol_names;

fn transform_resolved_package(files: Vec<(PathBuf, String)>) -> Vec<typecheck::TypedFileAst> {
    let mut deps = HashMap::new();
    let mut parsed: Vec<ParsedFile> = Vec::new();
    for (path, source) in files {
        if deps.is_empty() {
            deps = path.flask_path_dependencies();
        }
        let output = source.parse_source_full();
        parsed.push(ParsedFile {
            path: path.clone(),
            source,
            output,
        });
    }
    let mut resolved = resolve_imports(parsed, &deps);
    let entry = flask::CompileTarget::Library;
    for file in &mut resolved {
        file.output
            .symptoms
            .extend(typecheck::prepare_parse_ast(&mut file.output.ast, &entry));
    }
    transform_package_with_shared_context(
        resolved.into_iter().map(|file| file.output.ast).collect(),
        PackageTransformOptions::FULL,
    )
    .typed_asts
}

#[test]
fn core_bundle_import_types_like_target_gin() {
    let pkg = TempPackage::new("target_bundle_types");
    pkg.write_flask_with_deps(
        "core",
        r#"core: "./."
"#,
    );
    pkg.write("default/default.gin", "Default has default() Self\n");
    pkg.write("target/arch/arch.gin", "Architecture is X86 or Arm\n");
    pkg.write("target/os/os.gin", "OperatingSystem is Linux or Darwin\n");
    pkg.write("target/vendor/vendor.gin", "Vendor is Unknown or Apple\n");
    const TARGET: &str = "use core default.Default, target.arch.Architecture, target.os.OperatingSystem, target.vendor.Vendor\n\nTarget has Default\n    finish(self):\n        return\n    arch Architecture\n    vendor Vendor\n    os OperatingSystem\n    default() Self: return Self(arch: Architecture.X86, vendor: Vendor.Unknown, os: OperatingSystem.Linux)\n\ntarget := Target.default\n";
    let target_path = pkg.write("target/target.gin", TARGET);
    let default_path = pkg.write("default/default.gin", "Default has default() Self\n");
    let arch_path = pkg.write("target/arch/arch.gin", "Architecture is X86 or Arm\n");
    let os_path = pkg.write("target/os/os.gin", "OperatingSystem is Linux or Darwin\n");
    let vendor_path = pkg.write("target/vendor/vendor.gin", "Vendor is Unknown or Apple\n");

    let files = vec![
        (
            default_path,
            std::fs::read_to_string(pkg.join("default/default.gin")).unwrap(),
        ),
        (
            arch_path,
            std::fs::read_to_string(pkg.join("target/arch/arch.gin")).unwrap(),
        ),
        (
            os_path,
            std::fs::read_to_string(pkg.join("target/os/os.gin")).unwrap(),
        ),
        (
            vendor_path,
            std::fs::read_to_string(pkg.join("target/vendor/vendor.gin")).unwrap(),
        ),
        (target_path.clone(), TARGET.to_string()),
    ];

    let typed = transform_resolved_package(files);
    let target_typed = typed
        .iter()
        .find(|t| {
            t.tags
                .contains_key(&typecheck::TagId(Intern::from_ref("Target")))
        })
        .expect("target/target.gin typed output");

    for name in ["Architecture", "Vendor", "OperatingSystem", "Default"] {
        assert!(
            !unknown_symbol_names(target_typed).iter().any(|n| n == name),
            "expected imported type `{name}` in scope, flaws: {:?}",
            target_typed.all_flaws()
        );
    }

    assert!(
        target_typed
            .tags
            .contains_key(&typecheck::TagId(Intern::from_ref("Target"))),
        "Target tag should be declared; flaws: {:?}",
        target_typed.all_flaws()
    );

    let flaws = target_typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no type flaws on target/target.gin, got: {flaws:?}"
    );
}
