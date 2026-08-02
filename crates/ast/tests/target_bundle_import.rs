//! `use core.(folder.Symbol, …)` must typecheck like quoted sibling imports in target/.

use std::collections::HashMap;
use std::path::PathBuf;

use internment::Intern;
use parser::query::SourceParseExt;
use resolve::{GinPackageExt, ParsedFile, resolve_imports};
use test_fixtures::TempPackage;

use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform_package};

fn unknown_symbol_names(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, flaw)| flaw.code.slug() == "type-unknown-symbol")
        .map(|(_, flaw)| flaw.arg("name").unwrap_or("").to_string())
        .collect()
}

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
    let mut compile_time_eval_ast = ast::FileAst::default();
    for file in &resolved {
        compile_time_eval_ast.merge_from(file.output.ast.clone());
    }
    let ctx = TransformCtx::with_package_compile_time(compile_time_eval_ast);
    let file_asts: Vec<(ast::FileAst, FileId)> = resolved
        .into_iter()
        .enumerate()
        .map(|(i, f)| (f.output.ast, FileId(i as u32)))
        .collect();
    transform_package(
        &file_asts,
        &ctx,
        typecheck::transform::PackageTransformOptions::FULL,
    )
}

#[test]
fn core_bundle_import_types_like_target_gin() {
    let pkg = TempPackage::new("target_bundle_types");
    pkg.write_flask_with_deps("core", r#"{"core":{"path":"."}}"#);
    pkg.write("default/default.gin", "Default(value) has default value\n");
    pkg.write("target/arch/arch.gin", "Architecture is 'x86_64'\n");
    pkg.write("target/os/os.gin", "OperatingSystem is 'linux'\n");
    pkg.write("target/vendor/vendor.gin", "Vendor is 'unknown'\n");
    const TARGET: &str = "\
use core.(default.Default, target.arch.Architecture, target.os.OperatingSystem, target.vendor.Vendor)

Target has Default
    arch Architecture
    vendor Vendor
    os OperatingSystem
    Default.default: (arch: 'x86_64', vendor: 'unknown', os: 'unknown')

target := Target.default
";
    let target_path = pkg.write("target/target.gin", TARGET);
    let default_path = pkg.write("default/default.gin", "Default(value) has default value\n");
    let arch_path = pkg.write("target/arch/arch.gin", "Architecture is 'x86_64'\n");
    let os_path = pkg.write("target/os/os.gin", "OperatingSystem is 'linux'\n");
    let vendor_path = pkg.write("target/vendor/vendor.gin", "Vendor is 'unknown'\n");

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
