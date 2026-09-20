#![allow(dead_code)]

use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, ExprId, FileId, TypedFileAst};

pub fn check(source: &str) -> TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

pub fn body(typed: &TypedFileAst, name: &str) -> ExprId {
    match &typed.defs[&DefId(Intern::from_ref(name))].body {
        BindBody::Expr(expr) => *expr,
        BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()).unwrap(),
        BindBody::Unlowered => panic!("expected a body"),
    }
}

pub fn flaw_codes(typed: &TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect()
}
