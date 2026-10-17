//! Tests that characterize staged-expression behavior in resolved calls and literals.

use ast::{ConstValue, NormalExpr};
use internment::Intern;
use typecheck::{BindBody, DefId, TypedExprKind, TypedFileAst};

mod support;
use support::{body_expr_id, has_flaw_with_slug};
fn transform_source(source: &str) -> TypedFileAst {
    support::transform_source(&format!(
        "{source}\nInt is in 0 to 4294967295\nInt.finish(self): return\n"
    ))
}

fn transform_prepared(source: &str) -> TypedFileAst {
    support::transform_prepared(&format!(
        "{source}\n{}",
        test_fixtures::preludes::INT_PRELUDE
    ))
}

#[test]
fn type_argument_is_not_lowered_as_runtime_arg() {
    let typed = transform_source("id_ty(x) Int: return #size(x)\nmain: id_ty(Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall { target, args, .. } => {
            assert!(target.0.as_str().starts_with("id_ty$specialized$"));
            assert_eq!(args.as_ref().map(Vec::len), Some(0));
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn type_argument_substituted_return_type_is_available() {
    let typed = transform_source("size_of(x) Int: return #size(x)\nmain: size_of(Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert!(target.0.as_str().starts_with("size_of$specialized$"));
            assert_eq!(args.as_ref().map(Vec::len), Some(0));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn literal_arithmetic_expr_is_typed_as_binary() {
    let typed = transform_source("main: 1 + 2");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    assert!(
        matches!(expr.kind, TypedExprKind::IntrinsicCall { .. }),
        "expected intrinsic call, got {:?}",
        expr.kind
    );
}

#[test]
fn mixed_type_and_value_call_preserves_runtime_arg_positions() {
    let typed = transform_source("id(x, y x) x: return y\nmain: id(Int, 3)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(args.as_ref().map(Vec::len), Some(1), "{expr:?}");
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn named_type_and_value_call_preserves_runtime_arg_positions() {
    let typed = transform_source("id(x, y x) x: return y\nmain: id(y: 3, x: Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(args.as_ref().map(Vec::len), Some(1), "{expr:?}");
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn arithmetic_type_value_argument_is_used_for_substitution() {
    let source = "Array(x, n Int) has length Int\nmake(x, n Int) Array(x, n): return Array(length: n)\nmain: make(Int, 1 + 2)\n";
    let typed = transform_source(source);
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "make");
            assert_eq!(args.as_ref().map(Vec::len), Some(1));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn runtime_when_expression_is_typed_as_when_node() {
    let source = "value_or_default(x Int) Int: return when x is 0 then 0 else x\n";
    let typed = transform_source(source);
    let value_or_default_body =
        body_expr_id(&typed, "value_or_default").expect("value_or_default has body");
    let expr = typed
        .exprs
        .get(value_or_default_body.as_usize())
        .expect("value_or_default body");

    assert!(matches!(expr.kind, TypedExprKind::When(_)));
}

#[test]
fn pure_literal_arithmetic_expression_reaches_const_value() {
    let typed = transform_prepared("main: 1 + 2");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    let TypedExprKind::IntrinsicCall { args, op, .. } = expr.kind else {
        panic!("Expected intrinsic operator, got {:?}", expr.kind)
    };
    assert_eq!(op.name(), "BitsAdd");
    assert_eq!(args.len(), 2);
    assert!(matches!(
        typed.exprs.facts.const_facts.const_value[args[0].as_usize()],
        Some(ConstValue::Int(value)) if value == 1.into()
    ));
    assert!(matches!(
        typed.exprs.facts.const_facts.const_value[args[1].as_usize()],
        Some(ConstValue::Int(value)) if value == 2.into()
    ));
}

#[test]
fn generic_value_parameter_substitutes_array_size_in_call_type() {
    let typed =
        transform_source("sized_array(n Int) [Int; n]: return [0; n]\nmain: sized_array(2)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "sized_array");
            assert_eq!(args.as_ref().map(Vec::len), Some(1));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn generic_array_return_type_preserves_symbolic_addition_expression() {
    let typed = transform_source("make(n Int): return [0; (n + 1)]\nmain: make(2)");
    let def = typed
        .defs
        .get(&DefId::named(Intern::new("make".to_string())))
        .expect("make exists");
    let body = match &def.body {
        BindBody::Expr(id) => typed.exprs.get(id.as_usize()).expect("make body"),
        other => panic!("Expected make body expression, got {other:?}"),
    };
    match &body.kind {
        TypedExprKind::FixedArrayAlloc { size, .. } => {
            assert!(
                !matches!(size, NormalExpr::Value(_)),
                "expected symbolic sequence size expression, got {size:?}"
            );
        }
        other => panic!("Expected FixedArrayAlloc in make body, got {other:?}"),
    }

    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "make");
            assert_eq!(args.as_ref().map(Vec::len), Some(1));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn fixed_array_alloc_arithmetic_size_is_not_forced_to_zero() {
    let typed = transform_prepared("main: [0; 1 + 2]");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FixedArrayAlloc { size, .. } => {
            assert_ne!(*size, NormalExpr::from(0));
        }
        other => panic!("Expected FixedArrayAlloc, got {other:?}"),
    }
}

#[test]
fn fixed_array_alloc_subtract_size_is_normalized() {
    let typed = transform_prepared("main: [0; 6 - 1]");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FixedArrayAlloc { size, .. } => {
            assert_eq!(*size, NormalExpr::from(5));
        }
        other => panic!("Expected FixedArrayAlloc, got {other:?}"),
    }
}

#[test]
fn fixed_array_alloc_multiply_size_is_normalized() {
    let typed = transform_prepared("main: [0; 2 * 3]");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FixedArrayAlloc { size, .. } => {
            assert_eq!(*size, NormalExpr::from(6));
        }
        other => panic!("Expected FixedArrayAlloc, got {other:?}"),
    }
}

#[test]
fn runtime_argument_to_type_only_call_reports_diag() {
    let typed = transform_prepared(
        "size_of(x) Int: return #size(x)\n\npick(x Int) Int:\n  return x\n\nmain:\n  return size_of(pick(1))\n",
    );
    assert!(
        has_flaw_with_slug(&typed, "type-generic-specialization"),
        "expected type-generic-specialization: {:?}",
        typed.all_flaws()
    );
}
