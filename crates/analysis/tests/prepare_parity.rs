//! Package prepare uses the same [`ast::prepare_parse_ast`] path as `ginc`.

use analysis::PackageCache;
use flask::CompileTarget;
use parser::query::SourceParseExt;
use std::path::PathBuf;

#[test]
fn package_typecheck_runs_prepare_before_transform() {
    let path = PathBuf::from("/tmp/prepare_parity_main.gin");
    let source = "DefaultInteger is in 0 to 255\nDefaultInteger has finish(self): return\nmain:\n    return 0\n";

    let engine = PackageCache::for_test_with_file(path.clone());
    engine.set_contents(&path, source.to_string());

    let outputs = engine.package_semantics(&[path]);
    assert_eq!(outputs.len(), 1);
    assert!(
        outputs[0].symptoms.is_empty(),
        "valid main should typecheck cleanly after prepare+transform"
    );
}

#[test]
fn package_semantics_include_redundant_terminal_help() {
    let path = PathBuf::from("/tmp/redundant_terminal_help.gin");
    let source = "Redraw is Changed or Unchanged\nRedraw.release(self):\n    return\n";
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.set_contents(&path, source.to_string());

    let outputs = engine.package_semantics(&[path]);
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].symptoms.len(), 1, "{:?}", outputs[0].symptoms);
    let diagnostic = &outputs[0].symptoms[0];
    assert_eq!(
        diagnostic.code.slug().as_str(),
        "type-redundant-terminal-method"
    );
    assert_eq!(diagnostic.category, diagnostic::Category::Help);
    assert_eq!(
        &source[diagnostic.span.start() as usize..diagnostic.span.end() as usize],
        "release"
    );
}

#[test]
fn compile_and_ide_paths_share_cross_file_compile_time_context() {
    let package_a: ast::FileAst =
        "Int is in 0 to 255\nInt has finish(self): return\nshared_const := 1\n"
            .parse_source_full()
            .ast;
    let package_b: ast::FileAst = "main Int: return shared_const\n".parse_source_full().ast;
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
        !has_unknown_symbol(&full.typed_asts),
        "cross-file compile-time symbol should be known in shared package transform path"
    );
}
