//! `:=` constant binds vs `:` rebindable binds; comptime classification decoupled from `:=`.

use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::comptime_classify::FileAstComptimeExt;
use typecheck::prepare_file_ast;
use typecheck::transform::{TransformCtx, transform};

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_file_ast(&mut ast, &CompileTarget::Library);
    ast
}

fn transform_prepared(source: &str) -> typecheck::TypedFileAst {
    let file_ast = prepare(source);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

#[test]
fn constant_bind_sets_is_constant() {
    let ast = prepare("two := 1 + 1\n");
    let bind = ast.defs.get(&Intern::from_ref("two")).expect("two");
    assert!(bind.is_constant, "`:=` should set is_constant");
    assert!(
        bind.is_compile_time,
        "foldable `:=` value should be comptime-classified"
    );
}

#[test]
fn rebindable_colon_not_constant() {
    let mut ast = TokenCursor::parse_source("x: 1\n");
    ast.apply_comptime_classification();
    let bind = ast.defs.get(&Intern::from_ref("x")).expect("x");
    assert!(!bind.is_constant, "`:` should not set is_constant");
}

#[test]
fn main_classified_runtime() {
    let ast = prepare("main:\n  return 0\n");
    let bind = ast.defs.get(&Intern::from_ref("main")).expect("main");
    assert!(!bind.is_compile_time, "main is always runtime");
}

#[test]
fn local_reassign_constant_flaw() {
    let typed = transform_prepared("main:\n  x := 1\n  x: 2\n  return 0\n");
    assert!(
        typed.all_flaws().iter().any(|(_, f)| {
            f.code.slug() == "type-reassign-constant" && f.arg("name") == Some("x")
        }),
        "rebinding `:=` name with `:` should report ReassignConstant: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn const_bind_after_declare_warning() {
    let diags = prepare("val Int\nval := 32\n").parse_warnings;
    assert!(
        diags
            .iter()
            .any(|d| { d.code.slug() == "type-const-bind-after-declare" }),
        "expected ConstBindAfterDeclare warning, got {diags:?}"
    );
}
