//! Binary arithmetic: parsing, `ConstValue::eval_binop`, compile-time `:=` folding, and lowering.

use ast::prelude::*;
use ast::{ConstValue, HashFloat};
use flask::CompileTarget;
use internment::Intern;
use std::collections::HashMap;
use typecheck::analysis::{TyInfer, TyInferEnv, eval_compile_time_expr_public};
use typecheck::prepare_file_ast;
use typecheck::ty::Ty;
use typecheck::{DefId, ExprId, TypedExprKind};

mod support;
use support::transform_source;

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = parser::cursor::TokenCursor::parse_source(source);
    let _ = prepare_file_ast(&mut ast, &CompileTarget::Library);
    ast
}

fn bind_expr_const_value<'a>(ast: &'a ast::FileAst, name: &str) -> Option<&'a ConstValue> {
    let bind = ast.defs.get(&Intern::from_ref(name))?;
    match &bind.value {
        ast::BindValue::Expr(e) => e.const_value.as_ref(),
        _ => None,
    }
}

fn body_expr_id(typed: &typecheck::TypedFileAst, def_name: &str) -> Option<ExprId> {
    let def_id = DefId(Intern::new(def_name.to_string()));
    let bind = typed.defs.get(&def_id)?;
    match &bind.body {
        typecheck::BindBody::Expr(eid) => Some(*eid),
        typecheck::BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()),
        typecheck::BindBody::Extern => None,
    }
}

fn assert_const_int(cv: Option<&ConstValue>, n: i128) {
    assert_eq!(
        cv.and_then(ConstValue::as_const_size_int),
        Some(n),
        "expected const int {n}, got {cv:?}"
    );
}

#[test]
fn eval_binop_int_arithmetic_uses_size_const_tags() {
    // Plain `Int` operands route through `as_const_size_int` and fold to `Size::Const(n)`.
    assert_const_int(
        ConstValue::Int(1)
            .eval_binop(&BinOp::Add, &ConstValue::Int(1))
            .as_ref(),
        2,
    );
    assert_const_int(
        ConstValue::Int(10)
            .eval_binop(&BinOp::Subtract, &ConstValue::Int(3))
            .as_ref(),
        7,
    );
    assert_const_int(
        ConstValue::Int(6)
            .eval_binop(&BinOp::Multiply, &ConstValue::Int(7))
            .as_ref(),
        42,
    );
    assert_const_int(
        ConstValue::Int(20)
            .eval_binop(&BinOp::Divide, &ConstValue::Int(4))
            .as_ref(),
        5,
    );
    assert_eq!(
        ConstValue::Int(17).eval_binop(&BinOp::Modulo, &ConstValue::Int(5)),
        Some(ConstValue::Int(2))
    );
}

#[test]
fn eval_binop_int_divide_by_zero_is_none() {
    assert_eq!(
        ConstValue::Int(1).eval_binop(&BinOp::Divide, &ConstValue::Int(0)),
        None
    );
}

#[test]
fn eval_binop_float_arithmetic() {
    assert_eq!(
        ConstValue::Float(HashFloat(1.5))
            .eval_binop(&BinOp::Add, &ConstValue::Float(HashFloat(2.5))),
        Some(ConstValue::Float(HashFloat(4.0)))
    );
}

#[test]
fn compile_time_bind_folds_addition() {
    let ast = prepare("two := 1 + 1\n");
    let bind = ast.defs.get(&Intern::from_ref("two")).expect("two");
    assert!(bind.is_constant);
    assert!(bind.is_compile_time);
    assert_const_int(bind_expr_const_value(&ast, "two"), 2);
}

#[test]
fn compile_time_bind_folds_nested_addition() {
    let ast = prepare("sum := (1 + 2) + 3\n");
    assert_const_int(bind_expr_const_value(&ast, "sum"), 6);
}

#[test]
fn eval_compile_time_expr_folds_addition() {
    let ast = prepare("two := 1 + 1\n");
    let mut const_binds: HashMap<Intern<String>, Option<ConstValue>> =
        ast.defs.keys().map(|name| (*name, None)).collect();
    const_binds.insert(
        Intern::from_ref("two"),
        bind_expr_const_value(&ast, "two").cloned(),
    );
    let main = parser::cursor::TokenCursor::parse_source("x: 1 + 2\n");
    let bind = main.defs.get(&Intern::from_ref("x")).unwrap();
    let ast::BindValue::Expr(e) = &bind.value else {
        panic!("expected expr bind");
    };
    let cv = eval_compile_time_expr_public(&e.value, &const_binds, &main).expect("fold 1+2");
    assert_const_int(Some(&cv), 3);
}

#[test]
fn infer_ty_folds_literal_addition() {
    let file = parser::cursor::TokenCursor::parse_source("main: 1 + 1\n");
    let bind = file.defs.get(&Intern::from_ref("main")).unwrap();
    let ast::BindValue::Expr(e) = &bind.value else {
        panic!("expected expr");
    };
    let ast::Expr::Binary(bin) = &e.value else {
        panic!("expected binary body");
    };
    let env = TyInferEnv {
        tag_types: &HashMap::new(),
        fn_return_types: &HashMap::new(),
        locals: &HashMap::new(),
        tag_params: None,
    };
    let ty = bin.infer_ty(&env);
    assert!(
        matches!(
            ty,
            Ty::Int {
                value: Some(2),
                signed: true,
                ..
            }
        ),
        "TyInfer should fold `1 + 1` to value 2, got {ty:?}"
    );
}

#[test]
fn runtime_bind_resolve_type_folds_literal_addition() {
    let typed = transform_source("two: 1 + 1\n");
    let two_id = body_expr_id(&typed, "two").expect("two body");
    let expr = typed.exprs.get(two_id.as_usize()).expect("two expr");
    assert!(
        matches!(expr.kind, TypedExprKind::Binary { .. }),
        "bind body should stay Binary, got {:?}",
        expr.kind
    );
    assert!(
        matches!(
            expr.ty,
            Ty::Int {
                value: Some(2),
                signed: true,
                ..
            }
        ),
        "resolve_expr_type should fold `1 + 1` like TyInfer, got {:?}",
        expr.ty
    );
}

#[test]
fn rebindable_foldable_top_level_still_folds() {
    let raw = parser::cursor::TokenCursor::parse_source("two: 1 + 1\n");
    assert!(bind_expr_const_value(&raw, "two").is_none());
    let prepared = prepare("two: 1 + 1\n");
    let bind = prepared.defs.get(&Intern::from_ref("two")).expect("two");
    assert!(!bind.is_constant, "`:=` is required for constant bind");
    assert!(
        bind.is_compile_time,
        "pure literal fold should still comptime-classify"
    );
    assert_const_int(bind_expr_const_value(&prepared, "two"), 2);
}

#[test]
fn runtime_binary_with_variables_stays_dynamic() {
    let typed = transform_source("x: 10\ny: 20\nmain: x + y");
    let main_id = body_expr_id(&typed, "main").expect("main");
    let expr = typed.exprs.get(main_id.as_usize()).expect("main expr");
    assert!(matches!(expr.kind, TypedExprKind::Binary { .. }));
    assert!(
        matches!(expr.ty, Ty::Int { value: None, .. }),
        "variable operands should not carry a folded value, got {:?}",
        expr.ty
    );
}
