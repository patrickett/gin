//! `:=` constant binds vs `:` rebindable binds; comptime classification decoupled from `:=`.

use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::prepare_parse_ast;
use typecheck::transform::{TransformCtx, transform};

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
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
}

#[test]
fn rebindable_colon_not_constant() {
    let ast = prepare("x: 1\n");
    let bind = ast.defs.get(&Intern::from_ref("x")).expect("x");
    assert!(!bind.is_constant, "`:` should not set is_constant");
}

#[test]
fn main_classified_runtime() {
    let ast = prepare("main:\n  return 0\n");
    let bind = ast.defs.get(&Intern::from_ref("main")).expect("main");
    assert!(!bind.is_constant, "main is a rebindable runtime entry");
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

#[test]
fn compile_time_bind_rejects_runtime_call() {
    let source = "\
runtime(value Type) Type: value
bad(ty Type) Type := runtime(ty)
";
    let typed = transform_prepared(source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "compile-time-runtime-call"),
        "expected runtime-call diagnostic, got {:?}",
        typed.all_flaws()
    );
}
