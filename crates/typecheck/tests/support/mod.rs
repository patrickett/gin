#![allow(dead_code)]

use ast::FileAst;
use ast::prelude::FileSemanticOrigin;
use ast::ty::PackageInstanceKey;
use internment::Intern;
use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;
use typecheck::eval::{CompileTimeHost, Evaluator};
use typecheck::transform::{
    PackageTransformOptions, TransformCtx, transform,
    transform_package_with_shared_context_and_package,
};
use typecheck::{BindBody, DefId, ExprId, FileId, TypedFileAst};

pub fn check(source: &str) -> TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

pub fn check_lexed(source: &str) -> TypedFileAst {
    let parsed = TokenCursor::parse_source(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}\n{source}",
        parsed.parse_warnings
    );
    transform(&parsed, FileId(0), &TransformCtx::new())
}

pub fn check_parsed(source: &str) -> TypedFileAst {
    let parsed = source.parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "{:?}\n{source}",
        parsed.symptoms
    );
    transform(&parsed.ast, FileId(0), &TransformCtx::new())
}

pub fn check_with(prelude: &str, body: &str) -> TypedFileAst {
    check_parsed(&format!("{prelude}{body}"))
}

pub fn check_no_flaws_with(prelude: &str, body: &str) -> TypedFileAst {
    let typed = check_with(prelude, body);
    expect_no_flaws(&typed);
    typed
}

pub fn check_lexed_with(prelude: &str, body: &str) -> TypedFileAst {
    check_lexed(&format!("{prelude}{body}"))
}

pub fn check_unflawed(source: &str) -> TypedFileAst {
    let parsed = source.parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .all(|diagnostic| diagnostic.category != diagnostic::Category::Flaw),
        "{source}: {:?}",
        parsed.symptoms
    );
    transform(&parsed.ast, FileId(0), &TransformCtx::new())
}

pub fn check_unflawed_with(prelude: &str, body: &str) -> TypedFileAst {
    check_unflawed(&format!("{prelude}{body}"))
}

pub fn check_no_flaws(source: &str) -> TypedFileAst {
    let typed = check_lexed(source);
    expect_no_flaws(&typed);
    typed
}

pub fn body(typed: &TypedFileAst, name: &str) -> ExprId {
    match &typed.defs[&DefId::named(Intern::from_ref(name))].body {
        BindBody::Expr(expr) => *expr,
        BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()).unwrap(),
        BindBody::Unlowered => panic!("expected a body"),
    }
}

pub fn flaw_codes(typed: &TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.to_string())
        .collect()
}

pub fn flaw_codes_with(prelude: &str, body: &str) -> Vec<String> {
    flaw_codes(&check_with(prelude, body))
}

pub fn flaw_codes_of(source: &str) -> Vec<String> {
    flaw_codes(&check(source))
}

pub fn def_flaw_codes(typed: &TypedFileAst, name: &str) -> Vec<String> {
    typed.defs[&DefId::named(Intern::from_ref(name))]
        .flaws
        .iter()
        .map(|flaw| flaw.code.to_string())
        .collect()
}

pub fn expect_flaw_code(typed: &TypedFileAst, code: &str) {
    let codes = flaw_codes(typed);
    assert!(
        codes.iter().any(|actual| actual == code),
        "{:?}",
        typed.all_flaws()
    );
}

pub fn expect_flaw_code_named(source: &str, code: &str, case: &str) {
    let typed = check_lexed(source);
    assert!(
        flaw_codes(&typed).iter().any(|actual| actual == code),
        "case `{case}` failed: {:?}",
        typed.all_flaws()
    );
}

pub fn expect_no_flaws(typed: &TypedFileAst) {
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

pub fn expect_flaws(source: &str, expected: &[&str]) {
    let typed = check_parsed(source);
    assert_flaw_codes(&typed, expected, source);
}

pub fn expect_flaws_with(prelude: &str, body: &str, expected: &[&str]) {
    let source = format!("{prelude}{body}");
    expect_flaws(&source, expected);
}

pub fn expect_flaws_lexed_with(prelude: &str, body: &str, expected: &[&str]) {
    let source = format!("{prelude}{body}");
    let typed = check_lexed(&source);
    assert_flaw_codes(&typed, expected, &source);
}

pub fn expect_flaws_with_named(prelude: &str, body: &str, expected: &[&str], case: &str) {
    assert_flaw_codes_named(&check_with(prelude, body), expected, case);
}

pub fn expect_flaws_lexed_with_named(prelude: &str, body: &str, expected: &[&str], case: &str) {
    assert_flaw_codes_named(&check_lexed_with(prelude, body), expected, case);
}

fn assert_flaw_codes_named(typed: &TypedFileAst, expected: &[&str], case: &str) {
    let mut actual = flaw_codes(typed);
    let mut expected = expected.to_vec();
    actual.sort();
    expected.sort();
    assert_eq!(
        actual,
        expected,
        "case `{case}` failed: {:?}",
        typed.all_flaws()
    );
}

fn assert_flaw_codes(typed: &TypedFileAst, expected: &[&str], source: &str) {
    let mut actual = flaw_codes(typed);
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(actual, expected, "{:?}\n{source}", typed.all_flaws());
}

pub fn package_files(name: &str, files: &[(&str, &str)]) -> Vec<FileAst> {
    let package = PackageInstanceKey::workspace(name, "1");
    files
        .iter()
        .map(|&(module, source)| {
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{:?}\n{source}",
                parsed.symptoms
            );
            let mut ast = parsed.ast;
            ast.semantic_origin = Some(FileSemanticOrigin {
                package: package.clone(),
                module: vec![Intern::from_ref(module)],
            });
            ast
        })
        .collect()
}

pub fn package(name: &str, files: &[(&str, &str)]) -> Vec<TypedFileAst> {
    transform_package_with_shared_context_and_package(
        package_files(name, files),
        PackageTransformOptions::FULL,
        PackageInstanceKey::workspace(name, "1"),
    )
    .typed_asts
}

pub fn package_flaws(name: &str, files: &[(&str, &str)]) -> Vec<String> {
    package(name, files).iter().flat_map(flaw_codes).collect()
}

pub fn evaluate(source: &str) -> ast::ConstValue {
    let typed = check_unflawed(source);
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
    Evaluator::new(
        typed.resolved_program(),
        CompileTimeHost::new(&flask::CompileTarget::Library),
    )
    .call(DefId::named(Intern::from_ref("run")), &[])
    .unwrap()
}
