//! Package prepare uses the same [`ast::prepare_file_ast`] path as `ginc`.

use analysis::{CacheEngine, QueryEngine};
use flask::CompileTarget;
use parser::query::SourceParseExt;
use crossbeam_channel::unbounded;
use std::path::PathBuf;

#[test]
fn package_typecheck_runs_prepare_before_transform() {
    let path = PathBuf::from("/tmp/prepare_parity_main.gin");
    let source = "main:\n    return 0\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();
    engine.set_contents(&path, source.to_string());

    let outputs = engine.package_semantics(&[path]);
    assert_eq!(outputs.len(), 1);
    assert!(
        outputs[0].symptoms.is_empty(),
        "valid main should typecheck cleanly after prepare+transform"
    );
}

#[test]
fn compile_and_ide_paths_share_cross_file_compile_time_context() {
    let package_a: ast::FileAst = "shared_const := 1\n".parse_source_full().ast;
    let package_b: ast::FileAst = "main() Int: return shared_const\n".parse_source_full().ast;
    let compile_target = CompileTarget::Library;

    let mut file_asts = vec![package_a.clone(), package_b.clone()];
    let _prepare_diags = typecheck::prepare_package_asts(&mut file_asts, &compile_target);
    let full = typecheck::transform::transform_package_with_shared_context(
        file_asts.clone(),
        typecheck::transform::PackageTransformOptions::FULL,
    );
    let ide = typecheck::transform::transform_package_with_shared_context(
        file_asts,
        typecheck::transform::PackageTransformOptions::IDE,
    );

    let has_unknown_symbol = |typed: &[typecheck::TypedFileAst]| {
        typed.iter().any(|typed| {
            typed
                .all_flaws()
                .iter()
                .any(|(_, d)| d.code.slug() == "type-unknown-symbol")
        })
    };

    assert_eq!(
        has_unknown_symbol(&full.typed_asts),
        has_unknown_symbol(&ide.typed_asts),
        "FULL and IDE should both resolve cross-file names after shared transform setup"
    );
    assert!(
        has_unknown_symbol(&full.typed_asts) == false,
        "cross-file compile-time symbol should be known in shared package transform path"
    );

    // Deliberate behavioral difference: IDE intentionally skips full trait synthesis.
    assert!(full.trait_registry.is_some(), "FULL path should build compile-time trait registry");
    assert!(ide.trait_registry.is_none(), "IDE path should keep lightweight transform mode");
}
